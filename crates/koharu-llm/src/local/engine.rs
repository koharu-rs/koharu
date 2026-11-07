//! The thread that owns a loaded model: one llama.cpp context whose
//! sequences decode together, after llama.cpp's `parallel` example
//! (https://github.com/ggml-org/llama.cpp/blob/7aca0d2ea8d92b56911233b822db01f9fbb1ae50/examples/parallel/parallel.cpp#L264-L476).
//! A sequence keeps what it decoded after its request ends, and a new request
//! reuses the longest common prefix any sequence holds, as llama-server picks a
//! slot (`get_available_slot`, tools/server/server-context.cpp at b12143).

use std::{
    collections::VecDeque,
    ffi::CString,
    fmt,
    num::NonZeroU32,
    path::PathBuf,
    pin::Pin,
    sync::{
        Mutex, OnceLock, PoisonError,
        mpsc::{Receiver, SyncSender},
    },
};

use anyhow::{Context, Result, bail, ensure};
use koharu_llama::{
    context::{LlamaContext, params::LlamaContextParams},
    llama_backend::LlamaBackend,
    llama_batch::LlamaBatch,
    model::{LlamaModel, params::LlamaModelParams},
    mtmd::{
        MtmdBitmap, MtmdContext, MtmdContextParams, MtmdInputChunks, MtmdInputText,
        mtmd_default_marker,
    },
    sampling::LlamaSampler,
    token::LlamaToken,
};
use koharu_llama_sys::GGML_LOG_LEVEL_WARN;
use llguidance::{Matcher, ParserFactory, api::TopLevelGrammar};
use rig_core::error::{ErrorReport, ProviderError};
use tokio::sync::mpsc::UnboundedSender;

use super::{Sampling, Size, template::Template};

/// Only instructions and history are worth sharing; a shorter common prefix is
/// the chat template's header.
const MIN_SHARED: usize = 64;

/// The free device memory a load leaves, and the least context a sequence is
/// fitted down to: llama.cpp's defaults for `--fit`.
const MARGIN: usize = 1 << 30;
const MIN_CONTEXT: u32 = 4096;

pub(super) struct Files {
    pub(super) model: PathBuf,
    pub(super) projector: Option<PathBuf>,
    /// The ggml index of the device the model runs on.
    pub(super) device: usize,
}

/// One request: its prompt as the chat template rendered it, with a media
/// marker where each image goes, and where its reply goes.
pub(super) struct Job {
    pub(super) prompt: String,
    pub(super) images: Vec<Vec<u8>>,
    pub(super) sampling: Sampling,
    pub(super) schema: Option<String>,
    pub(super) frames: UnboundedSender<Result<Step, ProviderError>>,
}

/// One step of a reply.
#[derive(Clone, Debug)]
pub(super) enum Step {
    Text(String),
    /// The reply ended: at an end-of-generation token, or at its token
    /// limit or the end of its sequence's context.
    End {
        length: bool,
        prompt_tokens: u64,
        output_tokens: u64,
    },
}

/// What a loaded model tells the wire.
pub(super) struct Loaded {
    pub(super) template: Template,
    pub(super) vision: bool,
    /// The most tokens each request can hold.
    pub(super) context: u32,
}

/// A prompt longer than its sequence's context, which rig reads as a request
/// the model refused for its length.
#[derive(Debug)]
struct TooLong(String);

