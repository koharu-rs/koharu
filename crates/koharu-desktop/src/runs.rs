//! What runs on the open project besides the editor: the pipeline's runs,
//! told to the interface and measured, and the agent's run in the project's
//! conversation.

use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use anyhow::{Context as _, Result};
use koharu_agent::{Agent, AgentConfig};
use koharu_llm::{Context, Size, SummaryPrompts};
use koharu_pipeline::{Event, Outcome, Request, RunId, Stage, Target};
use koharu_scene::PageId;
use parking_lot::Mutex;
use serde::Serialize;
use specta::Type;
use tokio::sync::broadcast::error::RecvError;
use tokio_util::task::AbortOnDropHandle;
use tracing::{Instrument as _, Span};

use crate::{app::App, events};

/// What an agent run tells the panel: the agent's events as they come, then
/// how it ended. A cancelled run tells nothing more.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    Event { event: koharu_agent::Event },
    Completed { answer: String },
    Failed { error: String },
}

/// The pipeline's runs as Sentry sees them: a span each, continuing the trace
/// of what asked for it, and each page stage's time.
#[derive(Default)]
pub struct Telemetry {
    spans: Mutex<BTreeMap<RunId, Span>>,
    started: Mutex<BTreeMap<(RunId, PageId, Stage), Instant>>,
    /// The agent's runs, numbered in the order they started.
    agent_runs: AtomicU64,
}

impl Telemetry {
    /// Counts and times `event` as metrics, and ends a run's span with it.
    fn measure(&self, event: &Event) {
        match event {
            Event::Queued { .. } => {}
            Event::Started { run, page, stage } => {
                self.started
                    .lock()
                    .insert((*run, *page, *stage), Instant::now());
            }
            Event::Finished { run, page, stage } => {
                let stage_name = stage.to_string();
                if let Some(start) = self.started.lock().remove(&(*run, *page, *stage)) {
                    sentry::metrics::distribution(
                        "pipeline.stage.duration",
                        start.elapsed().as_secs_f64() * 1000.0,
                    )
                    .unit(sentry::protocol::Unit::Millisecond)
                    .attribute("stage", stage_name.clone())
                    .capture();
                }
                sentry::metrics::counter("pipeline.stage.pages", 1)
                    .attribute("stage", stage_name)
                    .capture();
            }
            Event::Ended { run, outcome } => {
                // A failed or stopped run leaves stages it never finished.
                self.started
                    .lock()
                    .retain(|(started, _, _), _| started != run);
                let outcome = match outcome {
                    Outcome::Done => "done",
                    Outcome::Failed { .. } => "failed",
                    Outcome::Stopped => "stopped",
                };
                sentry::metrics::counter("pipeline.runs", 1)
                    .attribute("outcome", outcome)
                    .capture();
                if let Some(span) = self.spans.lock().remove(run) {
                    span.record("outcome", outcome);
                }
            }
        }
    }
}

impl App {
    /// Runs `stages` on `target` in the open project. `trace`, a
    /// `sentry-trace` value, is the trace of what asked for the run, which
    /// the run's span continues.
    pub fn process(
        &self,
        target: Target,
        stages: BTreeSet<Stage>,
        trace: Option<String>,
    ) -> Result<RunId> {
        let span = tracing::info_span!(
            "pipeline run",
            sentry.op = "pipeline.run",
            sentry.trace = trace.as_deref().unwrap_or_default(),
            stages = ?stages,
            outcome = tracing::field::Empty,
        );
        let id = self.pipeline.run(
            self.session()?,
            Request {
                target,
                stages,
                config: None,
            },
        )?;
        self.telemetry.spans.lock().insert(id, span);
        Ok(id)
    }

