//! The agent: rig's agent loop over the tools the app's host gives it, on any
//! model that takes tools, keeping its conversation inside the model's window.

use std::{
    future::Future,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use anyhow::{Context as _, Result, anyhow};
use futures::StreamExt;
use indoc::indoc;
use rig_agent::{
    agent::{
        AgentBuilder, AgentHook, CompletionCallAction, CompletionCallEvent, HookContext,
        ModelTurnAction, ModelTurnFinished, MultiTurnStreamItem, RequestPatch,
    },
    completion::PromptError,
};
use rig_core::{
    catalog::ModelSpec,
    completion::{Document, Effort, GenerationOptions, Message, Usage},
    driver::DynModel,
    message::ToolName,
    operation::Completion,
    streaming::{Item, StreamEvent},
    tool::{DynamicTool, ToolErrorKind, ToolExecutionError, ToolOutput, validate::validate},
};
use rig_memory::{CompactReason, CompactionPolicy, estimate_tokens};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Context, KOHARU, context::spec};

const INSTRUCTIONS: &str = indoc! {"
    You are Koharu Agent, operating a manga translation project inside Koharu.
    The project's state when the request began is the JSON file with id project; tool results report changes since.
    Use the provided tools whenever the user asks to inspect or change the project. Never claim a change succeeded unless its tool result says it succeeded.
    All project changes are revisioned and reversible. Do not ask for permission. Continue using tools until the request is complete or cannot be completed.
    Do not invent identifiers. Preserve artwork and existing authored content unless the user asks to change them."};

/// A run stops after this many model calls; rig's default is one.
const MAX_TURNS: usize = 20;

/// How many times a run lets the model correct a call it wrote wrong.
const CORRECTIONS: usize = 2;

/// `[agent]`: the agent's model, as a `vendor/model` reference, and how much
/// it reasons.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct AgentConfig {
    pub model: Option<String>,
    pub reasoning: Option<Effort>,
}

impl koharu_config::Section for AgentConfig {
    const NAME: &'static str = "agent";
}

/// What the app gives the agent: the open project, and the tools that
/// read and edit it.
pub trait Host: Send + Sync + 'static {
    /// The project's state, sent with each prompt.
    fn context(&self) -> Value;

    fn tools(&self) -> Vec<Tool>;

    /// Runs `tool` with `arguments`, which match its schema. An error goes
    /// back to the model, marked as one.
    fn invoke(&self, tool: &str, arguments: Value) -> impl Future<Output = Invocation> + Send;
}

/// A tool, with the JSON schema of its arguments.
#[derive(Clone, Debug)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

pub type Invocation = Result<ToolOutput, String>;

/// What the agent panel shows while a run goes on.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Text { delta: String },
    Reasoning { delta: String },
    ToolStarted { tool: String, arguments: Value },
    ToolFinished { tool: String, error: Option<String> },
}

type Events = Arc<dyn Fn(Event) + Send + Sync>;

/// The agent's conversation about one open project, on one model.
pub struct Agent<H> {
    host: Arc<H>,
    model: DynModel<Completion>,
    context: Context,
    reasoning: Option<Effort>,
}

/// Why an attempt at a run failed.
enum Failure {
    /// The model refused a request as too long before any tool acted, so
    /// the run can start over once the conversation is shorter.
    Overflow(anyhow::Error),
    Other(anyhow::Error),
}

impl<H: Host> Agent<H> {
    pub fn new(host: Arc<H>, model: DynModel<Completion>, context: Context) -> Self {
        Self {
            host,
            model,
            context,
            reasoning: None,
        }
    }

    pub fn context(&self) -> &Context {
        &self.context
    }

    pub fn set_reasoning(&mut self, reasoning: Option<Effort>) {
        self.reasoning = reasoning;
    }

