//! The agent: rig's agent loop over the tools the app's host gives it, on a
//! hosted model.

use std::{future::Future, sync::Arc};

use anyhow::{Context, Result, bail};
use futures::StreamExt;
use rig_agent::agent::{AgentBuilder, MultiTurnStreamItem};
use rig_core::{
    completion::{Effort, GenerationOptions, Message},
    message::ToolName,
    streaming::{Item, StreamEvent},
    tool::{DynamicTool, ToolErrorKind, ToolExecutionError, ToolOutput},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Selection, hosted};

const INSTRUCTIONS: &str = "You are Koharu Agent, operating a manga translation project inside Koharu.
The current project state is supplied in a koharu_project_context block on every user turn.
Use the provided tools whenever the user asks to inspect or change the project. Never claim a change succeeded unless its tool result says it succeeded.
All project changes are revisioned and reversible. Do not ask for permission. Continue using tools until the request is complete or cannot be completed.
Do not invent identifiers. Preserve artwork and existing authored content unless the user asks to change them.";

/// A run stops after this many model calls; rig's default is one.
const MAX_TURNS: usize = 20;

/// `[agent]`: the agent's model and how much it reasons.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct AgentConfig {
    pub model: Option<Selection>,
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

/// The agent's conversation about one open project, kept in memory.
pub struct Agent<H> {
    host: Arc<H>,
    history: Vec<Message>,
}

impl<H: Host> Agent<H> {
    pub fn new(host: Arc<H>) -> Self {
        Self {
            host,
            history: Vec::new(),
        }
    }

    pub fn history(&self) -> &[Message] {
        &self.history
    }

    /// Runs `prompt` until the model answers without tools, on the model
    /// `[agent]` names, and answers with its last text. `events` hears the
    /// run as it goes. Dropping the future cancels the run; a tool already
    /// running finishes, as each edit is whole or not at all.
    pub async fn run(
        &mut self,
        prompt: &str,
        events: impl Fn(Event) + Send + Sync + 'static,
    ) -> Result<String> {
        let config: AgentConfig = koharu_config::get()?;
        let selection = config.model.context("[agent] names no model")?;
        crate::hosted_only(selection.provider)?;
        let model = hosted::model(selection.provider, &selection.model).await?;
        let events = Arc::new(events);
        let tools = self
            .host
            .tools()
            .into_iter()
            .map(|tool| {
                let host = Arc::clone(&self.host);
                let events = Arc::clone(&events);
                let name = tool.name.clone();
                let tool_name = ToolName::new(tool.name).context("a tool has no name")?;
                Ok(DynamicTool::new(
                    tool_name,
                    tool.description,
                    tool.parameters,
                    move |arguments| {
                        let host = Arc::clone(&host);
                        let events = Arc::clone(&events);
                        let name = name.clone();
                        Box::pin(async move {
                            events(Event::ToolStarted {
                                tool: name.clone(),
                                arguments: arguments.clone(),
                            });
                            let tool = name.clone();
                            // Detached, so dropping the run doesn't cut an
                            // edit short.
                            let invocation =
                                tokio::spawn(async move { host.invoke(&tool, arguments).await })
                                    .await
                                    .unwrap_or_else(|error| {
                                        Err(format!("the tool failed: {error}"))
                                    });
                            events(Event::ToolFinished {
                                tool: name,
                                error: invocation.as_ref().err().cloned(),
                            });
                            invocation.map_err(|message| {
                                ToolExecutionError::new(ToolErrorKind::Other, message)
                            })
                        })
                    },
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut agent = AgentBuilder::new(model)
            .preamble(INSTRUCTIONS)
            .dynamic_tools(tools);
        if let Some(effort) = config.reasoning {
            agent = agent.options(GenerationOptions::default().reasoning(effort));
        }
        let message = Message::user(format!(
            "<koharu_project_context>\n{}\n</koharu_project_context>\n\n{prompt}",
            self.host.context()
        ));
        let mut stream = agent
            .build()
            .prompt(message)
            .history(self.history.clone())
            .max_turns(MAX_TURNS)
            .stream();
        while let Some(item) = stream.next().await {
            match item? {
                MultiTurnStreamItem::StreamAssistantItem(Item::Event(StreamEvent::Text {
                    text,
                    ..
                })) => events(Event::Text { delta: text }),
                MultiTurnStreamItem::StreamAssistantItem(Item::Event(StreamEvent::Reasoning {
                    text,
                    ..
                })) => events(Event::Reasoning { delta: text }),
                MultiTurnStreamItem::FinalResponse(response) => {
                    self.history.extend(response.messages.iter().cloned());
                    return Ok(response.output());
                }
                _ => {}
            }
        }
        bail!("the run ended without an answer")
    }
}
