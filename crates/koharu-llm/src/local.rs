//! Koharu's llama.cpp engine as a rig model: a completion wire whose payload
//! is the prompt the model's chat template renders, with its sampling
//! settings, and whose transport is the thread that owns the model.

pub(crate) mod catalog;
mod engine;
mod template;

use std::{
    marker::PhantomData,
    path::PathBuf,
    sync::{Arc, mpsc},
};

use anyhow::{Context, Result, ensure};
use base64::Engine as _;
use rig_core::{
    completion::{
        Accepts, AssistantContent, CompletionRequest, FinishReason, Message, ReplayTarget, Usage,
        message::{Api, DocumentSourceKind, Image, UserContent},
        options::{FinalBody, Mapping, OptionFields, OptionMap, RawAt, Reasoning, request_params},
    },
    driver::{DynModel, Exchange, Model, Opened, Opening, Transport},
    error::{EncodeError, ProviderError},
    operation::{Block, Completion, Finish},
    wire::{
        Decoder, Descriptor, Flow, Mode, Out, Wire, WireEvent,
        document::{Reassemble, Serves},
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub use engine::Step;
use engine::{Files, Job};
use template::Template;

/// How many requests a loaded model decodes together, and how many tokens
/// each holds, prompt and answer together. The KV cache for all of them is
/// allocated when the model loads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Size {
    pub sequences: u32,
    pub context: u32,
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

/// Loads `model`, with `projector` when it reads images, on the runtime's
/// device; `defaults` are the sampling settings a request starts from.
pub(crate) async fn load(
    id: String,
    model: PathBuf,
    projector: Option<PathBuf>,
    size: Size,
    defaults: Sampling,
) -> Result<DynModel<Completion>> {
    ensure!(
        size.sequences > 0 && size.context > 0,
        "a local model needs at least one sequence and one token of context"
    );
    let runtime = koharu_runtime::Runtime::start().await?;
    let name = &runtime.device().name;
    let device = llama_cpp_2::list_llama_ggml_backend_devices()
        .into_iter()
        .find(|device| &device.name == name)
        .with_context(|| format!("llama.cpp doesn't list {name}"))?
        .index;
    tokio::task::spawn_blocking(move || {
        let (jobs, queue) = mpsc::channel();
        let (ready, loaded) = mpsc::sync_channel(1);
        let files = Files {
            model,
            projector,
            device,
        };
        std::thread::Builder::new()
            .name(format!("llm {id}"))
            .spawn(move || engine::serve(files, size, ready, queue))?;
        let loaded = loaded.recv().context("the model's thread stopped")??;
        let Value::Object(defaults) = serde_json::to_value(defaults)? else {
            unreachable!("sampling settings serialize to an object")
        };
        let wire = Local {
            id,
            template: Arc::new(loaded.template),
            vision: loaded.vision,
            defaults,
        };
        Ok(Model::new(wire, Engine(jobs)).erase())
    })
    .await?
}

/// The wire: the model's chat template, and what it reads.
#[derive(Clone, Debug)]
pub(crate) struct Local {
    id: String,
    template: Arc<Template>,
    vision: bool,
    /// The sampling settings a request starts from.
    defaults: Map<String, Value>,
}

/// What the engine runs for one request.
pub(crate) struct Prompt {
    text: String,
    images: Vec<Vec<u8>>,
    sampling: Sampling,
    schema: Option<String>,
}

impl Wire for Local {
    type Op = Completion;
    type Payload = Prompt;
    type Frame = Step;
    type Event = Step;
    type Decoder<'id> = Reply<'id>;
    type Reassembler = Document;

    fn describe(&self) -> Descriptor<'_> {
        Descriptor::new("local")
            .model(self.id.as_str())
            .replay(self)
    }

    #[expect(clippy::result_large_err, reason = "rig's encode error is rig's")]
    fn encode(&self, request: CompletionRequest, _mode: Mode) -> Result<Prompt, EncodeError> {
        if !request.tools.is_empty() {
            return Err(EncodeError::request("local models take no tools"));
        }
        // Below the request's options and `additional_params`: the model's
        // defaults, under the request's temperature and token limit.
        let mut base = self.defaults.clone();
        if let Some(temperature) = request.temperature {
            base.insert("temperature".to_owned(), json!(temperature));
        }
        if let Some(max_tokens) = request.max_tokens {
            base.insert("max_tokens".to_owned(), json!(max_tokens));
        }
        let body: FinalBody = request_params(self, &request, |_| Ok(base), RawAt::Top, &[])?;
        let sampling: Sampling = body.deserialize().map_err(EncodeError::request)?;
        sampling.check().map_err(EncodeError::request)?;

        let marker = llama_cpp_2::mtmd::mtmd_default_marker();
        let mut images = Vec::new();
        let mut messages = Vec::new();
        for message in &request.chat_history {
            messages.push(match &**message {
                Message::System { content } => ("system", content.clone()),
                Message::User { content } => {
                    let mut text = String::new();
                    for part in content {
                        match part {
                            UserContent::Text(part) => text.push_str(&part.text),
                            UserContent::Image(image) => {
                                images.push(bytes(image).map_err(EncodeError::request)?);
                                text.push_str(marker);
                            }
                            _ => {
                                return Err(EncodeError::request(
                                    "a local model reads only text and images",
                                ));
                            }
                        }
                    }
                    ("user", text)
                }
                Message::Assistant(turn) => {
                    let text = turn
                        .content
                        .iter()
                        .filter_map(|block| match block {
                            AssistantContent::Text(text) => Some(text.text.as_str()),
                            _ => None,
                        })
                        .collect();
                    ("assistant", text)
                }
            });
        }
        let text = self
            .template
            .render(&messages, sampling.enable_thinking.unwrap_or(false))
            .map_err(|error| EncodeError::request(format!("{error:#}")))?;
        let schema = request
            .output_schema
            .map(|schema| {
                let mut schema = schema.to_value();
                // llguidance's default allows any whitespace between tokens;
                // these are its compact separators.
                if let Some(schema) = schema.as_object_mut() {
                    schema.entry("x-guidance").or_insert_with(|| {
                        json!({
                            "item_separator": ", ",
                            "key_separator": ": ",
                            "whitespace_flexible": false,
                        })
                    });
                }
                serde_json::to_string(&schema)
            })
            .transpose()?;
        Ok(Prompt {
            text,
            images,
            sampling,
            schema,
        })
    }

    fn decoder<'id>(&self) -> Reply<'id> {
        Reply(PhantomData)
    }
}

fn bytes(image: &Image) -> Result<Vec<u8>, String> {
    match &image.data {
        DocumentSourceKind::Raw(bytes) => Ok(bytes.clone()),
        DocumentSourceKind::Base64(data) => base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|error| error.to_string()),
        _ => Err("a local model reads an image's bytes, not a URL or file id".to_owned()),
    }
}

