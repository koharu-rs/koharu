//! Koharu's llama.cpp engine as a rig model: a completion wire whose payload
//! is the prompt the model's chat template renders, with its sampling
//! settings, and whose transport is the thread that owns the model, with the
//! model's markup read off its replies on the way back.

pub(crate) mod catalog;
mod conversation;
mod engine;
pub(crate) mod markup;
#[cfg(test)]
pub(crate) mod scripted;
mod template;
mod wire;

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, mpsc},
};

use anyhow::{Context, Result, ensure};
use rig_core::{
    catalog::{ModelFacts, ModelSpec, ReasoningSupport},
    completion::Effort,
    driver::{DynModel, Exchange, Model, Opened, Opening, Transport},
    error::ProviderError,
    operation::Completion,
    providers::registry::ProviderId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use engine::{Files, Job, Step};
use markup::{Item, Markup, Parser};
use wire::{Gguf, Part, Prompt};

use crate::KOHARU;

/// How many requests a loaded model decodes together, the most tokens each
/// may hold, prompt and answer together, and whether it reads images. The KV
/// cache for all of them is allocated when the model loads, which may give
/// each request less if the device has too little free memory; the model's
/// spec says what it got.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Size {
    pub sequences: u32,
    pub context: u32,
    /// Whether the model's projector is fetched and loaded, so it reads
    /// images; without it, its memory goes to the model.
    pub images: bool,
}

impl Size {
    /// What the agent asks of a local model: one conversation at a time, of
    /// text.
    pub const AGENT: Self = Self {
        sequences: 1,
        context: 65_536,
        images: false,
    };
}

/// A local request's sampling settings: rig's temperature, token limit,
/// `top_p`, and seed, and llama.cpp's own settings, which a request sets in
/// its `additional_params`. An unset one leaves its sampler out, and the
/// model's distribution as it is.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Sampling {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_penalty: Option<f32>,
    /// How many of the last tokens the penalties look at; -1 for all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_last_n: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,
    /// What the chat template gets as `enable_thinking`; off by default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_thinking: Option<bool>,
    /// What the chat template gets as `reasoning_effort`, which only some
    /// read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<Effort>,
}

impl Sampling {
    fn check(&self) -> Result<(), String> {
        let between = |value: Option<f64>, name: &str, low: f64, high: f64| match value {
            Some(value) if !(value.is_finite() && (low..=high).contains(&value)) => Err(format!(
                "{name} must be between {low} and {high}, not {value}"
            )),
            _ => Ok(()),
        };
        between(self.temperature, "temperature", 0.0, f64::MAX)?;
        between(self.top_p, "top_p", 0.0, 1.0)?;
        between(self.min_p.map(f64::from), "min_p", 0.0, 1.0)?;
        between(
            self.repeat_penalty.map(f64::from),
            "repeat_penalty",
            f64::MIN_POSITIVE,
            f64::MAX,
        )?;
        between(
            self.frequency_penalty.map(f64::from),
            "frequency_penalty",
            f64::MIN,
            f64::MAX,
        )?;
        between(
            self.presence_penalty.map(f64::from),
            "presence_penalty",
            f64::MIN,
            f64::MAX,
        )?;
        if self.top_k == Some(0) {
            return Err("top_k must be at least 1".to_owned());
        }
        if self.repeat_last_n.is_some_and(|last| last < -1) {
            return Err("repeat_last_n must be -1 or more".to_owned());
        }
        Ok(())
    }
}

/// What Koharu's catalog says of one of its models beyond its spec.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Local {
    /// The default first.
    pub quantizations: Vec<Quantization>,
    /// The languages it translates into, as BCP 47 tags; `None` for any.
    pub languages: Option<&'static [&'static str]>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Quantization {
    pub id: &'static str,
    pub name: &'static str,
    pub downloaded: bool,
}

