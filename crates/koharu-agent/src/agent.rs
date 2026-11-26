//! The agent: rig's agent loop over Koharu's tools on the open project, on any
//! model that takes tools, keeping its conversation inside the model's window.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Result, anyhow};
use futures::StreamExt;
use indoc::indoc;
use rig_agent::{
    agent::{
        AgentBuilder, AgentHook, CompletionCallAction, CompletionCallEvent, DispatchAction,
        DispatchEvent, HookContext, InvalidToolCallAction, InvalidToolCallContext,
        InvalidToolCallReason, MultiTurnStreamItem, OutcomeAction, OutcomeEvent, RequestPatch,
    },
    completion::PromptError,
    tool::server::ToolServer,
};
use rig_core::{
    completion::{Document, GenerationOptions, Message, Reasoning, Usage},
    driver::DynModel,
    effect::{Answered, EffectKind},
    operation::Completion,
    streaming::{Item, StreamEvent},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use koharu_llm::{Context, KOHARU};

use crate::{Tools, tools::refusal};

/// The system prompt: Koharu's own, not a setting, since it describes the
/// tools and the project document, which Koharu owns.
const PROMPT: &str = indoc! {"
    You are Koharu Agent, operating a manga translation project inside Koharu.
    The project's state when the request began is the JSON file with id project; tool results report changes since.
    Use the provided tools whenever the user asks to inspect or change the project. Never claim a change succeeded unless its tool result says it succeeded.
    All project changes are revisioned and reversible. Do not ask for permission. Continue using tools until the request is complete or cannot be completed.
    Do not invent identifiers. Preserve artwork and existing authored content unless the user asks to change them."};

/// `[agent]`: the agent's model and how much it reasons.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
#[derive(specta::Type)]
pub struct AgentConfig {
    /// A `vendor/model` reference, as koharu-llm connects one.
    pub model: Option<String>,
    /// How much the model reasons before it answers: off, or one of rig's
    /// portable levels after OpenAI's `reasoning_effort`, written as one word;
    /// the model's own default when unset. rig refuses off for a model that
    /// can't stop reasoning.
    #[serde(with = "koharu_llm::reasoning")]
    #[specta(type = Option<String>)]
    pub reasoning: Option<Reasoning>,
}

impl koharu_config::Section for AgentConfig {
    const NAME: &'static str = "agent";
}

/// What the agent panel shows while a run goes on.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[derive(specta::Type)]
pub enum Event {
    Text { delta: String },
    Reasoning { delta: String },
    ToolStarted { tool: String, arguments: Value },
    ToolFinished { tool: String, error: Option<String> },
}

type Events = Arc<dyn Fn(Event) + Send + Sync>;

/// The agent's conversation about one open project, on one model.
pub struct Agent {
    tools: Tools,
    model: DynModel<Completion>,
    context: Context,
    config: AgentConfig,
}

/// Why an attempt at a run failed.
enum Failure {
    /// The model refused a request as too long before any tool acted, so
    /// the run can start over once the conversation is shorter.
    Overflow(anyhow::Error),
    Other(anyhow::Error),
}

impl Agent {
    pub fn new(tools: Tools, model: DynModel<Completion>, context: Context) -> Self {
        Self {
            tools,
            model,
            context,
            config: AgentConfig::default(),
        }
    }

    pub fn context(&self) -> &Context {
        &self.context
    }

    pub fn set_config(&mut self, config: AgentConfig) {
        self.config = config;
    }

    /// Runs `prompt` until the model answers without tools, and answers with
    /// its last text. `events` hears the run as it goes. Dropping the future
    /// cancels the run; an edit already applying finishes on its own thread,
    /// whole.
    pub async fn run(
        &mut self,
        prompt: &str,
        events: impl Fn(Event) + Send + Sync + 'static,
    ) -> Result<String> {
        let events: Events = Arc::new(events);
        match self.attempt(prompt, &events).await {
            Err(Failure::Overflow(_)) => {
                self.context.compact(&self.model).await?;
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
        let earlier = self.context.messages().len();
        let run = Arc::new(Mutex::new(Run {
            context: self.context.clone(),
            seen: earlier,
            usage: Usage::default(),
        }));
        let toolset = Tools::toolset();
        let calls = Calls {
            schemas: toolset
                .tool_definitions()
                .iter()
                .map(|tool| (tool.name.as_str().to_owned(), tool.parameters.clone()))
                .collect(),
            events: Arc::clone(events),
            acted: Arc::clone(&acted),
        };
        let tools = ToolServer::new().run();
        tools.add_tools(toolset);
        let mut agent = AgentBuilder::new(self.model.clone())
            .preamble(PROMPT)
            .tool_server_handle(tools)
            .add_hook(Window {
                model: self.model.clone(),
                run: Arc::clone(&run),
            })
            .add_hook(calls);
        if let Some(reasoning) = self.config.reasoning {
            agent = agent.options(GenerationOptions::default().reasoning(reasoning));
        }
        if self
            .model
            .spec()
            .is_some_and(|spec| spec.provider.vendor() == KOHARU)
        {
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
                text: self.tools.context().map_err(Failure::Other)?.to_string(),
                additional_props: Default::default(),
            })
            .tool_context(self.tools.tool_context())
            // rig always has a limit, and its default of one ends a run at its
            // first tool call.
            .max_turns(usize::MAX)
            .max_invalid_tool_call_retries(2)
            .max_consecutive_malformed_tool_calls(2)
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
                // Every call tells its usage here, one rig asks again for
                // included, which finishes no turn.
                MultiTurnStreamItem::CompletionCall(call) => run.lock().await.usage = call.usage,
                MultiTurnStreamItem::FinalResponse(response) => {
                    let answer = response.output();
                    let mut run = run.lock().await;
                    let seen = run.seen - earlier;
                    run.record(response.messages.into_iter().skip(seen).collect());
                    self.context = run.context.clone();
                    return Ok(answer);
                }
                _ => {}
            }
        }
        Err(Failure::Other(anyhow!("the run ended without an answer")))
    }
}

fn overflowed(error: &PromptError) -> bool {
    match error {
        PromptError::Provider(error) => error.report().is_context_overflow(),
        PromptError::Report(report) => report.is_context_overflow(),
        _ => false,
    }
}

/// Holds each tool call to its tool's schema, as rig-agent doesn't and a
/// local model has no grammar holding it to, sends one to a tool that isn't
/// there back to the model, and tells the panel about the rest.
struct Calls {
    schemas: HashMap<String, Value>,
    events: Events,
    /// Whether a call reached its tool, after which a run can't start over.
    acted: Arc<AtomicBool>,
}

impl AgentHook for Calls {
    async fn on_invalid_tool_call(
        &self,
        _ctx: &HookContext,
        event: &InvalidToolCallContext,
    ) -> Option<InvalidToolCallAction> {
        // rig sends malformed arguments back by itself, but ends a run at a
        // tool it doesn't have.
        matches!(event.reason, InvalidToolCallReason::UnknownTool).then(|| {
            InvalidToolCallAction::retry(format!(
                "no tool is named {}; the tools are {}",
                event.tool_name,
                event.available_tools.join(", ")
            ))
        })
    }

    async fn on_dispatch(&self, _ctx: &HookContext, event: DispatchEvent<'_>) -> DispatchAction {
        let EffectKind::ToolCall { name, args } = event.kind else {
            return DispatchAction::proceed();
        };
        let arguments = serde_json::from_str(args).unwrap_or_else(|_| Value::String(args.clone()));
        (self.events)(Event::ToolStarted {
            tool: name.clone(),
            arguments: arguments.clone(),
        });
        let refusal = self
            .schemas
            .get(name)
            .and_then(|schema| refusal(schema, &arguments));
        if let Some(refusal) = refusal {
            // A denied call has no outcome, so it finishes here.
            (self.events)(Event::ToolFinished {
                tool: name.clone(),
                error: Some(refusal.clone()),
            });
            return DispatchAction::skip(refusal);
        }
        self.acted.store(true, Ordering::Relaxed);
        DispatchAction::proceed()
    }

    async fn on_outcome(&self, _ctx: &HookContext, event: OutcomeEvent<'_>) -> OutcomeAction {
        if let EffectKind::ToolCall { name, .. } = event.kind {
            let error = match event.outcome {
                Ok(Answered::ToolResult { result }) => {
                    result.error().map(|error| error.message().to_owned())
                }
                Ok(_) => None,
                Err(report) => Some(report.to_string()),
            };
            (self.events)(Event::ToolFinished {
                tool: name.clone(),
                error,
            });
        }
        OutcomeAction::proceed()
    }
}

/// Keeps a run inside the model's window, as rig's own agent does: before each
/// model call, older messages are summarized if the call wouldn't fit.
struct Window {
    model: DynModel<Completion>,
    run: Arc<Mutex<Run>>,
}

/// A run's own copy of the conversation, which goes with a run that fails.
struct Run {
    context: Context,
    /// How much of rig's history of the run it has.
    seen: usize,
    /// What the last model call reported, which covers its reply.
    usage: Usage,
}

impl Run {
    /// Adds what rig's history gained, which ends in the last reply.
    fn record(&mut self, messages: Vec<Message>) {
        if !messages.is_empty() {
            self.seen += messages.len();
            self.context.record([(messages, self.usage)]);
        }
    }
}

impl AgentHook for Window {
    async fn on_completion_call(
        &self,
        _ctx: &HookContext,
        event: CompletionCallEvent<'_>,
    ) -> CompletionCallAction {
        let mut run = self.run.lock().await;
        let gained = event.history.get(run.seen..).unwrap_or_default();
        run.record(
            gained
                .iter()
                .map(|message| Message::clone(message))
                .collect(),
        );
        run.context.fit(&self.model, event.prompt).await;
        let mut patch = RequestPatch::new();
        patch.history = Some(run.context.messages().to_vec());
        CompletionCallAction::patch(patch)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, PoisonError};

    use rig_core::{
        catalog::ModelSpec,
        providers::registry::ProviderId,
        test_utils::{MockCompletionModel, MockError, MockStreamEvent},
    };

    use super::*;

    /// Tools on a project with one page, and the page.
    fn project() -> (tempfile::TempDir, Tools, koharu_scene::PageId) {
        let dir = tempfile::tempdir().expect("a folder");
        let mut session = koharu_scene::Session::create(
            dir.path().join("test.khr"),
            koharu_scene::Meta::default(),
        )
        .expect("a project");
        let page = json!({"op": "add_page", "index": 0, "size": {"width": 100, "height": 100}});
        let change = session
            .apply(koharu_scene::Edit {
                label: "test".to_owned(),
                ops: vec![serde_json::from_value(page).expect("an op")],
            })
            .expect("a page");
        let Some(koharu_scene::Created::Page { id, .. }) = change.created.first().cloned() else {
            panic!("{change:?}");
        };
        let tools = Tools::new(
            Arc::new(parking_lot::Mutex::new(session)),
            koharu_pipeline::Pipeline::new(koharu_pipeline::PipelineConfig::default()),
            koharu_renderer::Renderer::new(),
        );
        (dir, tools, id)
    }

    /// A model that takes tools, streaming `turns` in order.
    fn model(turns: Vec<Vec<MockStreamEvent>>) -> MockCompletionModel {
        let vendor = ProviderId::declare("mock").expect("a vendor");
        let spec = ModelSpec::new(vendor, "mock")
            .with_tools(true)
            .with_context_window(16_384);
        MockCompletionModel::from_stream_turns(turns).priced_as(spec)
    }

    fn reply(text: &str) -> Vec<MockStreamEvent> {
        vec![
            MockStreamEvent::text(text),
            MockStreamEvent::final_response_with_default_usage(),
        ]
    }

    fn call(id: &str, tool: &str, arguments: Value) -> Vec<MockStreamEvent> {
        vec![
            MockStreamEvent::tool_call(id, tool, arguments),
            MockStreamEvent::final_response_with_default_usage(),
        ]
    }

    fn usage(tokens: u64) -> Usage {
        let mut usage = Usage::default();
        usage.input_tokens = Some(tokens);
        usage
    }

    #[tokio::test]
    async fn a_model_runs_the_tools() {
        let (_dir, tools, page) = project();
        let model = model(vec![
            call("1", "get_page", json!({"page": page})),
            reply("The page is empty."),
        ]);
        let mut agent = Agent::new(tools, model.clone().erase(), Context::default());
        let events = Arc::new(Mutex::new(Vec::new()));
        let heard = Arc::clone(&events);
        let answer = agent
            .run("What is on the page?", move |event| {
                heard
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(event);
            })
            .await
            .expect("an answer");
        assert_eq!(answer, "The page is empty.");
        assert_eq!(model.request_count(), 2);
        // The tool found the project in its context and answered.
        let events = events.lock().unwrap_or_else(PoisonError::into_inner);
        assert!(
            matches!(
                events.as_slice(),
                [Event::ToolStarted { tool, .. }, Event::ToolFinished { error: None, .. }, ..]
                    if tool == "get_page"
            ),
            "{events:?}"
        );
        // The conversation lives on.
        assert!(agent.context().messages().len() >= 4);
    }

    /// A model with no grammar holding it to the schema can write arguments
    /// that don't fit, and the check sends them back rather than running the
    /// tool.
    #[tokio::test]
    async fn arguments_that_do_not_fit_go_back_to_the_model() {
        let (_dir, tools, page) = project();
        let model = model(vec![
            call("1", "get_page", json!({})),
            call("2", "get_page", json!({"page": page})),
            reply("Done."),
        ]);
        let mut agent = Agent::new(tools, model.clone().erase(), Context::default());
        let answer = agent
            .run("Read the page.", |_| {})
            .await
            .expect("an answer");
        assert_eq!(answer, "Done.");
        let second = format!("{:?}", model.requests()[1]);
        assert!(second.contains("the arguments don't fit the tool's schema"));
    }

    /// The summary goes out between the run's calls, in place of older
    /// messages, and the call it was made for keeps its tool call and result.
    #[tokio::test]
    async fn a_run_that_fills_the_window_is_summarized_and_goes_on() {
        let (_dir, tools, page) = project();
        let model = model(vec![
            vec![
                MockStreamEvent::tool_call("1", "get_page", json!({"page": page})),
                MockStreamEvent::final_response(usage(13_000)),
            ],
            reply("The story so far."),
            reply("Done."),
        ]);
        let mut context = Context::default();
        let earlier = (0..20)
            .flat_map(|_| {
                [
                    Message::user("x".repeat(1000)),
                    Message::assistant("y".repeat(1000)),
                ]
            })
            .collect();
        context.record([(earlier, usage(10_000))]);
        let mut agent = Agent::new(tools, model.clone().erase(), context);
        let answer = agent
            .run("Read the page.", |_| {})
            .await
            .expect("an answer");
        assert_eq!(answer, "Done.");
        assert_eq!(model.request_count(), 3);
        let last = format!("{:?}", model.requests()[2].chat_history);
        assert!(last.contains("The story so far."));
        assert!(last.contains("ToolCall") && last.contains("ToolResult"));
        // The conversation goes on from the summary, with the whole run.
        let kept = agent.context().messages();
        assert!(kept.len() < 40, "{}", kept.len());
        assert!(format!("{:?}", kept[0]).contains("The story so far."));
        assert_eq!(*kept[kept.len() - 4], Message::user("Read the page."));
    }

    #[tokio::test]
    async fn a_request_refused_as_too_long_starts_over() {
        let (_dir, tools, _) = project();
        let overflow = MockStreamEvent::Error(MockError::request("the prompt is too long"));
        let model = model(vec![vec![overflow], reply("Done.")]);
        let mut agent = Agent::new(tools, model.clone().erase(), Context::default());
        let answer = agent.run("Go on.", |_| {}).await.expect("an answer");
        assert_eq!(answer, "Done.");
        assert_eq!(model.request_count(), 2);
    }
}