impl fmt::Display for TooLong {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TooLong {}

/// Loads the model and serves `queue` until every sender is gone; `ready`
/// gets the load's result first.
pub(super) fn serve(
    files: Files,
    size: Size,
    ready: SyncSender<Result<Loaded>>,
    queue: Receiver<Job>,
) {
    let fitted = fit(&files, size).and_then(|(params, context)| {
        let model = load(&files, &params)?;
        Ok((model, Size { context, ..size }))
    });
    let (model, size) = match fitted {
        Ok(fitted) => fitted,
        Err(error) => return drop(ready.send(Err(error))),
    };
    let engine = Template::new(&model).and_then(|template| {
        let engine = Engine::new(&model, &files, size)?;
        Ok((template, engine))
    });
    let (template, engine) = match engine {
        Ok(loaded) => loaded,
        Err(error) => return drop(ready.send(Err(error))),
    };
    let loaded = Loaded {
        template,
        vision: engine.mtmd.is_some(),
        context: size.context,
    };
    if ready.send(Ok(loaded)).is_ok() {
        engine.run(&queue);
    }
}

fn backend() -> Result<&'static LlamaBackend> {
    static BACKEND: OnceLock<LlamaBackend> = OnceLock::new();
    if let Some(backend) = BACKEND.get() {
        return Ok(backend);
    }
    koharu_llama::send_logs_to_tracing(koharu_llama::LogOptions::default());
    let backend = LlamaBackend::init().context("couldn't start llama.cpp")?;
    Ok(BACKEND.get_or_init(|| backend))
}

/// Fits the model to the device's free memory, and answers how it should load
/// and the context each sequence gets. Fit leaves alone a context and a layer
/// count the caller sets (common/fit.cpp), so it gets neither: it measures from
/// the model's trained context, reduces that, then moves layers to system
/// memory, and the sequences keep to `size` below what it allows.
fn fit(files: &Files, size: Size) -> Result<(Pin<Box<LlamaModelParams>>, u32)> {
    // Fit swaps llama.cpp's global logger while it runs.
    static FITTING: Mutex<()> = Mutex::new(());
    backend()?;
    let mut params = Box::pin(LlamaModelParams::default().with_devices(&[files.device])?);
    let mut context = context_params(size, None, None);
    // Fit doesn't count the projector.
    let projector = match &files.projector {
        Some(projector) => std::fs::metadata(projector)
            .with_context(|| format!("couldn't read {}", projector.display()))?
            .len() as usize,
        None => 0,
    };
    let mut margins = vec![MARGIN + projector; koharu_llama::max_devices()];
    let path = files
        .model
        .to_str()
        .with_context(|| format!("{} has no UTF-8 path", files.model.display()))?;
    let path = CString::new(path)?;
    let fitted = {
        let _fitting = FITTING.lock().unwrap_or_else(PoisonError::into_inner);
        params.as_mut().fit_params(
            &path,
            &mut context,
            &mut margins,
            MIN_CONTEXT.min(size.context),
            GGML_LOG_LEVEL_WARN,
        )
    }
    .with_context(|| format!("{} doesn't fit in free memory", files.model.display()))?;
    let context = fitted
        .n_ctx
        .min(size.sequences.saturating_mul(size.context))
        / size.sequences;
    ensure!(context > 0, "{} got no context", files.model.display());
    Ok((params, context))
}

fn load(files: &Files, params: &LlamaModelParams) -> Result<LlamaModel> {
    LlamaModel::load_from_file(backend()?, &files.model, params)
        .with_context(|| format!("couldn't load {}", files.model.display()))
}

/// The batch a context decodes in: every running sequence decodes one token per
/// step.
fn batch_size(size: Size) -> usize {
    (size.context as usize)
        .min(2048)
        .max(size.sequences as usize)
}

/// A context for `size`. Separate KV streams, so each sequence holds its own
/// tokens; a whole context of `None` is for fit to choose.
fn context_params(size: Size, context: Option<u32>, n_ubatch: Option<usize>) -> LlamaContextParams {
    let n_batch = batch_size(size);
    LlamaContextParams::default()
        .with_n_ctx(context.and_then(NonZeroU32::new))
        .with_n_seq_max(size.sequences)
        .with_kv_unified(false)
        .with_n_batch(n_batch as u32)
        .with_n_ubatch(n_ubatch.unwrap_or_else(|| n_batch.min(512)) as u32)
}

