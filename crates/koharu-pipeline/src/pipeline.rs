use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use koharu_scene::{ProjectId, Session};
use parking_lot::{Mutex, RwLock};
use tokio::sync::{broadcast, mpsc};

use crate::{
    Event, PipelineConfig, Request, Result, RunId,
    scheduler::{Command, Plan, Run, Scheduler},
};

const EVENTS: usize = 1024;

/// Runs Koharu's models for the app, its canvas, and the agent alike: a
/// cheap handle to clone. The scheduler and the thread owning the models end
/// when the last handle drops.
#[derive(Clone)]
pub struct Pipeline(Arc<Shared>);

struct Shared {
    config: Arc<RwLock<PipelineConfig>>,
    events: broadcast::Sender<Event>,
    commands: mpsc::UnboundedSender<Command>,
    next: AtomicU64,
}

impl Pipeline {
    /// Starts the scheduler and the thread owning the models, so it needs a
    /// Tokio runtime. Models load on first use.
    #[must_use]
    pub fn new(config: PipelineConfig) -> Self {
        let config = Arc::new(RwLock::new(config));
        let events = broadcast::channel(EVENTS).0;
        let commands = Scheduler::spawn(Arc::clone(&config), events.clone());
        Self(Arc::new(Shared {
            config,
            events,
            commands,
            next: AtomicU64::new(0),
        }))
    }

    /// Follows a changed `[pipeline]`. A model switched away from leaves
    /// memory once the work queued with it ends; runs already asked for keep
    /// the settings they started with.
    pub fn set_config(&self, config: PipelineConfig) {
        *self.0.config.write() = config;
        self.send(Command::Settle);
    }

    /// What every run tells the app from now on.
    #[must_use]
    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.0.events.subscribe()
    }

    /// Queues `request` on the project `session` holds and returns its run.
    pub fn run(&self, session: Arc<Mutex<Session>>, request: Request) -> Result<RunId> {
        let config = request
            .config
            .unwrap_or_else(|| self.0.config.read().clone());
        let project = session.lock().project();
        let plan = Plan::new(&project, &request.target, &request.stages)?;
        let id = RunId(self.0.next.fetch_add(1, Ordering::Relaxed));
        let run = Run::new(id, session, project.id(), config, plan, &request.stages);
        self.send(Command::Run(run));
        Ok(id)
    }

    /// Stops every run, running and queued. What they finished stays, and
    /// nothing more is written; a model call on the GPU finishes, but a
    /// local translation stops at its next token.
    pub fn stop(&self) {
        self.send(Command::Stop);
    }

    /// Unloads every model once the GPU's current work ends, as ComfyUI's
    /// Unload Models does.
    pub fn unload(&self) {
        self.send(Command::Unload);
    }

    /// Forgets the translation conversation of a project that closed.
    pub fn close(&self, project: ProjectId) {
        self.send(Command::Close(project));
    }

    fn send(&self, command: Command) {
        self.0
            .commands
            .send(command)
            .unwrap_or_else(|_| panic!("the scheduler runs while the pipeline does"));
    }
}