/// Koharu's catalog's facts on `spec`, when it's one of Koharu's models.
#[must_use]
pub fn local(spec: &ModelSpec) -> Option<Local> {
    if spec.provider.vendor() != KOHARU {
        return None;
    }
    let entry = catalog::entry(&spec.id).ok()?;
    Some(Local {
        quantizations: entry
            .quantizations
            .iter()
            .map(|quantization| Quantization {
                id: quantization.id,
                name: quantization.name,
                downloaded: std::iter::once(quantization.file)
                    .chain(entry.projector)
                    .all(|file| entry.repository.exists(file)),
            })
            .collect(),
        languages: entry.languages,
    })
}

/// Every model of Koharu's catalog, as rig describes a model; how much
/// context one gets is known once it loads.
pub(crate) fn specs() -> Vec<ModelSpec> {
    catalog::MODELS.iter().map(catalog::Entry::spec).collect()
}

/// What rig and a conversation's context know of one of Koharu's models. A
/// model of a specialized `kind`, such as translation, calls no tools,
/// whatever markup its template writes; it reasons when its template does, as
/// Index-Translate's opens a think block unless told not to.
pub(crate) fn spec(id: &str, markup: Option<&Markup>, kind: Option<&str>) -> ModelSpec {
    let general = kind.is_none();
    let mut spec = ModelSpec::new(vendor(), id)
        .with_tools(general && markup.is_some_and(|markup| markup.call.is_some()));
    if markup.is_some_and(|markup| markup.reasoning.is_some()) {
        spec = spec.with_reasoning(ReasoningSupport::Unknown { default: None });
    }
    spec.structured_output = true;
    spec.details.kind = kind.map(str::to_owned);
    spec
}

/// The vendor Koharu's own models are filed under.
fn vendor() -> ProviderId {
    ProviderId::declare(KOHARU).expect("`koharu` spells a vendor")
}

/// The catalog model `id` names, at the quantization after a colon or its
/// first, its files fetched first if the store doesn't have them; its
/// projector only when `size` asks for images.
pub(crate) async fn connect(id: &str, size: Size) -> Result<DynModel<Completion>> {
    let (id, quantization) = match id.split_once(':') {
        Some((id, quantization)) => (id, Some(quantization)),
        None => (id, None),
    };
    let entry = catalog::entry(id)?;
    let quantization = entry.quantization(quantization)?;
    let (model, projector) = match entry.projector.filter(|_| size.images) {
        Some(projector) => {
            let [model, projector] = entry
                .repository
                .resolve([quantization.file, projector])
                .await?;
            (model, Some(projector))
        }
        None => {
            let [model] = entry.repository.resolve([quantization.file]).await?;
            (model, None)
        }
    };
    let loading = Loading {
        spec: entry.spec(),
        model,
        projector,
        defaults: entry.sampling.clone(),
        markup: entry.markup,
    };
    load(loading, size).await
}

/// What to load: the model's spec, which loading completes, and its files.
pub(crate) struct Loading {
    pub(crate) spec: ModelSpec,
    pub(crate) model: PathBuf,
    pub(crate) projector: Option<PathBuf>,
    /// The sampling settings a request starts from.
    pub(crate) defaults: Sampling,
    pub(crate) markup: Option<&'static Markup>,
}

/// Loads a model, with its projector when it reads images, on the runtime's
/// device.
pub(crate) async fn load(loading: Loading, size: Size) -> Result<DynModel<Completion>> {
    ensure!(
        size.sequences > 0 && size.context > 0,
        "a local model needs at least one sequence and one token of context"
    );
    let device = koharu_runtime::device().await?.index;
    tokio::task::spawn_blocking(move || {
        let Loading {
            spec,
            model,
            projector,
            mut defaults,
            markup,
        } = loading;
        let (jobs, queue) = mpsc::channel();
        let (ready, loaded) = mpsc::sync_channel(1);
        let files = Files {
            model,
            projector,
            device,
        };
        std::thread::Builder::new()
            .name(format!("llm {}", spec.id))
            .spawn(move || engine::serve(files, size, ready, queue))?;
        let loaded = loaded.recv().context("the model's thread stopped")??;
        // An answer, reasoning included, may take the quarter of the context
        // a conversation's `Context` keeps free for it.
        defaults
            .max_tokens
            .get_or_insert(u64::from(loaded.context) / 4);
        let Value::Object(defaults) = serde_json::to_value(defaults)? else {
            unreachable!("sampling settings serialize to an object")
        };
        let mut spec = spec.with_context_window(loaded.context);
        spec.input.image = loaded.vision;
        let wire = Gguf {
            id: spec.id.clone(),
            template: Arc::new(loaded.template),
            marker: koharu_llama::mtmd::mtmd_default_marker().to_owned(),
            vision: loaded.vision,
            defaults,
            markup,
            facts: ModelFacts::new(spec),
        };
        Ok(Model::new(wire, Queue(jobs)).erase())
    })
    .await?
}

