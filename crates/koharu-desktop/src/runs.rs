//! What runs on the open project besides the editor: the pipeline's runs as
//! the interface lists them, the agent's run with its conversation, and the
//! MCP server serving the agent's tools to outside agents.

use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    sync::Arc,
    time::Instant,
};

use anyhow::{Context as _, Result};
use koharu_agent::{Agent, AgentConfig, McpServer, Tools};
use koharu_llm::{Context, Size, SummaryPrompts};
use koharu_pipeline::{Event, Outcome, Request, RunId, Stage, Target};
use koharu_scene::{PageId, Project};
use parking_lot::Mutex;
use serde::Serialize;
use specta::Type;
use tokio::{sync::broadcast::error::RecvError, task::AbortHandle};
use tracing::{Instrument as _, Span};

use crate::app::App;

/// The port the MCP server listens on, the `serve` example's, so a client's
/// configuration stays valid from one launch to the next.
const MCP_PORT: u16 = 7878;

#[derive(Default)]
pub struct Runs {
    jobs: Mutex<BTreeMap<RunId, Job>>,
    /// Each pipeline run's span, open until it ends; it continues the trace
    /// of what asked for the run.
    spans: Mutex<BTreeMap<RunId, Span>>,
    /// When each page's stage started, for its time.
    started: Mutex<BTreeMap<(RunId, PageId, Stage), Instant>>,
    agent: Mutex<AgentRun>,
    mcp: Mutex<Option<McpServer>>,
}

#[derive(Default)]
struct AgentRun {
    /// The open project's conversation, on the model it was made with.
    conversation: Option<(String, Arc<tokio::sync::Mutex<Agent>>)>,
    /// The run going on, cancelled by aborting its task, which drops the
    /// agent's future as koharu-agent cancels a run.
    run: Option<(u64, AbortHandle)>,
    next: u64,
}

/// What an agent run tells the panel: the agent's events as they come, then
/// how it ended. A cancelled run tells nothing more.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    Event { event: koharu_agent::Event },
    Completed { answer: String },
    Failed { error: String },
}

/// A pipeline run as the interface shows it.
#[derive(Clone, Debug, Serialize, Type)]
pub struct Job {
    pub id: RunId,
    pub state: JobState,
    /// The page stages done, of `total`.
    pub completed: usize,
    pub total: usize,
    /// The stage started last, and its page.
    pub stage: Option<Stage>,
    pub page: Option<PageId>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Done,
    Failed,
    Stopped,
}

impl Job {
    fn new(id: RunId) -> Self {
        Self {
            id,
            state: JobState::Running,
            completed: 0,
            total: 0,
            stage: None,
            page: None,
            error: None,
        }
    }
}

impl Runs {
    pub fn jobs(&self) -> Vec<Job> {
        self.jobs.lock().values().cloned().collect()
    }

    /// The job `event` changes, and whether its run wrote to the project.
    fn record(&self, event: &Event) -> (Job, bool) {
        let mut jobs = self.jobs.lock();
        let (run, wrote) = match event {
            Event::Started { run, .. } => (*run, false),
            Event::Finished { run, .. } | Event::Ended { run, .. } => (*run, true),
        };
        let job = jobs.entry(run).or_insert_with(|| Job::new(run));
        match event {
            Event::Started { page, stage, .. } => {
                job.stage = Some(*stage);
                job.page = Some(*page);
            }
            Event::Finished { .. } => job.completed += 1,
            Event::Ended { outcome, .. } => {
                job.state = match outcome {
                    Outcome::Done => JobState::Done,
                    Outcome::Stopped => JobState::Stopped,
                    Outcome::Failed { reason, .. } => {
                        job.error = Some(reason.clone());
                        JobState::Failed
                    }
                };
            }
        }
        (job.clone(), wrote)
    }

