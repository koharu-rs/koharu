//! The agent panel: the in-app agent's runs on the open project, its
//! settings, and the MCP server serving its tools to outside agents.

use std::sync::Arc;

use anyhow::Context as _;
use koharu_agent::AgentConfig;
use serde::Serialize;
use specta::Type;
use tauri::{State, ipc::Channel};

use super::Result;
use crate::{app::App, runs::AgentEvent};

#[derive(Clone, Debug, Serialize, Type)]
pub struct AgentStatus {
    pub config: AgentConfig,
    /// The run going on.
    pub run: Option<u64>,
    /// The MCP server's URL while it runs.
    pub mcp: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub fn get_agent_status(app: State<'_, Arc<App>>) -> Result<AgentStatus> {
    Ok(AgentStatus {
        config: koharu_config::get()?,
        run: app.agent_run(),
        mcp: app.mcp(),
    })
}

/// Saves `[agent]`; the next run takes it.
#[tauri::command]
#[specta::specta]
pub fn save_agent_config(config: AgentConfig) -> Result<AgentConfig> {
    koharu_config::update(|section: &mut AgentConfig| *section = config)?;
    Ok(koharu_config::get()?)
}

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
    ))
}

#[tauri::command]
#[specta::specta]
pub fn cancel_agent(app: State<'_, Arc<App>>, run: u64) {
    app.cancel_agent(run);
}

/// Starts or stops serving the agent's tools over MCP, and answers the URL
/// while it serves.
#[tauri::command]
#[specta::specta]
pub async fn set_mcp(app: State<'_, Arc<App>>, enabled: bool) -> Result<Option<String>> {
    Ok(app.set_mcp(enabled).await?)
}
