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

use std::{
    collections::VecDeque,
    marker::PhantomData,
    path::PathBuf,
    sync::{Arc, mpsc},
};

use anyhow::{Context, Result, ensure};
use rig_core::{
    catalog::{ModelFacts, ModelSpec, ReasoningSupport},
    completion::{
        Accepts, CompletionRequest, Effort, FinishReason, ReplayTarget, ToolDefinition, Usage,
        message::{Api, ToolChoice},
        options::{FinalBody, Mapping, OptionFields, OptionMap, RawAt, Reasoning, request_params},
    },
    driver::{DynModel, Exchange, Model, Opened, Opening, Transport},
    error::{EncodeError, ProviderError},
    message::{CallId, LocalCallId, ToolName},
    operation::{Block, Completion, Finish},
    providers::registry::ProviderId,
    wire::{
        Decoder, Descriptor, Flow, Mode, Out, Wire, WireEvent,
        document::{Reassemble, Serves},
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use engine::{Files, Job, Step};
use markup::{Item, Markup, Parser};
use template::{Rendering, Template};

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
        Ok(Model::new(wire, Engine(jobs)).erase())
    })
    .await?
}

/// The wire: the model's chat template, and what it reads and writes.
#[derive(Clone, Debug)]
pub(crate) struct Gguf {
    id: String,
    template: Arc<Template>,
    /// Where an image goes in a prompt.
    marker: String,
    vision: bool,
    /// The sampling settings a request starts from.
    defaults: Map<String, Value>,
    markup: Option<&'static Markup>,
    facts: ModelFacts,
}

/// What the engine runs for one request.
pub(crate) struct Prompt {
    text: String,
    images: Vec<Vec<u8>>,
    sampling: Sampling,
    schema: Option<String>,
    /// The marker after which the schema holds the reply, which reasons
    /// freely before it.
    answer_after: Option<&'static str>,
    parser: Parser,
}

impl Wire for Gguf {
    type Op = Completion;
    type Payload = Prompt;
    type Frame = Part;
    type Event = Part;
    type Decoder<'id> = Reply<'id>;
    type Reassembler = Document;

    fn describe(&self) -> Descriptor<'_> {
        Descriptor::new(KOHARU)
            .model(self.id.as_str())
            .replay(self)
            .facts(&self.facts)
    }

    #[expect(clippy::result_large_err, reason = "rig's encode error is rig's")]
    fn encode(&self, request: CompletionRequest, _mode: Mode) -> Result<Prompt, EncodeError> {
        let tools = self.tools(&request).map_err(EncodeError::request)?;
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

        let thinking = sampling.enable_thinking.unwrap_or(false);
        if request.output_schema.is_some() && !tools.is_empty() {
            return Err(EncodeError::request(
                "a local model can't keep to a schema while it calls tools",
            ));
        }
        // A reply that reasons first keeps to the schema once its reasoning
        // closes.
        let answer_after = match (&request.output_schema, thinking) {
            (Some(_), true) => Some(
                self.markup
                    .and_then(|markup| markup.reasoning)
                    .map(|(_, close)| close)
                    .ok_or_else(|| EncodeError::request("this local model doesn't reason"))?,
            ),
            _ => None,
        };
        let conversation = conversation::conversation(&request.chat_history, &self.marker)
            .map_err(EncodeError::request)?;
        let text = self
            .template
            .render(&Rendering {
                messages: &conversation.messages,
                tools: &tools
                    .iter()
                    .map(|tool| declaration(tool))
                    .collect::<Vec<_>>(),
                thinking,
                effort: sampling.reasoning_effort.map(effort_name),
            })
            .map_err(|error| EncodeError::request(format!("{error:#}")))?;
        let parser = match (self.markup, request.output_schema.is_some()) {
            (Some(markup), false) => Parser::new(markup, tools, &text),
            (Some(markup), true) if answer_after.is_some() => Parser::answer(markup, &text),
            _ => Parser::plain(),
        };
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
            images: conversation.images,
            sampling,
            schema,
            answer_after,
            parser,
        })
    }

    fn decoder<'id>(&self) -> Reply<'id> {
        Reply {
            called: false,
            id: PhantomData,
        }
    }
}

impl Gguf {
    /// The tools the request offers, which only a model with call markup
    /// takes, and which it can't be forced to call without a grammar.
    fn tools(&self, request: &CompletionRequest) -> Result<Vec<Arc<ToolDefinition>>, String> {
        if request.tools.is_empty() {
            return Ok(Vec::new());
        }
        if !self.spec().tools {
            return Err("this local model takes no tools".to_owned());
        }
        match &request.tool_choice {
            None | Some(ToolChoice::Auto) => Ok(request.tools.clone()),
            Some(ToolChoice::None) => Ok(Vec::new()),
            Some(_) => Err("a local model can't be made to call a tool".to_owned()),
        }
    }