struct Engine<'m> {
    model: &'m LlamaModel,
    context: LlamaContext<'m>,
    mtmd: Option<MtmdContext>,
    size: Size,
    batch: LlamaBatch<'static>,
    n_batch: usize,
    grammars: Option<ParserFactory>,
    /// What each sequence holds from position 0: its tokens up to its first
    /// image, which they stop being a record of.
    held: Vec<Vec<LlamaToken>>,
}

struct Sequence {
    id: i32,
    frames: UnboundedSender<Result<Step, ProviderError>>,
    sampler: LlamaSampler,
    /// The token sampled last, decoded at `position` in the next step.
    next: LlamaToken,
    position: i32,
    max_tokens: Option<u64>,
    prompt_tokens: u64,
    output_tokens: u64,
    /// The bytes of a character whose last token hasn't come yet.
    pending: Vec<u8>,
    /// Whether `held` records this sequence's tokens: not past an image.
    tracked: bool,
}

impl Sequence {
    fn end(&self, length: bool) -> Step {
        Step::End {
            length,
            prompt_tokens: self.prompt_tokens,
            output_tokens: self.output_tokens,
        }
    }
}

impl<'m> Engine<'m> {
    fn new(model: &'m LlamaModel, files: &Files, size: Size) -> Result<Self> {
        let mtmd = files
            .projector
            .as_ref()
            .map(|projector| {
                let path = projector
                    .to_str()
                    .context("the projector's path isn't UTF-8")?;
                let mtmd = MtmdContext::init_from_file(
                    path,
                    model,
                    &MtmdContextParams {
                        use_gpu: true,
                        print_timings: false,
                        ..MtmdContextParams::default()
                    },
                )
                .with_context(|| format!("couldn't load {}", projector.display()))?;
                ensure!(
                    mtmd.support_vision(),
                    "{} reads no images",
                    projector.display()
                );
                Ok(mtmd)
            })
            .transpose()?;
        let n_batch = batch_size(size);
        // An image decoded with non-causal attention must fit in one
        // micro-batch.
        let non_causal = mtmd
            .as_ref()
            .is_some_and(MtmdContext::decode_use_non_causal);
        let params = context_params(
            size,
            Some(size.sequences * size.context),
            non_causal.then_some(n_batch),
        );
        let context = model
            .new_context(backend()?, params)
            .context("couldn't create the llama.cpp context")?;
        Ok(Self {
            model,
            context,
            mtmd,
            size,
            batch: LlamaBatch::new(n_batch, 1),
            n_batch,
            grammars: None,
            held: vec![Vec::new(); size.sequences as usize],
        })
    }

    fn run(mut self, queue: &Receiver<Job>) {
        let mut waiting = VecDeque::new();
        let mut running: Vec<Sequence> = Vec::new();
        let mut free: Vec<i32> = (0..self.size.sequences as i32).rev().collect();
        loop {
            if running.is_empty() && waiting.is_empty() {
                match queue.recv() {
                    Ok(job) => waiting.push_back(job),
                    Err(_) => return,
                }
            }
            waiting.extend(queue.try_iter());
            while !free.is_empty()
                && let Some(job) = waiting.pop_front()
            {
                if job.frames.is_closed() {
                    continue;
                }
                match self.admit(&job, &mut free) {
                    Ok(Some(sequence)) => running.push(sequence),
                    Ok(None) => {}
                    Err(error) => {
                        let _ = job.frames.send(Err(failure(&error)));
                    }
                }
            }
            if !running.is_empty() {
                self.step(&mut running, &mut free);
            }
        }
    }

