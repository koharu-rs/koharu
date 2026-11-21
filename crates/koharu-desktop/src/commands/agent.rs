//! The agent panel: the in-app agent's run on the open project, one at a
//! time, in the project's conversation, both of which close with it. Its
//! settings are the preferences' `[agent]`.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use anyhow::Context as _;
use koharu_agent::{Agent, AgentConfig};
use koharu_llm::{Context, Size, SummaryPrompts};
use serde::Serialize;
use specta::Type;
use tauri::{State, ipc::Channel};
use tokio_util::task::AbortOnDropHandle;
use tracing::Instrument as _;

use super::Result;
use crate::app::App;

/// The agent's runs, numbered in the order they started.
static RUNS: AtomicU64 = AtomicU64::new(0);

/// What an agent run tells the panel: the agent's events as they come, then
/// how it ended. A cancelled run tells nothing more.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    Event { event: koharu_agent::Event },
    Completed { answer: String },
    Failed { error: String },
}

/// Runs `prompt` on the open project, in its conversation, in place of the
/// run going, and answers the run's id at once; `on_event` hears the run's
/// events and how it ended. The model connects inside the run, so closing
/// the project stops a connection as it stops a run.
#[tauri::command]
#[specta::specta]
pub async fn run_agent(
    app: State<'_, Arc<App>>,
    prompt: String,
    on_event: Channel<AgentEvent>,
) -> Result<u64> {
    let config: AgentConfig = koharu_config::get()?;
    let model = config
        .model
        .clone()
        .context("no agent model is set in the preferences")?;
    let span = tracing::info_span!("agent run", sentry.op = "agent.run", model,);
    let id = RUNS.fetch_add(1, Ordering::Relaxed) + 1;
    let mut project = app.project.lock();
    let project = project.as_mut().context("no project is open")?;
    let running = Arc::clone(&app);
    let task = tokio::spawn(
        async move {
            let events = on_event.clone();
            let result = async {
                let conversation = conversation(&running, &model).await?;
                let mut agent = conversation.lock().await;
                agent.set_config(config);
                anyhow::Ok(
                    agent
                        .run(&prompt, move |event| {
                            let _ = events.send(AgentEvent::Event { event });
                        })
                        .await?,
                )
            }
            .await;
            let _ = on_event.send(match result {
                Ok(answer) => AgentEvent::Completed { answer },
                Err(error) => AgentEvent::Failed {
                    error: format!("{error:#}"),
                },
            });
            if let Some(project) = running.project.lock().as_mut()
                && project.run.as_ref().is_some_and(|(going, _)| *going == id)
            {
                // The task is past its last await, so its own abort is moot.
                project.run = None;
            }
        }
        .instrument(span),
    );
    // The run it replaces is aborted as its handle drops.
    project.run = Some((id, AbortOnDropHandle::new(task)));
    Ok(id)
}

#[tauri::command]
#[specta::specta]
pub fn stop_agent(app: State<'_, Arc<App>>, run: u64) {
    if let Some(project) = app.project.lock().as_mut()
        && project.run.as_ref().is_some_and(|(going, _)| *going == run)
    {
        project.run = None;
    }
}

/// Starts the open project's conversation over, as a chat app's new chat
/// does: the run going stops, and the next run begins a new conversation.
#[tauri::command]
#[specta::specta]
pub fn reset_agent(app: State<'_, Arc<App>>) {
    if let Some(project) = app.project.lock().as_mut() {
        project.run = None;
        project.conversation = None;
    }
}

/// The open project's conversation with the agent on `model`, made on its
/// first run.
async fn conversation(app: &App, model: &str) -> anyhow::Result<Arc<tokio::sync::Mutex<Agent>>> {
    let (session, tools) = {
        let project = app.project.lock();
        let project = project.as_ref().context("no project is open")?;
        if let Some((made, conversation)) = &project.conversation
            && made == model
        {
            return Ok(Arc::clone(conversation));
        }
        (Arc::clone(&project.session), app.tools(project))
    };
    let connected = koharu_llm::connect(model, Size::AGENT).await?;
    let conversation = Arc::new(tokio::sync::Mutex::new(Agent::new(
        tools,
        connected,
        Context::new(SummaryPrompts::DEFAULT),
    )));
    // Kept only while its project is still the open one: one made during a
    // close goes with the run that made it.
    if let Some(project) = app.project.lock().as_mut()
        && Arc::ptr_eq(&project.session, &session)
    {
        project.conversation = Some((model.to_owned(), Arc::clone(&conversation)));
    }
    Ok(conversation)
}