    /// What the model can do, which its spec says.
    fn spec(&self) -> &ModelSpec {
        self.facts
            .spec()
            .expect("a local model's facts hold its spec")
    }
}

/// A tool as chat templates read it.
fn declaration(tool: &ToolDefinition) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name.as_str(),
            "description": tool.description,
            "parameters": tool.parameters,
        },
    })
}

impl ReplayTarget for Gguf {
    fn api(&self) -> Api {
        Api::from_static("koharu.local")
    }

    fn provider(&self) -> &str {
        KOHARU
    }

    fn model(&self) -> &str {
        &self.id
    }

    fn accepts(&self, _model: &str) -> Accepts {
        Accepts {
            user_images: self.vision,
            assistant_images: false,
            tool_result_images: false,
            tools: self.spec().tools,
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
        let reasons = self.spec().reasoning.supported();
        OptionMap {
            reasoning: Mapping::of(reasoning, |reasoning| match reasoning {
                Reasoning::Off => Mapping::Send(json!({"enable_thinking": false})),
                Reasoning::Effort(effort) if reasons => Mapping::Send(json!({
                    "enable_thinking": true,
                    "reasoning_effort": effort,
                })),
                Reasoning::Effort(_) => Mapping::unsupported("this local model doesn't reason"),
                _ => Mapping::unsupported("a local model takes a reasoning effort, not a budget"),
            }),
            cache: Mapping::of(cache, |_| {
                Mapping::unsupported("the local engine shares prompts on its own")
            }),
            service_tier: Mapping::of(service_tier, |_| Mapping::unsupported(NONE)),
            verbosity: Mapping::of(verbosity, |_| Mapping::unsupported(NONE)),
            parallel_tool_calls: Mapping::of(parallel_tool_calls, |_| {
                Mapping::unsupported("a local model decides how many tools it calls")
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

/// The names Qwen 3.8's template reads; the others ignore `reasoning_effort`.
fn effort_name(effort: Effort) -> &'static str {
    match effort {
        Effort::Minimal | Effort::Low => "low",
        Effort::Medium => "medium",
        _ => "xhigh",
    }
}

/// One piece of a reply, on its way from the engine to rig.
#[derive(Clone, Debug)]
pub(crate) enum Part {
    Text(String),
    Reasoning(String),
    Call {
        name: String,
        arguments: String,
    },
    /// The reply ended: at an end-of-generation token, or at its token limit
    /// or the end of its sequence's context.
    End {
        length: bool,
        prompt_tokens: u64,
        output_tokens: u64,
    },
}

impl From<Item> for Part {
    fn from(item: Item) -> Self {
        match item {
            Item::Text(text) => Self::Text(text),
            Item::Reasoning(text) => Self::Reasoning(text),
            Item::Call(call) => Self::Call {
                name: call.name,
                arguments: call.arguments,
            },
        }
    }
}

/// The decoder: a reply's parts as rig's blocks, then the end.
pub(crate) struct Reply<'id> {
    called: bool,
    id: PhantomData<fn(&'id ()) -> &'id ()>,
}

impl<'id> Decoder<'id, Completion, Part> for Reply<'id> {
    type Event = Part;

    fn classify(&self, frame: Part) -> WireEvent<Part> {
        WireEvent::Known(frame)
    }

    fn decode(&mut self, part: Part, mut out: Out<'id, Completion>) -> Result<Flow, ProviderError> {
        match part {
            Part::Text(text) => {
                out.run(Block::Text, &text)?;
                Ok(Flow::More)
            }
            Part::Reasoning(text) => {
                out.run(Block::Reasoning { redacted: false }, &text)?;
                Ok(Flow::More)
            }
            Part::Call { name, arguments } => {
                out.end_run()?;
                let block = Block::Call {
                    // These models write no ids.
                    id: CallId::Local(LocalCallId::new()),
                    name: ToolName::new(&name).map_err(|error| {
                        ProviderError::Provider(format!("the model named a bad tool: {error}"))
                    })?,
                };
                let index = out.fresh_index();
                // rig reads arguments that aren't JSON as a malformed call and
                // answers it, so they don't end the reply.
                out.whole(index, block, Value::Null, &arguments)?;
                self.called = true;
                Ok(Flow::More)
            }
            Part::End {
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
                    } else if self.called {
                        FinishReason::ToolCalls
                    } else {
                        FinishReason::Stop
                    }),
                    ..Finish::default()
                }))
            }
        }
    }
}

/// The reply as one document: what it said, and how it ended.
#[derive(Default)]
pub(crate) struct Document {
    text: String,
    reasoning: String,
    calls: Vec<Value>,
    end: Option<Value>,
}