    /// Counts and times `event` as metrics, and ends a run's span with it.
    fn measure(&self, event: &Event) {
        match event {
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

/// How many page stages a run of `stages` on `target` does.
fn total(project: &Project, target: &Target, stages: usize) -> Result<usize> {
    let pages = match target {
        Target::Project => project.pages().count(),
        Target::Pages(pages) => pages.len(),
        Target::Blocks(blocks) => blocks
            .iter()
            .map(|&block| Ok(project.parent(block)?.0.id))
            .collect::<Result<BTreeSet<_>>>()?
            .len(),
        Target::Area { .. } => 1,
    };
    Ok(pages * stages)
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
        let session = self.session()?;
        let total = total(&session.lock().project(), &target, stages.len())?;
        let span = tracing::info_span!(
            "pipeline run",
            sentry.op = "pipeline.run",
            sentry.trace = trace.as_deref().unwrap_or_default(),
            stages = ?stages,
            pages = total / stages.len().max(1),
            outcome = tracing::field::Empty,
        );
        let id = self.pipeline.run(
            session,
            Request {
                target,
                stages,
                config: None,
            },
        )?;
        let job = {
            let mut jobs = self.runs.jobs.lock();
            let job = jobs.entry(id).or_insert_with(|| Job::new(id));
            job.total = total;
            job.clone()
        };
        self.runs.spans.lock().insert(id, span);
        self.channels.job(job);
        Ok(id)
    }

    /// Follows the pipeline's events and the runtime's downloads for as
    /// long as the app runs: each job to the interface, and each write to
    /// the project through `changed`.
    pub fn follow(self: &Arc<Self>) {
        let app = Arc::clone(self);
        let mut events = self.pipeline.events();
        tauri::async_runtime::spawn(async move {
            loop {
                let event = match events.recv().await {
                    Ok(event) => event,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return,
                };
                let (job, wrote) = app.runs.record(&event);
                app.runs.measure(&event);
                app.channels.job(job);
                if wrote {
                    let app = Arc::clone(&app);
                    tauri::async_runtime::spawn_blocking(move || {
                        if let Err(error) = app.changed() {
                            tracing::error!("failed to show a run's write: {error:#}");
                        }
                    });
                }
            }
        });
        let app = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            use futures::StreamExt as _;
            let mut transfers = koharu_runtime::transfers();
            while let Some(transfer) = transfers.next().await {
                app.channels.download(transfer);
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
    ) -> u64 {
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
                    let writes = Arc::clone(&app);
                    anyhow::Ok(
                        agent
                            .run(&prompt, move |event| {
                                // A tool may have written to the project.
                                if matches!(event, koharu_agent::Event::ToolFinished { .. }) {
                                    let app = Arc::clone(&writes);
                                    tauri::async_runtime::spawn_blocking(move || {
                                        if let Err(error) = app.changed() {
                                            tracing::error!(
                                                "failed to show the agent's write: {error:#}"
                                            );
                                        }
                                    });
                                }
                                events(AgentEvent::Event { event });
                            })
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

    /// Starts an agent run in place of the one going, and answers its id.
    pub fn start_agent(self: &Arc<Self>, run: impl Future<Output = ()> + Send + 'static) -> u64 {
        let mut agent = self.runs.agent.lock();
        if let Some((_, task)) = agent.run.take() {
            task.abort();
        }
        agent.next += 1;
        let id = agent.next;
        let app = Arc::clone(self);
        let task = tokio::spawn(async move {
            run.await;
            let mut agent = app.runs.agent.lock();
            if agent.run.as_ref().is_some_and(|(run, _)| *run == id) {
                agent.run = None;
            }
        });
        agent.run = Some((id, task.abort_handle()));
        id
    }

    /// The agent's run going on.
    pub fn agent_run(&self) -> Option<u64> {
        self.runs.agent.lock().run.as_ref().map(|(run, _)| *run)
    }

    pub fn cancel_agent(&self, run: u64) {
        let mut agent = self.runs.agent.lock();
        if agent.run.as_ref().is_some_and(|(going, _)| *going == run)
            && let Some((_, task)) = agent.run.take()
        {
            task.abort();
        }
    }

    /// The open project's conversation with the agent on `model`, made on
    /// its first run.
    pub async fn conversation(&self, model: &str) -> Result<Arc<tokio::sync::Mutex<Agent>>> {
        if let Some((made, conversation)) = &self.runs.agent.lock().conversation
            && made == model
        {
            return Ok(Arc::clone(conversation));
        }
        let session = self.session()?;
        let connected = koharu_llm::connect(model, Size::AGENT).await?;
        let tools = Tools::new(
            Arc::clone(&session),
            self.pipeline.clone(),
            self.renderer.clone(),
        );
        let conversation = Arc::new(tokio::sync::Mutex::new(Agent::new(
            tools,
            connected,
            Context::new(SummaryPrompts::DEFAULT),
        )));
        // Kept only while its project is still the open one, under the
        // project's lock: `close` takes the project before it drops the
        // conversation, so one made during a close never outlives it.
        let project = self.project.lock();
        if project
            .as_ref()
            .is_some_and(|open| Arc::ptr_eq(&open.session, &session))
        {
            self.runs.agent.lock().conversation =
                Some((model.to_owned(), Arc::clone(&conversation)));
        }
        drop(project);
        Ok(conversation)
    }

    /// Stops what runs on the open project, before it closes: the
    /// pipeline's runs, the agent's run and conversation, and MCP's calls.
    pub fn stop_runs(&self) {
        self.pipeline.stop();
        let mut agent = self.runs.agent.lock();
        if let Some((_, task)) = agent.run.take() {
            task.abort();
        }
        agent.conversation = None;
        drop(agent);
        if let Some(server) = self.runs.mcp.lock().as_ref() {
            server.open(None);
        }
    }

    /// Points MCP's calls at the open project.
    pub fn serve(&self) {
        if let Some(server) = self.runs.mcp.lock().as_ref() {
            server.open(self.tools());
        }
    }

    fn tools(&self) -> Option<Tools> {
        let session = self.session().ok()?;
        Some(Tools::new(
            session,
            self.pipeline.clone(),
            self.renderer.clone(),
        ))
    }

    /// Starts or stops the MCP server, and answers its URL while it runs.
    pub async fn set_mcp(&self, enabled: bool) -> Result<Option<String>> {
        if !enabled {
            drop(self.runs.mcp.lock().take());
            return Ok(None);
        }
        let running = self.runs.mcp.lock().as_ref().map(McpServer::address);
        let address = match running {
            Some(address) => address,
            None => {
                let server = McpServer::start(MCP_PORT)
                    .await
                    .with_context(|| format!("couldn't serve MCP on port {MCP_PORT}"))?;
                server.open(self.tools());
                let address = server.address();
                *self.runs.mcp.lock() = Some(server);
                address
            }
        };
        Ok(Some(format!("http://{address}/mcp")))
    }

    pub fn mcp(&self) -> Option<String> {
        self.runs
            .mcp
            .lock()
            .as_ref()
            .map(|server| format!("http://{}/mcp", server.address()))
    }
}