    /// Runs `prompt` until the model answers without tools, and answers with
    /// its last text. `events` hears the run as it goes. Dropping the future
    /// cancels the run; a tool already running finishes, as each edit is whole
    /// or not at all.
    pub async fn run(
        &mut self,
        prompt: &str,
        events: impl Fn(Event) + Send + Sync + 'static,
    ) -> Result<String> {
        self.context.fit(&self.model).await?;
        let events: Events = Arc::new(events);
        match self.attempt(prompt, &events).await {
            Err(Failure::Overflow(_)) => {
                self.context
                    .compact(&self.model, CompactReason::Overflow)
                    .await?;
                self.attempt(prompt, &events)
                    .await
                    .map_err(|failure| match failure {
                        Failure::Overflow(error) | Failure::Other(error) => error,
                    })
            }
            Err(Failure::Other(error)) => Err(error),
            Ok(answer) => Ok(answer),
        }
    }

    async fn attempt(&mut self, prompt: &str, events: &Events) -> Result<String, Failure> {
        let acted = Arc::new(AtomicBool::new(false));
        let spec = spec(&self.model).map_err(Failure::Other)?.clone();
        let tools = self
            .host
            .tools()
            .into_iter()
            .map(|tool| self.dynamic(tool, events, &acted))
            .collect::<Result<Vec<_>>>()
            .map_err(Failure::Other)?;
        let mut agent = AgentBuilder::new(self.model.clone())
            .preamble(INSTRUCTIONS)
            .dynamic_tools(tools)
            .add_hook(Window::new(self.context.policy(&spec), spec.clone()));
        if let Some(effort) = self.reasoning {
            agent = agent.options(GenerationOptions::default().reasoning(effort));
        }
        if spec.provider.vendor() == KOHARU {
            // The catalog's settings are for translation: a repeat penalty
            // would penalize repeated call markers.
            agent = agent.additional_params(json!({"repeat_penalty": 1.0}));
        }
        let history = self
            .context
            .messages()
            .iter()
            .map(|message| Message::clone(message));
        let mut stream = agent
            .build()
            .prompt(Message::user(prompt))
            .history(history)
            // rig sends the document ahead of the history with each of the
            // run's requests and keeps it out of the history, so the model
            // sees only the project's latest state.
            .document(Document {
                id: "project".to_owned(),
                text: self.host.context().to_string(),
                additional_props: Default::default(),
            })
            .max_turns(MAX_TURNS)
            .max_invalid_tool_call_retries(CORRECTIONS)
            .max_consecutive_malformed_tool_calls(CORRECTIONS)
            .stream();
        while let Some(item) = stream.next().await {
            let item = match item {
                Ok(item) => item,
                Err(error) if overflowed(&error) && !acted.load(Ordering::Relaxed) => {
                    return Err(Failure::Overflow(error.into()));
                }
                Err(error) => return Err(Failure::Other(error.into())),
            };
            match item {
                MultiTurnStreamItem::StreamAssistantItem(Item::Event(StreamEvent::Text {
                    text,
                    ..
                })) => events(Event::Text { delta: text }),
                MultiTurnStreamItem::StreamAssistantItem(Item::Event(StreamEvent::Reasoning {
                    text,
                    ..
                })) => events(Event::Reasoning { delta: text }),
                MultiTurnStreamItem::FinalResponse(response) => {
                    let usage = response
                        .completion_calls()
                        .last()
                        .map_or_else(Usage::default, |call| call.usage);
                    let answer = response.output();
                    self.context.record([(response.messages, usage)]);
                    return Ok(answer);
                }
                _ => {}
            }
        }
        Err(Failure::Other(anyhow!("the run ended without an answer")))
    }