/// The transport: the queue of the thread that owns the model, and the reply's
/// markup read as it comes back. The thread ends when the last handle is
/// dropped.
#[derive(Clone)]
pub(crate) struct Queue(mpsc::Sender<Job>);

impl Transport<Gguf> for Queue {
    fn send(&self, prompt: Prompt, _exchange: Exchange) -> Opening<Part> {
        let jobs = self.0.clone();
        Opening::new(async move {
            let (frames, replies) = tokio::sync::mpsc::unbounded_channel();
            jobs.send(Job {
                prompt: prompt.text,
                images: prompt.images,
                sampling: prompt.sampling,
                schema: prompt.schema,
                answer_after: prompt.answer_after,
                frames,
            })
            .map_err(|_| ProviderError::Provider("the local model stopped".to_owned()))?;
            // Dropping the stream closes the channel, which ends the request
            // at the engine's next step.
            let reply = Parsed {
                replies,
                parser: prompt.parser,
                ready: VecDeque::new(),
            };
            Ok(Opened::new(futures::stream::unfold(
                reply,
                |mut reply| async move {
                    let part = reply.next().await?;
                    Some((part, reply))
                },
            )))
        })
    }
}

/// The engine's steps, read into parts.
struct Parsed {
    replies: tokio::sync::mpsc::UnboundedReceiver<Result<Step, ProviderError>>,
    parser: Parser,
    ready: VecDeque<Result<Part, ProviderError>>,
}

impl Parsed {
    async fn next(&mut self) -> Option<Result<Part, ProviderError>> {
        loop {
            if let Some(part) = self.ready.pop_front() {
                return Some(part);
            }
            match self.replies.recv().await? {
                Ok(Step::Text(text)) => self.take(|parser| parser.feed(&text)),
                Ok(Step::End {
                    length,
                    prompt_tokens,
                    output_tokens,
                }) => {
                    self.take(Parser::finish);
                    self.ready.push_back(Ok(Part::End {
                        length,
                        prompt_tokens,
                        output_tokens,
                    }));
                }
                Err(error) => return Some(Err(error)),
            }
        }
    }

    #[expect(clippy::result_large_err, reason = "rig's provider error is rig's")]
    fn take(&mut self, read: impl FnOnce(&mut Parser) -> Vec<Result<Item, String>>) {
        self.ready.extend(
            read(&mut self.parser)
                .into_iter()
                .map(|item| item.map(Part::from).map_err(ProviderError::Provider)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{markup::FUNCTION_TAGS, *};

    #[test]
    fn the_spec_says_what_the_model_can_do() {
        let general = spec("m", Some(&FUNCTION_TAGS), None);
        assert!(general.tools && general.reasoning.supported());
        assert!(!spec("m", None, None).tools);
        // A translation model's template may write a general model's markup,
        // whose reasoning it keeps and whose tool calls it doesn't.
        let translation = spec("m", Some(&FUNCTION_TAGS), Some(crate::TRANSLATION));
        assert!(!translation.tools && translation.reasoning.supported());
        assert_eq!(
            translation.details.kind.as_deref(),
            Some(crate::TRANSLATION)
        );
    }
}