impl ReplayTarget for Local {
    fn api(&self) -> Api {
        Api::from_static("koharu.local")
    }

    fn provider(&self) -> &str {
        "local"
    }

    fn model(&self) -> &str {
        &self.id
    }

    fn accepts(&self, _model: &str) -> Accepts {
        Accepts {
            user_images: self.vision,
            assistant_images: false,
            tool_result_images: false,
            tools: false,
            foreign_reasoning: false,
        }
    }

    fn map_options(&self, _request: &CompletionRequest, fields: OptionFields<'_>) -> OptionMap {
        const NONE: &str = "the local engine has no such setting";
        let OptionFields {
            reasoning,
            cache,
            service_tier,
            verbosity,
            parallel_tool_calls,
            top_p,
            seed,
            stop,
            cache_key,
        } = fields;
        OptionMap {
            reasoning: Mapping::of(reasoning, |reasoning| match reasoning {
                Reasoning::Off => Mapping::Send(json!({"enable_thinking": false})),
                _ => Mapping::unsupported(
                    "the local engine doesn't separate a model's reasoning from its answer",
                ),
            }),
            cache: Mapping::of(cache, |_| {
                Mapping::unsupported("the local engine shares instructions on its own")
            }),
            service_tier: Mapping::of(service_tier, |_| Mapping::unsupported(NONE)),
            verbosity: Mapping::of(verbosity, |_| Mapping::unsupported(NONE)),
            parallel_tool_calls: Mapping::of(parallel_tool_calls, |_| {
                Mapping::unsupported("local models take no tools")
            }),
            top_p: Mapping::of(top_p, |top_p| Mapping::Send(json!({"top_p": top_p}))),
            seed: Mapping::of(seed, |seed| Mapping::Send(json!({"seed": seed}))),
            stop: Mapping::of_stop(stop, |_| {
                Mapping::unsupported("the local engine takes no stop sequences")
            }),
            cache_key: Mapping::unrouted(cache_key),
        }
    }
}