    /// Decodes `job`'s prompt in a free sequence and samples its first token,
    /// or answers `None` when the request ended before it did.
    fn admit(&mut self, job: &Job, free: &mut Vec<i32>) -> Result<Option<Sequence>> {
        let marker = mtmd_default_marker();
        let (head, tail) = job
            .prompt
            .find(marker)
            .map_or((job.prompt.as_str(), ""), |at| job.prompt.split_at(at));
        let head = self.model.vocab().tokenize(head.as_bytes(), true, true);
        let (chunks, _bitmaps) = if tail.is_empty() {
            (None, Vec::new())
        } else {
            let mtmd = self.mtmd.as_ref().context("this model reads no images")?;
            let bitmaps = job
                .images
                .iter()
                .map(|image| MtmdBitmap::from_buffer(mtmd, image, false))
                .collect::<Result<Vec<_>, _>>()
                .context("couldn't read an image")?;
            let chunks = mtmd.tokenize(
                MtmdInputText {
                    text: tail.to_owned(),
                    add_special: false,
                    parse_special: true,
                },
                &bitmaps.iter().collect::<Vec<_>>(),
            )?;
            (Some(chunks), bitmaps)
        };
        let prompt_tokens = head.len() + chunks.as_ref().map_or(0, MtmdInputChunks::total_tokens);
        let prompt_positions = head.len() as i64
            + chunks
                .as_ref()
                .map_or(0, |chunks| i64::from(chunks.total_positions()));
        ensure!(prompt_positions > 0, "the prompt is empty");
        if prompt_positions >= i64::from(self.size.context) {
            bail!(TooLong(format!(
                "the prompt is too long: it takes {prompt_positions} positions, and the model has {} for each request",
                self.size.context
            )));
        }

        let (id, start) = self.place(&head, chunks.is_none(), free)?;
        match self.decode(job, id, &head, chunks.as_ref(), start, prompt_tokens) {
            Ok(Some(sequence)) => Ok(Some(sequence)),
            Ok(None) => {
                free.push(id);
                Ok(None)
            }
            Err(error) => {
                self.forget(id);
                free.push(id);
                Err(error)
            }
        }
    }

    /// Takes a free sequence for a prompt starting with `head`, holding as much
    /// of it as any sequence does; answers it and how many tokens it holds.
    /// When `logits` is set, the head's last token is left to decode again, as
    /// logits come from it.
    fn place(
        &mut self,
        head: &[LlamaToken],
        logits: bool,
        free: &mut Vec<i32>,
    ) -> Result<(i32, usize)> {
        let (best, common) = self
            .held
            .iter()
            .enumerate()
            .map(|(id, held)| (id as i32, common_prefix(held, head)))
            .max_by_key(|&(id, common)| (common, free.contains(&id)))
            .context("the model has no sequences")?;
        let common = if logits {
            common.min(head.len().saturating_sub(1))
        } else {
            common
        };
        let in_place = free.iter().position(|&id| id == best);
        let id = match in_place.filter(|_| common >= MIN_SHARED) {
            Some(at) => free.swap_remove(at),
            None => self.take_emptiest(free)?,
        };
        if common < MIN_SHARED {
            self.forget(id);
            return Ok((id, 0));
        }
        if id != best {
            // Sequences in separate KV streams copy whole streams; the data
            // moves at the next decode, and what follows is taken off now.
            self.context.copy_kv_cache_seq(best, id, None, None)?;
            self.held[id as usize] = self.held[best as usize].clone();
        }
        if !self
            .context
            .clear_kv_cache_seq(Some(id as u32), Some(common as u32), None)?
        {
            self.forget(id);
            return Ok((id, 0));
        }
        self.held[id as usize].truncate(common);
        Ok((id, common))
    }

    /// The free sequence that holds the least.
    fn take_emptiest(&self, free: &mut Vec<i32>) -> Result<i32> {
        let at = free
            .iter()
            .enumerate()
            .min_by_key(|&(_, &id)| self.held[id as usize].len())
            .map(|(at, _)| at)
            .context("no sequence is free")?;
        Ok(free.swap_remove(at))
    }

