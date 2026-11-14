//! The agent panel: the in-app agent's runs on the open project. Its
//! settings are the preferences' `[agent]`.

use std::sync::Arc;

use anyhow::Context as _;
use koharu_agent::AgentConfig;
use tauri::{State, ipc::Channel};

use super::Result;
use crate::{app::App, runs::AgentEvent};

/// Runs `prompt` on the open project, in its conversation, and answers the
/// run's id at once. `trace` is the caller's `sentry-trace` value, which the
/// run continues.
#[tauri::command]
#[specta::specta]
pub async fn run_agent(
    app: State<'_, Arc<App>>,
    prompt: String,
    on_event: Channel<AgentEvent>,
    trace: Option<String>,
) -> Result<u64> {
    let config: AgentConfig = koharu_config::get()?;
    let model = config
        .model
        .clone()
        .context("no agent model is set in the preferences")?;
    Ok(app.run_agent(
        &model,
        config,
        prompt,
        move |event| {
            let _ = on_event.send(event);
        },
        trace,
    )?)
}

#[tauri::command]
#[specta::specta]
pub fn cancel_agent(app: State<'_, Arc<App>>, run: u64) {
    app.cancel_agent(run);
}
