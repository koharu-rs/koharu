//! The thread that owns a loaded model: one llama.cpp context whose
//! sequences decode together, after llama.cpp's `parallel` example
//! (https://github.com/ggml-org/llama.cpp/blob/7aca0d2ea8d92b56911233b822db01f9fbb1ae50/examples/parallel/parallel.cpp#L264-L476).
//! Sequence 0 holds the instructions requests share, which `parallel` keeps
//! there too; requests run in sequences 1 to `Size::sequences`.

use std::{
    collections::VecDeque,
    num::NonZeroU32,
    path::PathBuf,
    sync::{
        OnceLock,
        mpsc::{Receiver, SyncSender},
    },
};

use anyhow::{Context, Result, bail, ensure};
use llama_cpp_2::{
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
use llguidance::{Matcher, ParserFactory, api::TopLevelGrammar};
use rig_core::error::ProviderError;
use tokio::sync::mpsc::UnboundedSender;

use super::{Sampling, Size, template::Template};

/// Only instructions are worth sharing; a shorter common prefix is the chat
/// template's header.
const MIN_SHARED: usize = 64;

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
pub enum Step {
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
}

/// Loads the model and serves `queue` until every sender is gone; `ready`
/// gets the load's result first.
pub(super) fn serve(
    files: Files,
    size: Size,
    ready: SyncSender<Result<Loaded>>,
    queue: Receiver<Job>,
) {
    let model = match load(&files) {
        Ok(model) => model,
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
    llama_cpp_2::send_logs_to_tracing(llama_cpp_2::LogOptions::default());
    let backend = LlamaBackend::init().context("couldn't start llama.cpp")?;
    Ok(BACKEND.get_or_init(|| backend))
}

fn load(files: &Files) -> Result<LlamaModel> {
    let params = LlamaModelParams::default()
        .with_n_gpu_layers(u32::MAX)
        .with_devices(&[files.device])?;
    LlamaModel::load_from_file(backend()?, &files.model, &params)
        .with_context(|| format!("couldn't load {}", files.model.display()))
}

struct Engine<'m> {
    model: &'m LlamaModel,
    context: LlamaContext<'m>,
    mtmd: Option<MtmdContext>,
    size: Size,
    batch: LlamaBatch<'static>,
    n_batch: usize,
    grammars: Option<ParserFactory>,
    /// What sequence 0 holds.
    shared: Vec<LlamaToken>,
    /// The last prompt admitted, whose start the next one may share.
    last: Vec<LlamaToken>,
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
        let context = size.context as usize;
        // Every running sequence decodes one token per step.
        let n_batch = context.min(2048).max(size.sequences as usize);
        // An image decoded with non-causal attention must fit in one
        // micro-batch.
        let non_causal = mtmd
            .as_ref()
            .is_some_and(MtmdContext::decode_use_non_causal);
        let n_ubatch = if non_causal {
            n_batch
        } else {
            n_batch.min(512)
        };
        let sequences = size.sequences + 1;
        let params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(sequences * size.context))
            .with_n_seq_max(sequences)
            .with_kv_unified(false)
            .with_n_batch(n_batch as u32)
            .with_n_ubatch(n_ubatch as u32);
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
            shared: Vec::new(),
            last: Vec::new(),
        })
    }

    fn run(mut self, queue: &Receiver<Job>) {
        let mut waiting = VecDeque::new();
        let mut running: Vec<Sequence> = Vec::new();
        let mut free: Vec<i32> = (1..=self.size.sequences as i32).rev().collect();
        loop {
            if running.is_empty() && waiting.is_empty() {
                match queue.recv() {
                    Ok(job) => waiting.push_back(job),
                    Err(_) => return,
                }
            }
            waiting.extend(queue.try_iter());
            while let Some(&id) = free.last()
                && let Some(job) = waiting.pop_front()
            {
                if job.frames.is_closed() {
                    continue;
                }
                match self.admit(&job, id) {
                    Ok(Some(sequence)) => {
                        free.pop();
                        running.push(sequence);
                    }
                    Ok(None) => self.clear(id),
                    Err(error) => {
                        self.clear(id);
                        let _ = job.frames.send(Err(failure(&error)));
                    }
                }
            }
            if !running.is_empty() {
                self.step(&mut running, &mut free);
            }
        }
    }

    /// Decodes `job`'s prompt in sequence `id` and samples its first token,
    /// or answers `None` when the request was dropped before it ended.
    fn admit(&mut self, job: &Job, id: i32) -> Result<Option<Sequence>> {
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
        ensure!(
            prompt_positions < i64::from(self.size.context),
            "the prompt takes {prompt_positions} positions, and the model was loaded with {} for each request",
            self.size.context
        );

        let mut start = self.share(&head, id)?;
        // Logits come from the head's last token when no image follows, so
        // that token is decoded again even when it was shared.
        if chunks.is_none() && start == head.len() {
            start -= 1;
            if !self
                .context
                .clear_kv_cache_seq(Some(id as u32), Some(start as u32), None)?
            {
                self.clear(id);
                start = 0;
            }
        }
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
        }
        let mut position = head.len() as i32;
        if let Some(chunks) = &chunks {
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
            .chain(chunks.as_ref().map(text_tokens).unwrap_or_default());
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
        };
        let token = sequence.sampler.sample(&self.context, -1);
        Ok(self.take(&mut sequence, token)?.then_some(sequence))
    }

    /// Copies the instructions sequence 0 holds into `id` when `head` starts
    /// with them, after decoding them there first when `head` and the last
    /// prompt start alike; answers how many tokens `id` now holds.
    fn share(&mut self, head: &[LlamaToken], id: i32) -> Result<usize> {
        let last = std::mem::replace(&mut self.last, head.to_vec());
        if self.shared.is_empty() || !head.starts_with(&self.shared) {
            let common = last
                .iter()
                .zip(head)
                .take_while(|(last, head)| last == head)
                .count();
            if common < MIN_SHARED {
                return Ok(0);
            }
            self.shared.clear();
            self.clear(0);
            for (at, chunk) in head[..common].chunks(self.n_batch).enumerate() {
                self.batch.clear();
                for (offset, &token) in chunk.iter().enumerate() {
                    self.batch
                        .add(token, (at * self.n_batch + offset) as i32, &[0], false)?;
                }
                self.context.decode(&mut self.batch)?;
            }
            self.shared = head[..common].to_vec();
        }
        // Sequences in separate KV streams copy whole streams.
        self.context.copy_kv_cache_seq(0, id, None, None)?;
        Ok(self.shared.len())
    }

    fn step(&mut self, running: &mut Vec<Sequence>, free: &mut Vec<i32>) {
        let mut dropped = Vec::new();
        running.retain(|sequence| {
            let open = !sequence.frames.is_closed();
            if !open {
                dropped.push(sequence.id);
            }
            open
        });
        for id in dropped {
            self.clear(id);
            free.push(id);
        }
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
                self.clear(sequence.id);
                free.push(sequence.id);
            }
            return;
        }
        let mut index = 0;
        running.retain_mut(|sequence| {
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
                self.clear(sequence.id);
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

    fn clear(&mut self, id: i32) {
        // A whole sequence's removal always succeeds.
        let _ = self.context.clear_kv_cache_seq(Some(id as u32), None, None);
    }
}

fn text_tokens(chunks: &MtmdInputChunks) -> Vec<LlamaToken> {
    (0..chunks.len())
        .filter_map(|index| chunks.get(index))
        .filter_map(|chunk| chunk.text_tokens().map(<[LlamaToken]>::to_vec))
        .flatten()
        .collect()
}

fn failure(error: &anyhow::Error) -> ProviderError {
    ProviderError::Provider(format!("{error:#}"))
}