    /// Tells the interface the pipeline's events and the runtime's
    /// downloads for as long as the app runs.
    pub fn follow_pipeline(self: &Arc<Self>) {
        let app = Arc::clone(self);
        let mut events = self.pipeline.events();
        tauri::async_runtime::spawn(async move {
            loop {
                let event = match events.recv().await {
                    Ok(event) => event,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return,
                };
                app.telemetry.measure(&event);
                app.emit(events::PIPELINE, event);
            }
        });
        let app = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            use futures::StreamExt as _;
            let mut transfers = koharu_runtime::transfers();
            while let Some(transfer) = transfers.next().await {
                app.emit(events::DOWNLOAD, transfer);
            }
        });
    }

    /// Runs `prompt` on the open project with `model`, in its conversation,
    /// and answers the run's id at once; `send` hears the run's events and
    /// how it ended. The model connects inside the run, so closing the
    /// project stops a connection as it stops a run. `trace`, a
    /// `sentry-trace` value, is the trace the run's span continues.
    pub fn run_agent(
        self: &Arc<Self>,
        model: &str,
        config: AgentConfig,
        prompt: String,
        send: impl Fn(AgentEvent) + Send + Sync + 'static,
        trace: Option<String>,
    ) -> Result<u64> {
        let span = tracing::info_span!(
            "agent run",
            sentry.op = "agent.run",
            sentry.trace = trace.as_deref().unwrap_or_default(),
            model,
        );
        let app = Arc::clone(self);
        let model = model.to_owned();
        let send = Arc::new(send);
        self.start_agent(
            async move {
                let events = Arc::clone(&send);
                let result = async {
                    let conversation = app.conversation(&model).await?;
                    let mut agent = conversation.lock().await;
                    agent.set_config(config);
                    anyhow::Ok(
                        agent
                            .run(&prompt, move |event| events(AgentEvent::Event { event }))
                            .await?,
                    )
                }
                .await;
                send(match result {
                    Ok(answer) => AgentEvent::Completed { answer },
                    Err(error) => AgentEvent::Failed {
                        error: format!("{error:#}"),
                    },
                });
            }
            .instrument(span),
        )
    }

    /// Starts an agent run on the open project in place of the one going,
    /// and answers its id.
    pub fn start_agent(
        self: &Arc<Self>,
        run: impl Future<Output = ()> + Send + 'static,
    ) -> Result<u64> {
        let id = self.telemetry.agent_runs.fetch_add(1, Ordering::Relaxed) + 1;
        let mut project = self.project.lock();
        let project = project.as_mut().context("no project is open")?;
        let app = Arc::clone(self);
        let task = tokio::spawn(async move {
            run.await;
            if let Some(project) = app.project.lock().as_mut()
                && project.run.as_ref().is_some_and(|(going, _)| *going == id)
            {
                // The task is past its last await, so its own abort is moot.
                project.run = None;
            }
        });
        // The run it replaces is aborted as its handle drops.
        project.run = Some((id, AbortOnDropHandle::new(task)));
        Ok(id)
    }

    pub fn cancel_agent(&self, run: u64) {
        if let Some(project) = self.project.lock().as_mut()
            && project.run.as_ref().is_some_and(|(going, _)| *going == run)
        {
            project.run = None;
        }
    }

    /// The open project's conversation with the agent on `model`, made on
    /// its first run.
    async fn conversation(&self, model: &str) -> Result<Arc<tokio::sync::Mutex<Agent>>> {
        let (session, tools) = {
            let project = self.project.lock();
            let project = project.as_ref().context("no project is open")?;
            if let Some((made, conversation)) = &project.conversation
                && made == model
            {
                return Ok(Arc::clone(conversation));
            }
            (Arc::clone(&project.session), self.tools(project))
        };
        let connected = koharu_llm::connect(model, Size::AGENT).await?;
        let conversation = Arc::new(tokio::sync::Mutex::new(Agent::new(
            tools,
            connected,
            Context::new(SummaryPrompts::DEFAULT),
        )));
        // Kept only while its project is still the open one: one made during
        // a close goes with the run that made it.
        if let Some(project) = self.project.lock().as_mut()
            && Arc::ptr_eq(&project.session, &session)
        {
            project.conversation = Some((model.to_owned(), Arc::clone(&conversation)));
        }
        Ok(conversation)
    }
}