    /// `tool` for rig: its arguments checked against its schema, as rig-agent
    /// doesn't and a local model has no grammar holding it to, then run on the
    /// host.
    fn dynamic(&self, tool: Tool, events: &Events, acted: &Arc<AtomicBool>) -> Result<DynamicTool> {
        let host = Arc::clone(&self.host);
        let events = Arc::clone(events);
        let acted = Arc::clone(acted);
        let name = tool.name.clone();
        let parameters = tool.parameters.clone();
        let tool_name = ToolName::new(tool.name).context("a tool has no name")?;
        Ok(DynamicTool::new(
            tool_name,
            tool.description,
            tool.parameters,
            move |arguments| {
                let host = Arc::clone(&host);
                let events = Arc::clone(&events);
                let acted = Arc::clone(&acted);
                let name = name.clone();
                let parameters = parameters.clone();
                Box::pin(async move {
                    events(Event::ToolStarted {
                        tool: name.clone(),
                        arguments: arguments.clone(),
                    });
                    let invocation = match refusal(&parameters, &arguments) {
                        Some(refusal) => Err(refusal),
                        None => {
                            acted.store(true, Ordering::Relaxed);
                            let tool = name.clone();
                            // Detached, so dropping the run doesn't cut an
                            // edit short.
                            tokio::spawn(async move { host.invoke(&tool, arguments).await })
                                .await
                                .unwrap_or_else(|error| Err(format!("the tool failed: {error}")))
                        }
                    };
                    events(Event::ToolFinished {
                        tool: name,
                        error: invocation.as_ref().err().cloned(),
                    });
                    invocation
                        .map_err(|message| ToolExecutionError::new(ToolErrorKind::Other, message))
                })
            },
        ))
    }
}

/// What the model is told when `arguments` don't fit `parameters`.
fn refusal(parameters: &Value, arguments: &Value) -> Option<String> {
    let problems = validate(parameters, arguments).err()?;
    let problems = problems
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ");
    Some(format!(
        "the arguments don't fit the tool's schema: {problems}"
    ))
}

fn overflowed(error: &PromptError) -> bool {
    match error {
        PromptError::Provider(error) => error.report().is_context_overflow(),
        PromptError::Report(report) => report.is_context_overflow(),
        _ => false,
    }
}

/// Keeps a run inside the model's window: over the threshold, a turn is sent
/// with older tool outputs cleared on a copy of its history. Clearing needs no
/// model call, and any earlier would drop outputs for no reason and invalidate
/// prompt caches.
struct Window {
    policy: CompactionPolicy,
    spec: ModelSpec,
    /// How long the history was at the last model call.
    asked: AtomicUsize,
    /// The tokens the last turn reported, and how many messages they cover.
    count: Mutex<Option<(u64, usize)>>,
}

impl Window {
    fn new(policy: CompactionPolicy, spec: ModelSpec) -> Self {
        Self {
            policy,
            spec,
            asked: AtomicUsize::new(0),
            count: Mutex::new(None),
        }
    }
}

impl AgentHook for Window {
    async fn on_completion_call(
        &self,
        _ctx: &HookContext,
        event: CompletionCallEvent<'_>,
    ) -> CompletionCallAction {
        let history = event.history;
        self.asked.store(history.len(), Ordering::Relaxed);
        let count = *self.count.lock().unwrap_or_else(PoisonError::into_inner);
        let earlier = match count {
            Some((tokens, through)) => {
                tokens.saturating_add(estimate_tokens(history.get(through..).unwrap_or_default()))
            }
            None => estimate_tokens(history),
        };
        let tokens = earlier.saturating_add(estimate_tokens(std::slice::from_ref(event.prompt)));
        if !self.policy.over_threshold(tokens, &self.spec) {
            return CompletionCallAction::continue_run();
        }
        let mut history = history.to_vec();
        self.policy.clearing.clear(&mut history);
        let mut patch = RequestPatch::new();
        patch.history = Some(history);
        CompletionCallAction::patch(patch)
    }

    async fn on_model_turn_finished(
        &self,
        _ctx: &HookContext,
        event: ModelTurnFinished<'_>,
    ) -> ModelTurnAction {
        // It covers the history, the prompt, and the reply.
        let through = self.asked.load(Ordering::Relaxed) + 2;
        *self.count.lock().unwrap_or_else(PoisonError::into_inner) =
            event.usage.context_tokens().map(|tokens| (tokens, through));
        ModelTurnAction::Continue
    }
}

#[cfg(test)]
mod tests {
    use indoc::formatdoc;
    use rig_core::error::{ErrorReport, ProviderError};

    use super::*;
    use crate::local::{
        markup::FUNCTION_TAGS,
        scripted::{Scripted, scripted},
    };

    #[derive(Default)]
    struct Pages {
        invoked: Mutex<Vec<Value>>,
    }

    impl Host for Pages {
        fn context(&self) -> Value {
            json!({"title": "The project"})
        }

