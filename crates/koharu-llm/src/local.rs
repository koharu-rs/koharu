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

/// How many requests a loaded model decodes together, and the most tokens
/// each may hold, prompt and answer together. The KV cache for all of them is
/// allocated when the model loads, which may give each request less if the
/// device has too little free memory; the model's spec says what it got.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Size {
    pub sequences: u32,
    pub context: u32,
}

impl Size {
    /// What the agent asks of a local model: one conversation at a time.
    pub const AGENT: Self = Self {
        sequences: 1,
        context: 65_536,
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

/// What to load, as the catalog or a caller names it.
pub(crate) struct Loading {
    pub(crate) id: String,
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
            id,
            model,
            projector,
            defaults,
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
            .name(format!("llm {id}"))
            .spawn(move || engine::serve(files, size, ready, queue))?;
        let loaded = loaded.recv().context("the model's thread stopped")??;
        let Value::Object(defaults) = serde_json::to_value(defaults)? else {
            unreachable!("sampling settings serialize to an object")
        };
        let facts = ModelFacts::new(spec(&id, markup, loaded.vision, loaded.context)?);
        let wire = Local {
            id,
            template: Arc::new(loaded.template),
            marker: koharu_llama::mtmd::mtmd_default_marker().to_owned(),
            vision: loaded.vision,
            defaults,
            markup,
            facts,
        };
        Ok(Model::new(wire, Engine(jobs)).erase())
    })
    .await?
}

/// What rig and a conversation's context know of a loaded model.
fn spec(
    id: &str,
    markup: Option<&'static Markup>,
    vision: bool,
    context: u32,
) -> Result<ModelSpec> {
    let mut spec = ModelSpec::new(ProviderId::declare("local")?, id)
        .with_context_window(context)
        .with_tools(markup.is_some_and(|markup| markup.call.is_some()));
    if markup.is_some_and(|markup| markup.reasoning.is_some()) {
        spec = spec.with_reasoning(ReasoningSupport::Unknown { default: None });
    }
    spec.input.image = vision;
    spec.structured_output = true;
    Ok(spec)
}

/// The wire: the model's chat template, and what it reads and writes.
#[derive(Clone, Debug)]
pub(crate) struct Local {
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
    parser: Parser,
}

impl Wire for Local {
    type Op = Completion;
    type Payload = Prompt;
    type Frame = Part;
    type Event = Part;
    type Decoder<'id> = Reply<'id>;
    type Reassembler = Document;

    fn describe(&self) -> Descriptor<'_> {
        Descriptor::new("local")
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
        if request.output_schema.is_some() && (thinking || !tools.is_empty()) {
            // The grammar holds the reply from its first token.
            return Err(EncodeError::request(
                "a local model can't keep to a schema while it reasons or calls tools",
            ));
        }
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
        let parser = match self.markup.filter(|_| request.output_schema.is_none()) {
            Some(markup) => Parser::new(markup, tools, &text),
            None => Parser::plain(),
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

impl Local {
    /// The tools the request offers, which only a model with call markup
    /// takes, and which it can't be forced to call without a grammar.
    fn tools(&self, request: &CompletionRequest) -> Result<Vec<Arc<ToolDefinition>>, String> {
        if request.tools.is_empty() {
            return Ok(Vec::new());
        }
        if !self.markup.is_some_and(|markup| markup.call.is_some()) {
            return Err("this local model takes no tools".to_owned());
        }
        match &request.tool_choice {
            None | Some(ToolChoice::Auto) => Ok(request.tools.clone()),
            Some(ToolChoice::None) => Ok(Vec::new()),
            Some(_) => Err("a local model can't be made to call a tool".to_owned()),
        }
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
            tools: self.markup.is_some_and(|markup| markup.call.is_some()),
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
        let reasons = self.markup.is_some_and(|markup| markup.reasoning.is_some());
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

impl Transport<Local> for Engine {
    fn send(&self, prompt: Prompt, _exchange: Exchange) -> Opening<Part> {
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
        let reply = script.model.completion.call(with_tools()).await.unwrap();
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
        let reply = script.model.completion.call(with_tools()).await.unwrap();
        let call = reply.choice.iter().find_map(|block| match block {
            AssistantContent::ToolCall(call) => Some(call),
            _ => None,
        });
        assert!(call.unwrap().function.invalid_arguments.is_some());
    }

    #[tokio::test]
    async fn a_model_without_call_markup_refuses_tools() {
        let script = scripted(None, 4096, Vec::new());
        assert!(script.model.completion.call(with_tools()).await.is_err());
    }

    #[tokio::test]
    async fn a_schema_can_not_go_with_tools() {
        let script = scripted(Some(&FUNCTION_TAGS), 4096, Vec::new());
        let schema = schemars::Schema::try_from(json!({"type": "object"})).unwrap();
        let request = with_tools().output_schema(schema);
        assert!(script.model.completion.call(request).await.is_err());
    }

    /// The grammar holds such a reply to JSON, where `<think>` is only text.
    #[tokio::test]
    async fn a_reply_to_a_schema_is_not_parsed() {
        let answer = "{\"text\": \"<think>a</think>\"}";
        let script = scripted(Some(&FUNCTION_TAGS), 4096, vec![Ok(answer)]);
        let schema = schemars::Schema::try_from(json!({"type": "object"})).unwrap();
        let request = CompletionRequest::new(Message::user("Hi")).output_schema(schema);
        let reply = script.model.completion.call(request).await.unwrap();
        assert_eq!(reply.text(), answer);
    }

    #[test]
    fn the_spec_says_what_the_model_can_do() {
        let spec = spec("m", Some(&FUNCTION_TAGS), true, 8192).unwrap();
        assert_eq!(spec.context_window, Some(8192));
        assert!(spec.tools && spec.input.image && spec.reasoning.supported());
        let spec = spec_without_markup();
        assert!(!spec.tools && !spec.reasoning.supported());
    }

    fn spec_without_markup() -> ModelSpec {
        spec("m", None, false, 8192).unwrap()
    }
}