    /// Decodes a prompt in sequence `id`, past the `start` tokens it holds.
    fn decode(
        &mut self,
        job: &Job,
        id: i32,
        head: &[LlamaToken],
        chunks: Option<&MtmdInputChunks>,
        start: usize,
        prompt_tokens: usize,
    ) -> Result<Option<Sequence>> {
        for (at, chunk) in head[start..].chunks(self.n_batch).enumerate() {
            if job.frames.is_closed() {
                return Ok(None);
            }
            let first = start + at * self.n_batch;
            self.batch.clear();
            for (offset, &token) in chunk.iter().enumerate() {
                let position = first + offset;
                let last = chunks.is_none() && position + 1 == head.len();
                self.batch.add(token, position as i32, &[id], last)?;
            }
            self.context.decode(&mut self.batch)?;
            self.held[id as usize].extend_from_slice(chunk);
        }
        let mut position = head.len() as i32;
        if let Some(chunks) = chunks {
            if job.frames.is_closed() {
                return Ok(None);
            }
            let mtmd = self.mtmd.as_mut().context("this model reads no images")?;
            position = chunks.eval_chunks(
                mtmd,
                &mut self.context,
                position,
                id,
                self.n_batch as i32,
                true,
            )?;
        }

        let history = head
            .iter()
            .copied()
            .chain(chunks.map(text_tokens).unwrap_or_default());
        let mut sequence = Sequence {
            id,
            frames: job.frames.clone(),
            sampler: self.sampler(&job.sampling, job.schema.as_deref(), history)?,
            next: LlamaToken(0),
            position,
            max_tokens: job.sampling.max_tokens,
            prompt_tokens: prompt_tokens as u64,
            output_tokens: 0,
            pending: Vec::new(),
            tracked: chunks.is_none(),
        };
        let token = sequence.sampler.sample(&self.context, -1);
        Ok(self.take(&mut sequence, token)?.then_some(sequence))
    }

    fn step(&mut self, running: &mut Vec<Sequence>, free: &mut Vec<i32>) {
        running.retain(|sequence| {
            let open = !sequence.frames.is_closed();
            if !open {
                free.push(sequence.id);
            }
            open
        });
        if running.is_empty() {
            return;
        }
        let decoded = (|| -> Result<()> {
            self.batch.clear();
            for sequence in running.iter() {
                self.batch
                    .add(sequence.next, sequence.position, &[sequence.id], true)?;
            }
            Ok(self.context.decode(&mut self.batch)?)
        })();
        if let Err(error) = decoded {
            for sequence in running.drain(..) {
                let _ = sequence.frames.send(Err(failure(&error)));
                self.forget(sequence.id);
                free.push(sequence.id);
            }
            return;
        }
        let mut index = 0;
        running.retain_mut(|sequence| {
            if sequence.tracked {
                self.held[sequence.id as usize].push(sequence.next);
            }
            sequence.position += 1;
            let token = sequence.sampler.sample(&self.context, index);
            index += 1;
            let going = match self.take(sequence, token) {
                Ok(going) => going,
                Err(error) => {
                    let _ = sequence.frames.send(Err(failure(&error)));
                    false
                }
            };
            if !going {
                free.push(sequence.id);
            }
            going
        });
    }

    /// Sends what `token` adds to the reply, or its end; answers whether the
    /// sequence goes on.
    fn take(&self, sequence: &mut Sequence, token: LlamaToken) -> Result<bool> {
        let vocab = self.model.vocab();
        if vocab.is_eog(token) {
            ensure!(
                sequence.pending.is_empty(),
                "the answer ends inside a UTF-8 character"
            );
            let _ = sequence.frames.send(Ok(sequence.end(false)));
            return Ok(false);
        }
        sequence.output_tokens += 1;
        sequence
            .pending
            .extend(vocab.token_to_piece(token, true, None));
        let valid = match std::str::from_utf8(&sequence.pending) {
            Ok(text) => text.len(),
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => bail!("the model wrote bytes that aren't UTF-8"),
        };
        if valid > 0 {
            let text = String::from_utf8_lossy(&sequence.pending[..valid]).into_owned();
            sequence.pending.drain(..valid);
            if sequence.frames.send(Ok(Step::Text(text))).is_err() {
                return Ok(false);
            }
        }
        let limit = sequence
            .max_tokens
            .is_some_and(|max| sequence.output_tokens >= max);
        if limit || sequence.position >= self.size.context as i32 {
            let _ = sequence.frames.send(Ok(sequence.end(true)));
            return Ok(false);
        }
        sequence.next = token;
        Ok(true)
    }

