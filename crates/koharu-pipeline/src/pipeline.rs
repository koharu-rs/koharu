use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use koharu_scene::{ProjectId, Session};
use parking_lot::{Mutex, RwLock};
use tokio::sync::{broadcast, mpsc};

use crate::{
    Error, Event, PipelineConfig, Request, Result, RunId,
    event::Events,
    scheduler::{Command, Plan, Run, Scheduler},
};

/// Runs Koharu's models for the app, its canvas, and the agent alike: a
/// cheap handle to clone. The scheduler and the thread owning the models end
/// when the last handle drops.
#[derive(Clone)]
pub struct Pipeline(Arc<Shared>);

struct Shared {
    config: Arc<RwLock<PipelineConfig>>,
    events: Events,
    commands: mpsc::UnboundedSender<Command>,
    next: AtomicU64,
}

impl Pipeline {
    /// Starts the scheduler and the thread owning the models, so it needs a
    /// Tokio runtime. Models load on first use.
    #[must_use]
    pub fn new(config: PipelineConfig) -> Self {
        let config = Arc::new(RwLock::new(config));
        let events = Events::new();
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
        self.0.events.issued(id);
        self.send(Command::Run(run));
        Ok(id)
    }

    /// What `run` told so far, its events in order, once it ended or `wait`
    /// is over, whichever comes first; its last event says how it ended, as
    /// ComfyUI's `/history/{prompt_id}` keeps a prompt's.
    pub async fn history(&self, run: RunId, wait: Duration) -> Result<Vec<Event>> {
        self.0
            .events
            .history(run, wait)
            .await
            .ok_or_else(|| Error::invalid(format!("there is no run {}", run.0)))
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

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, time::Instant};

    use koharu_scene::Edit;
    use serde_json::json;

    use super::*;
    use crate::{Event, Outcome, Stage, Target};

    /// A project of one blank page, with no scan, so detection loads no
    /// model, in a folder removed with it.
    fn project() -> anyhow::Result<(tempfile::TempDir, Arc<Mutex<Session>>)> {
        let dir = tempfile::tempdir()?;
        let mut session = Session::create(dir.path().join("test.khr"))?;
        session.apply(Edit {
            label: "test".to_owned(),
            ops: vec![serde_json::from_value(json!({
                "op": "add_page", "index": 0, "size": {"width": 100, "height": 100},
            }))?],
        })?;
        Ok((dir, Arc::new(Mutex::new(session))))
    }

    #[tokio::test]
    async fn a_run_keeps_its_history_after_it_ended() {
        let (_dir, session) = project().expect("a project");
        let pipeline = Pipeline::new(PipelineConfig::default());
        let request = Request {
            target: Target::Project,
            stages: BTreeSet::from([Stage::Detect]),
            config: None,
        };
        let run = pipeline.run(session, request).expect("the run starts");
        let ended = Event::Ended {
            run,
            outcome: Outcome::Done,
        };
        for _ in 0..2 {
            let history = pipeline
                .history(run, Duration::from_secs(5))
                .await
                .expect("a history");
            assert_eq!(history.last(), Some(&ended));
        }
    }

    #[tokio::test]
    async fn an_unknown_run_fails_at_once() {
        let pipeline = Pipeline::new(PipelineConfig::default());
        let asked = Instant::now();
        let error = pipeline
            .history(RunId(999), Duration::from_secs(30))
            .await
            .expect_err("no such run");
        assert!(error.to_string().contains("999"), "{error}");
        assert!(asked.elapsed() < Duration::from_secs(1));
    }
}