impl Serves<Completion> for Document {}

impl Reassemble<Part> for Document {
    fn absorb(&mut self, part: &Part) {
        match part {
            Part::Text(text) => self.text.push_str(text),
            Part::Reasoning(text) => self.reasoning.push_str(text),
            Part::Call { name, arguments } => {
                self.calls
                    .push(json!({"name": name, "arguments": arguments}));
            }
            Part::End {
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
        json!({
            "text": self.text,
            "reasoning": self.reasoning,
            "calls": self.calls,
            "end": self.end,
        })
    }
}

/// The transport: the queue of the thread that owns the model, and the reply's
/// markup read as it comes back. The thread ends when the last handle is
/// dropped.
#[derive(Clone)]
pub(crate) struct Engine(mpsc::Sender<Job>);

impl Transport<Gguf> for Engine {
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
    use indoc::indoc;
    use rig_core::{
        completion::{AssistantContent, Message},
        message::ToolName,
    };
    use serde_json::json;

    use super::{markup::FUNCTION_TAGS, scripted::scripted, *};

    fn tool() -> Arc<ToolDefinition> {
        Arc::new(ToolDefinition {
            name: ToolName::new("get_page").unwrap(),
            description: "Reads a page.".to_owned(),
            parameters: json!({"type": "object", "properties": {"page": {"type": "integer"}}}),
            annotations: Default::default(),
        })
    }

    fn with_tools() -> CompletionRequest {
        let mut request = CompletionRequest::new("Read page 3.");
        request.tools = vec![tool()];
        request
    }

    /// Through rig's decoding: reasoning, text, and a call come back as rig's
    /// own blocks, and the tool list reaches the template.
    #[tokio::test]
    async fn a_reply_reads_as_blocks() {
        let script = scripted(
            Some(&FUNCTION_TAGS),
            4096,
            vec![Ok(indoc! {"
                <think>Reading.</think>On it.<tool_call>
                <function=get_page>
                <parameter=page>
                3
                </parameter>
                </function>
                </tool_call>"
            })],
        );
        let reply = script.model.call(with_tools()).await.unwrap();
        let blocks = reply.choice.iter().collect::<Vec<_>>();
        assert!(
            matches!(blocks[0], AssistantContent::Reasoning(block) if block.text == "Reading.")
        );
        assert!(matches!(blocks[1], AssistantContent::Text(block) if block.text == "On it."));
        let AssistantContent::ToolCall(call) = blocks[2] else {
            panic!("a call, not {:?}", blocks[2]);
        };
        assert_eq!(call.function.name.as_str(), "get_page");
        assert_eq!(
            serde_json::Value::Object(call.function.arguments.clone()),
            json!({"page": 3})
        );
        assert!(script.prompts.lock().unwrap()[0].contains("get_page"));
    }

    /// Arguments that aren't JSON stay a call for rig to answer, and don't end
    /// the reply.
    #[tokio::test]
    async fn a_malformed_call_does_not_fail_the_reply() {
        let script = scripted(
            Some(&FUNCTION_TAGS),
            4096,
            vec![Ok(
                "<tool_call><function=get_page><parameter=page>3</tool_call>",
            )],
        );
        let reply = script.model.call(with_tools()).await.unwrap();
        let call = reply.choice.iter().find_map(|block| match block {
            AssistantContent::ToolCall(call) => Some(call),
            _ => None,
        });
        assert!(call.unwrap().function.invalid_arguments.is_some());
    }

    #[tokio::test]
    async fn a_model_without_call_markup_refuses_tools() {
        let script = scripted(None, 4096, Vec::new());
        assert!(script.model.call(with_tools()).await.is_err());
    }

    #[tokio::test]
    async fn a_schema_can_not_go_with_tools() {
        let script = scripted(Some(&FUNCTION_TAGS), 4096, Vec::new());
        let schema = schemars::Schema::try_from(json!({"type": "object"})).unwrap();
        let request = with_tools().output_schema(schema);
        assert!(script.model.call(request).await.is_err());
    }

    /// The grammar holds such a reply to JSON, where `<think>` is only text.
    #[tokio::test]
    async fn a_reply_to_a_schema_is_not_parsed() {
        let answer = "{\"text\": \"<think>a</think>\"}";
        let script = scripted(Some(&FUNCTION_TAGS), 4096, vec![Ok(answer)]);
        let schema = schemars::Schema::try_from(json!({"type": "object"})).unwrap();
        let request = CompletionRequest::new(Message::user("Hi")).output_schema(schema);
        let reply = script.model.call(request).await.unwrap();
        assert_eq!(reply.text(), answer);
    }

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
