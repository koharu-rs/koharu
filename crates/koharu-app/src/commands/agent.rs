mod host;

use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use anyhow::{Context as _, Result, anyhow};
use koharu_agent::{Account, Agent, Codex, CodexModel, Config, Control, Event, LoginEvent, RunId};
use koharu_desktop::Desktop;
use parking_lot::Mutex;
use serde::Serialize;
use specta::Type;
use tokio::sync::Notify;

use self::host::KoharuHost;
use super::{
    Channel, Error,
    canvas::CanvasChannel,
    processing::{JobChannel, Processing},
    project::CurrentProject,
};
use crate::host::Pipeline;

#[derive(Clone, Debug, Serialize, Type)]
pub struct AgentStatus {
    pub account: Option<Account>,
    pub models: Vec<CodexModel>,
    pub config: Config,
    pub running: Option<RunId>,
}

#[derive(Clone)]
pub(crate) struct AgentState {
    agent: Arc<OnceLock<Agent<KoharuHost>>>,
    runs: Arc<Mutex<HashMap<RunId, Control>>>,
    login: Arc<Mutex<Option<Control>>>,
    idle: Arc<Notify>,
}

impl AgentState {
    pub(crate) fn new(
        project: CurrentProject,
        desktop: Desktop,
        canvas: CanvasChannel,
        processing: Processing,
        jobs: JobChannel,
        pipeline: Pipeline,
    ) -> Result<Self> {
        let state = Self::empty();
        state
            .agent
            .set(Agent::new(
                Codex::new()?,
                KoharuHost::new(project, desktop, canvas, processing, jobs, pipeline),
            )?)
            .map_err(|_| anyhow!("agent is already initialized"))?;
        Ok(state)
    }

    pub(crate) fn empty() -> Self {
        Self {
            agent: Arc::new(OnceLock::new()),
            runs: Arc::new(Mutex::new(HashMap::new())),
            login: Arc::new(Mutex::new(None)),
            idle: Arc::new(Notify::new()),
        }
    }

    fn agent(&self) -> Result<&Agent<KoharuHost>> {
        self.agent.get().context("agent is not initialized")
    }

    async fn status(&self) -> Result<AgentStatus> {
        let mut account = self.agent()?.codex().account()?;
        let models = if account.is_some() {
            match self.agent()?.models().await {
                Ok(models) => models,
                Err(error) => {
                    account = self.agent()?.codex().account()?;
                    if account.is_some() {
                        return Err(error);
                    }
                    self.agent()?.clear().await;
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        Ok(AgentStatus {
            account,
            models,
            config: self.agent()?.config()?,
            running: self.runs.lock().keys().next().copied(),
        })
    }

    pub(crate) async fn reset(&self) {
        if let Some(login) = self.login.lock().as_ref() {
            login.cancel();
        }
        for control in self.runs.lock().values() {
            control.cancel();
        }
        loop {
            let idle = self.idle.notified();
            if self.login.lock().is_none() && self.runs.lock().is_empty() {
                break;
            }
            idle.await;
        }
        if let Some(agent) = self.agent.get() {
            agent.clear().await;
        }
    }

    pub(crate) fn cancel_all(&self) {
        if let Some(login) = self.login.lock().take() {
            login.cancel();
        }
        for control in self.runs.lock().drain().map(|(_, control)| control) {
            control.cancel();
        }
    }
}

#[koharu_macros::command]
pub(crate) async fn get_agent_status(state: AgentState) -> std::result::Result<AgentStatus, Error> {
    Ok(state.status().await?)
}

#[tracing::instrument(
    target = "koharu_metrics",
    name = "agent_login",
    skip_all,
    fields(provider = "codex")
)]
#[koharu_macros::command]
pub(crate) async fn login_agent(
    state: AgentState,
    on_event: Channel<LoginEvent>,
) -> std::result::Result<AgentStatus, Error> {
    let control = Control::default();
    {
        let mut login = state.login.lock();
        if login.is_some() {
            return Err(anyhow!("Codex sign-in is already running").into());
        }
        *login = Some(control.clone());
    }
    let result = state
        .agent()?
        .codex()
        .login_device(&control, |event| {
            if let LoginEvent::DeviceCode {
                verification_url, ..
            } = &event
                && let Err(error) = open::that(verification_url)
            {
                tracing::warn!(%error, "failed to open the Codex device sign-in page");
            }
            let _ = on_event.send(event);
        })
        .await;
    state.login.lock().take();
    state.idle.notify_waiters();
    result?;
    Ok(state.status().await?)
}

#[tracing::instrument(
    target = "koharu_metrics",
    name = "agent_logout",
    skip_all,
    fields(provider = "codex")
)]
#[koharu_macros::command]
pub(crate) async fn logout_agent(state: AgentState) -> std::result::Result<AgentStatus, Error> {
    state.reset().await;
    state.agent()?.codex().logout()?;
    Ok(state.status().await?)
}

#[tracing::instrument(
    target = "koharu_metrics",
    name = "preferences_saved",
    skip_all,
    fields(setting = "agent")
)]
#[koharu_macros::command]
pub(crate) async fn save_agent_config(
    config: Config,
    state: AgentState,
) -> std::result::Result<Config, Error> {
    let config = state.agent()?.save_config(config)?;
    tracing::info!(
        target: "koharu_metrics",
        metric = "preference_changed",
        setting = "agent",
    );
    Ok(config)
}

#[koharu_macros::command]
pub(crate) async fn run_agent(
    prompt: String,
    on_event: Channel<Event>,
    state: AgentState,
) -> std::result::Result<RunId, Error> {
    let prompt = prompt.trim().to_owned();
    if prompt.is_empty() {
        return Err(anyhow!("message cannot be empty").into());
    }
    if state.agent()?.codex().account()?.is_none() {
        return Err(anyhow!("Codex is not signed in").into());
    }
    let run = RunId::new();
    let control = Control::default();
    {
        let mut runs = state.runs.lock();
        if !runs.is_empty() {
            return Err(anyhow!("another agent request is already running").into());
        }
        runs.insert(run, control.clone());
    }
    let agent = Arc::clone(&state.agent);
    let runs = state.runs.clone();
    let idle = state.idle.clone();
    drop(tauri::async_runtime::spawn(async move {
        let _metric = tracing::info_span!(
            target: "koharu_metrics",
            "agent_run",
            provider = "codex",
            character_count = prompt.chars().count(),
        );
        let publish_control = control.clone();
        let Some(agent) = agent.get() else {
            runs.lock().remove(&run);
            idle.notify_waiters();
            return;
        };
        let result = agent
            .run(run, prompt, control, |event| {
                if on_event.send(event).is_err() {
                    publish_control.cancel();
                }
            })
            .await;
        if let Err(error) = result {
            tracing::error!(%run, error = ?error, "agent request failed");
        }
        runs.lock().remove(&run);
        idle.notify_waiters();
    }));
    Ok(run)
}

#[tracing::instrument(
    target = "koharu_metrics",
    name = "agent_cancel",
    skip_all,
    fields(provider = "codex", state = "requested")
)]
#[koharu_macros::command]
pub(crate) async fn cancel_agent(run: RunId, state: AgentState) -> std::result::Result<(), Error> {
    state
        .runs
        .lock()
        .get(&run)
        .with_context(|| format!("agent run {run} is not active"))?
        .cancel();
    Ok(())
}
