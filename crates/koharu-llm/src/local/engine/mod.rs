//! The thread that owns a loaded model: one llama.cpp context whose
//! sequences decode together, after llama.cpp's `parallel` example
//! (https://github.com/ggml-org/llama.cpp/blob/7aca0d2ea8d92b56911233b822db01f9fbb1ae50/examples/parallel/parallel.cpp#L264-L476).
//! A sequence keeps what it decoded after its request ends, and a new request
//! reuses the longest common prefix any sequence holds, as llama-server picks a
//! slot (`get_available_slot`, tools/server/server-context.cpp at b12143). A
//! recurrent or hybrid model's state can't be cut back to that prefix, so its
//! sequences keep a checkpoint of their recurrent state just short of each
//! prompt's end, and go back to it, as llama-server's context checkpoints do.

mod load;
mod picker;
mod reuse;

use std::{
    collections::VecDeque,
    fmt,
    path::PathBuf,
    sync::mpsc::{Receiver, SyncSender},
};

use anyhow::{Context, Result, bail, ensure};
use koharu_llama::{
    context::LlamaContext,
    llama_batch::LlamaBatch,
    model::LlamaModel,
    mtmd::{MtmdBitmap, MtmdContext, MtmdInputChunks, MtmdInputText, mtmd_default_marker},
    token::LlamaToken,
};
use llguidance::ParserFactory;
use rig_core::error::{ErrorReport, ProviderError};
use tokio::sync::mpsc::UnboundedSender;

use super::{Sampling, Size, template::Template};
use picker::Picker;
use reuse::Checkpoint;

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
    /// The opening and closing markers of the reasoning the reply may write
    /// before `schema` holds it.
    pub(super) answer_after: Option<(&'static str, &'static str)>,
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
    let (model, size) = match load::model(&files, size) {
        Ok(loaded) => loaded,
        Err(error) => return drop(ready.send(Err(error))),
    };
    let started = Template::new(&model)
        .and_then(|template| Ok((template, Engine::new(&model, &files, size)?)));
    let (template, engine) = match started {
        Ok(started) => started,
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
    /// Whether the model's state is recurrent, in whole or in part.
    recurrent: bool,
    /// Each sequence's recurrent state after its first `at` held tokens.
    checkpoints: Vec<Option<Checkpoint>>,
}

struct Sequence {
    id: i32,
    frames: UnboundedSender<Result<Step, ProviderError>>,
    picker: Picker,
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
            .as_deref()
            .map(|projector| load::projector(model, projector))
            .transpose()?;
        let n_batch = load::batch_size(size);
        Ok(Self {
            model,
            context: load::context(model, size, mtmd.as_ref())?,
            mtmd,
            size,
            batch: LlamaBatch::new(n_batch, 1),
            n_batch,
            grammars: None,
            held: vec![Vec::new(); size.sequences as usize],
            recurrent: model.is_recurrent() || model.is_hybrid(),
            checkpoints: (0..size.sequences).map(|_| None).collect(),
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
        let Some(from) = self.checkpoint(job, id, head, start)? else {
            return Ok(None);
        };
        if !self.feed(job, id, head, from..head.len(), chunks.is_none())? {
            return Ok(None);
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

        let history: Vec<LlamaToken> = head
            .iter()
            .copied()
            .chain(chunks.map(text_tokens).unwrap_or_default())
            .collect();
        let mut sequence = Sequence {
            id,
            frames: job.frames.clone(),
            picker: self.picker(job, &history)?,
            next: LlamaToken(0),
            position,
            max_tokens: job.sampling.max_tokens,
            prompt_tokens: prompt_tokens as u64,
            output_tokens: 0,
            pending: Vec::new(),
            tracked: chunks.is_none(),
        };
        let token = sequence.picker.pick(self.model, &self.context, -1);
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
            let token = sequence.picker.pick(self.model, &self.context, index);
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

    /// Decodes `range` of `head` in sequence `id`, in batches, with logits
    /// for the head's last token when `logits` is set. Answers whether the
    /// request is still open.
    fn feed(
        &mut self,
        job: &Job,
        id: i32,
        head: &[LlamaToken],
        range: std::ops::Range<usize>,
        logits: bool,
    ) -> Result<bool> {
        let start = range.start;
        for (at, chunk) in head[range].chunks(self.n_batch).enumerate() {
            if job.frames.is_closed() {
                return Ok(false);
            }
            let first = start + at * self.n_batch;
            self.batch.clear();
            for (offset, &token) in chunk.iter().enumerate() {
                let position = first + offset;
                let last = logits && position + 1 == head.len();
                self.batch.add(token, position as i32, &[id], last)?;
            }
            self.context.decode(&mut self.batch)?;
            self.held[id as usize].extend_from_slice(chunk);
        }
        Ok(true)
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
    let message = format!("{error:#}");
    if error.is::<TooLong>() {
        return ProviderError::Relayed(Box::new(ErrorReport::context_overflow(message)));
    }
    ProviderError::Provider(message)
}