        fn tools(&self) -> Vec<Tool> {
            vec![Tool {
                name: "get_page".to_owned(),
                description: "Reads a page.".to_owned(),
                parameters: json!({
                    "type": "object",
                    "properties": {"page": {"type": "integer"}},
                    "required": ["page"],
                }),
            }]
        }

        async fn invoke(&self, _tool: &str, arguments: Value) -> Invocation {
            self.invoked.lock().unwrap().push(arguments);
            Ok(ToolOutput::text("Page 3 says hello."))
        }
    }

    fn call(page: &str) -> String {
        formatdoc! {"
            <tool_call>
            <function=get_page>
            <parameter=page>
            {page}
            </parameter>
            </function>
            </tool_call>"
        }
    }

    fn leak(text: String) -> &'static str {
        Box::leak(text.into_boxed_str())
    }

    #[tokio::test]
    async fn a_local_model_runs_the_tools() {
        let Scripted { model, prompts } = scripted(
            Some(&FUNCTION_TAGS),
            16_384,
            vec![Ok(leak(call("3"))), Ok("Page 3 says hello to you.")],
        );
        let host = Arc::new(Pages::default());
        let mut agent = Agent::new(Arc::clone(&host), model, Context::default());
        let events = Arc::new(Mutex::new(Vec::new()));
        let heard = Arc::clone(&events);
        let answer = agent
            .run("What does page 3 say?", move |event| {
                heard.lock().unwrap().push(event);
            })
            .await
            .unwrap();
        assert_eq!(answer, "Page 3 says hello to you.");
        assert_eq!(*host.invoked.lock().unwrap(), [json!({"page": 3})]);
        let prompts = prompts.lock().unwrap();
        assert_eq!(prompts.len(), 2);
        // The project goes with the request, and the tool's result with the
        // second.
        assert!(prompts[0].contains("The project"));
        assert!(prompts[1].contains("<tool>Page 3 says hello.</tool>"));
        // The conversation lives on.
        assert!(agent.context().messages().len() >= 4);
        assert!(matches!(
            events.lock().unwrap().first(),
            Some(Event::ToolStarted { tool, .. }) if tool == "get_page"
        ));
    }

    /// A local model has no grammar holding it to the schema, so the check
    /// sends it back rather than reaching the host.
    #[tokio::test]
    async fn arguments_that_do_not_fit_go_back_to_the_model() {
        let Scripted { model, prompts } = scripted(
            Some(&FUNCTION_TAGS),
            16_384,
            vec![Ok(leak(call("three"))), Ok(leak(call("3"))), Ok("Done.")],
        );
        let host = Arc::new(Pages::default());
        let mut agent = Agent::new(Arc::clone(&host), model, Context::default());
        let answer = agent.run("Read page 3.", |_| {}).await.unwrap();
        assert_eq!(answer, "Done.");
        assert_eq!(*host.invoked.lock().unwrap(), [json!({"page": 3})]);
        assert!(prompts.lock().unwrap()[1].contains("schema"));
    }

    #[tokio::test]
    async fn a_refused_request_starts_over_after_compacting() {
        let overflow = ProviderError::Relayed(Box::new(ErrorReport::context_overflow(
            "the prompt is too long",
        )));
        let Scripted { model, prompts } = scripted(
            Some(&FUNCTION_TAGS),
            400_000,
            vec![Err(overflow), Ok("The story so far."), Ok("Done.")],
        );
        let mut context = Context::default();
        let earlier = (0..4)
            .flat_map(|_| [Message::user("Page."), Message::assistant("Answer.")])
            .collect();
        context.record([(earlier, Usage::default())]);
        let host = Arc::new(Pages::default());
        let mut agent = Agent::new(host, model, context);
        let answer = agent.run("Go on.", |_| {}).await.unwrap();
        assert_eq!(answer, "Done.");
        assert_eq!(prompts.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn a_model_without_call_markup_fails_the_run() {
        let Scripted { model, .. } = scripted(None, 16_384, Vec::new());
        let mut agent = Agent::new(Arc::new(Pages::default()), model, Context::default());
        assert!(agent.run("Read page 3.", |_| {}).await.is_err());
    }
}