/// The decoder: text as it comes, then the end.
pub(crate) struct Reply<'id>(PhantomData<fn(&'id ()) -> &'id ()>);

impl<'id> Decoder<'id, Completion, Step> for Reply<'id> {
    type Event = Step;

    fn classify(&self, frame: Step) -> WireEvent<Step> {
        WireEvent::Known(frame)
    }

    fn decode(&mut self, step: Step, mut out: Out<'id, Completion>) -> Result<Flow, ProviderError> {
        match step {
            Step::Text(text) => {
                out.run(Block::Text, &text)?;
                Ok(Flow::More)
            }
            Step::End {
                length,
                prompt_tokens,
                output_tokens,
            } => {
                out.end_run()?;
                let mut usage = Usage::default();
                usage.input_tokens = Some(prompt_tokens);
                usage.output_tokens = Some(output_tokens);
                Ok(out.end(Finish {
                    usage,
                    reason: Some(if length {
                        FinishReason::Length
                    } else {
                        FinishReason::Stop
                    }),
                    ..Finish::default()
                }))
            }
        }
    }
}

/// The reply as one document: its text and how it ended.
#[derive(Default)]
pub(crate) struct Document {
    text: String,
    end: Option<Value>,
}

impl Serves<Completion> for Document {}

impl Reassemble<Step> for Document {
    fn absorb(&mut self, step: &Step) {
        match step {
            Step::Text(text) => self.text.push_str(text),
            Step::End {
                length,
                prompt_tokens,
                output_tokens,
            } => {
                self.end = Some(json!({
                    "length": length,
                    "prompt_tokens": prompt_tokens,
                    "output_tokens": output_tokens,
                }));
            }
        }
    }

    fn finish(self) -> Value {
        json!({ "text": self.text, "end": self.end })
    }
}

/// The transport: the queue of the thread that owns the model. The thread
/// ends when the last handle is dropped.
#[derive(Clone)]
pub(crate) struct Engine(mpsc::Sender<Job>);

impl Transport<Local> for Engine {
    fn send(&self, prompt: Prompt, _exchange: Exchange) -> Opening<Step> {
        let jobs = self.0.clone();
        Opening::new(async move {
            let (frames, replies) = tokio::sync::mpsc::unbounded_channel();
            jobs.send(Job {
                prompt: prompt.text,
                images: prompt.images,
                sampling: prompt.sampling,
                schema: prompt.schema,
                frames,
            })
            .map_err(|_| ProviderError::Provider("the local model stopped".to_owned()))?;
            // Dropping the stream closes the channel, which frees the
            // request's sequence at the engine's next step.
            Ok(Opened::new(futures::stream::unfold(
                replies,
                |mut replies| async move { replies.recv().await.map(|step| (step, replies)) },
            )))
        })
    }
}