    fn sampler(
        &mut self,
        sampling: &Sampling,
        schema: Option<&str>,
        history: impl IntoIterator<Item = LlamaToken>,
    ) -> Result<LlamaSampler> {
        let mut samplers = Vec::new();
        if let Some(schema) = schema {
            if self.grammars.is_none() {
                let tokens = LlamaSampler::llguidance_tok_env(self.model);
                self.grammars = Some(
                    ParserFactory::new_simple(&tokens)
                        .map_err(|error| anyhow::anyhow!("{error}"))?,
                );
            }
            let factory = self.grammars.as_ref().context("no grammar factory")?;
            let grammar = TopLevelGrammar::from_tagged_str("json_schema", schema)
                .context("couldn't compile the JSON schema")?;
            let parser = factory
                .create_parser(grammar)
                .context("couldn't compile the JSON schema")?;
            samplers.push(LlamaSampler::from(Matcher::new(Ok(parser))));
        }
        let repeat = sampling.repeat_penalty.unwrap_or(1.0);
        let frequency = sampling.frequency_penalty.unwrap_or(0.0);
        let presence = sampling.presence_penalty.unwrap_or(0.0);
        if repeat != 1.0 || frequency != 0.0 || presence != 0.0 {
            let mut penalties = LlamaSampler::penalties(
                self.model.n_vocab(),
                // llama.cpp's default window.
                sampling.repeat_last_n.unwrap_or(64),
                repeat,
                frequency,
                presence,
            );
            penalties.accept_many(history);
            samplers.push(penalties);
        }
        let temperature = sampling.temperature.unwrap_or(1.0);
        if temperature == 0.0 {
            samplers.push(LlamaSampler::greedy());
        } else {
            if let Some(top_k) = sampling.top_k {
                samplers.push(LlamaSampler::top_k(i32::try_from(top_k)?));
            }
            if let Some(top_p) = sampling.top_p {
                samplers.push(LlamaSampler::top_p(top_p as f32, 1));
            }
            if let Some(min_p) = sampling.min_p {
                samplers.push(LlamaSampler::min_p(min_p, 1));
            }
            samplers.push(LlamaSampler::temp(temperature as f32));
            // llama.h's LLAMA_DEFAULT_SEED, a random seed.
            let seed = sampling.seed.map_or(Ok(u32::MAX), u32::try_from)?;
            samplers.push(LlamaSampler::dist(seed));
        }
        Ok(LlamaSampler::chain_simple(samplers))
    }

    /// Empties sequence `id`.
    fn forget(&mut self, id: i32) {
        // A whole sequence's removal always succeeds.
        let _ = self.context.clear_kv_cache_seq(Some(id as u32), None, None);
        self.held[id as usize].clear();
    }
}

fn common_prefix(held: &[LlamaToken], head: &[LlamaToken]) -> usize {
    held.iter()
        .zip(head)
        .take_while(|(held, head)| held == head)
        .count()
}

fn text_tokens(chunks: &MtmdInputChunks) -> Vec<LlamaToken> {
    (0..chunks.len())
        .filter_map(|index| chunks.get(index))
        .filter_map(|chunk| chunk.text_tokens().map(<[LlamaToken]>::to_vec))
        .flatten()
        .collect()
}

fn failure(error: &anyhow::Error) -> ProviderError {
    let message = format!("{error:#}");
    if error.is::<TooLong>() {
        return ProviderError::Relayed(Box::new(ErrorReport::context_overflow(message)));
    }
    ProviderError::Provider(message)
}
