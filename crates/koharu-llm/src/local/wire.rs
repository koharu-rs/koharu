//! The wire: a rig request as the prompt the model's chat template renders,
//! with its sampling settings and grammar, and the reply's parts as rig's
//! blocks.

use std::{marker::PhantomData, sync::Arc};

use rig_core::{
    catalog::{ModelFacts, ModelSpec},
    completion::{
        Accepts, CompletionRequest, Effort, FinishReason, ReplayTarget, ToolDefinition, Usage,
        message::{Api, ToolChoice},
        options::{FinalBody, Mapping, OptionFields, OptionMap, RawAt, Reasoning, request_params},
    },
    error::{EncodeError, ProviderError},
    message::{CallId, LocalCallId, ToolName},
    operation::{Block, Completion, Finish},
    wire::{
        Decoder, Descriptor, Flow, Mode, Out, Wire, WireEvent,
        document::{Reassemble, Serves},
    },
};
use serde_json::{Map, Value, json};

use super::{
    Sampling, conversation,
    markup::{Call, Item, Markup, Parser},
    template::{Rendering, Template},
};
use crate::KOHARU;

/// The wire: the model's chat template, and what it reads and writes.
#[derive(Clone, Debug)]
pub(crate) struct Gguf {
    pub(super) id: String,
    pub(super) template: Arc<Template>,
    /// Where an image goes in a prompt.
    pub(super) marker: String,
    pub(super) vision: bool,
    /// The sampling settings a request starts from.
    pub(super) defaults: Map<String, Value>,
    pub(super) markup: Option<&'static Markup>,
    pub(super) facts: ModelFacts,
}

/// What the engine runs for one request.
pub(crate) struct Prompt {
    pub(super) text: String,
    pub(super) images: Vec<Vec<u8>>,
    pub(super) sampling: Sampling,
    pub(super) schema: Option<String>,
    /// The markers of the reasoning the reply may write before the schema
    /// holds it.
    pub(super) answer_after: Option<(&'static str, &'static str)>,
    pub(super) parser: Parser,
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

    fn encode(&self, request: CompletionRequest, _mode: Mode) -> Result<Prompt, EncodeError> {
        let tools = self.tools(&request).map_err(EncodeError::request)?;
        let sampling = self.sampling(&request)?;
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
        let schema = request.output_schema.map(grammar).transpose()?;
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
            open: None,
            id: PhantomData,
        }
    }
}

impl Gguf {
    /// The request's sampling settings. Below its options and
    /// `additional_params`: the model's defaults, under the request's
    /// temperature and token limit.
    #[expect(clippy::result_large_err, reason = "rig's encode error is rig's")]
    fn sampling(&self, request: &CompletionRequest) -> Result<Sampling, EncodeError> {
        let mut base = self.defaults.clone();
        if let Some(temperature) = request.temperature {
            base.insert("temperature".to_owned(), json!(temperature));
        }
        if let Some(max_tokens) = request.max_tokens {
            base.insert("max_tokens".to_owned(), json!(max_tokens));
        }
        let body: FinalBody = request_params(self, request, |_| Ok(base), RawAt::Top, &[])?;
        let sampling: Sampling = body.deserialize().map_err(EncodeError::request)?;
        sampling.check().map_err(EncodeError::request)?;
        Ok(sampling)
    }

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

/// `schema` as the grammar llguidance compiles, with the compact separators
/// in place of its default, which allows any whitespace between tokens.
fn grammar(schema: schemars::Schema) -> Result<String, serde_json::Error> {
    let mut schema = schema.to_value();
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
        /// Whether the call closed before the reply ended.
        finished: bool,
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
            Item::Call(Call { name, arguments }) => Self::Call {
                name,
                arguments,
                finished: true,
            },
            Item::Unfinished(Call { name, arguments }) => Self::Call {
                name,
                arguments,
                finished: false,
            },
        }
    }
}

/// The decoder: a reply's parts as rig's blocks, then the end.
pub(crate) struct Reply<'id> {
    called: bool,
    /// The call the reply ended in the middle of.
    open: Option<usize>,
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
            Part::Call {
                name,
                arguments,
                finished,
            } => {
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
                out.open(index, block, Value::Null)?;
                out.push(index, &arguments)?;
                if finished {
                    out.finish(index)?;
                } else {
                    self.open = Some(index);
                }
                self.called = true;
                Ok(Flow::More)
            }
            Part::End {
                length,
                prompt_tokens,
                output_tokens,
            } => {
                out.end_run()?;
                // A call the token limit cut stays open, so rig doesn't run it.
                // One the model left unclosed itself is as it meant it.
                if let Some(index) = self.open.take()
                    && !length
                {
                    out.finish(index)?;
                }
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
            Part::Call {
                name, arguments, ..
            } => {
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

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use rig_core::{
        completion::{AssistantContent, Message},
        message::StopReason,
    };
    use serde_json::json;

    use super::*;
    use crate::local::{markup::FUNCTION_TAGS, scripted::scripted};

    fn tool() -> Arc<ToolDefinition> {
        Arc::new(ToolDefinition {
            name: ToolName::new("get_page").expect("`get_page` names a tool"),
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

    /// A model can end its reply without closing its call, and rig fails a
    /// reply with a call left open that its token limit didn't cut.
    #[tokio::test]
    async fn a_call_the_model_leaves_unclosed_is_a_call() {
        let script = scripted(
            Some(&FUNCTION_TAGS),
            4096,
            vec![Ok(
                "<tool_call><function=get_page><parameter=page>3</parameter></function>",
            )],
        );
        let reply = script.model.call(with_tools()).await.unwrap();
        assert!(matches!(reply.stop(), StopReason::ToolUse));
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
}
