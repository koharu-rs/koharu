//! The pipeline's runs on the open project.

use std::sync::Arc;

use koharu_pipeline::{RunId, Stage, Target};
use tauri::State;

use super::Result;
use crate::app::App;

/// Runs `stages` on `target`; each run's progress goes to the job channel.
/// `trace` is the caller's `sentry-trace` value, which the run continues.
#[tauri::command]
#[specta::specta]
pub fn process(
    app: State<'_, Arc<App>>,
    target: Target,
    stages: Vec<Stage>,
    trace: Option<String>,
) -> Result<RunId> {
    Ok(app.process(target, stages.into_iter().collect(), trace)?)
}

/// Stops every run, running and queued; what they finished stays.
#[tauri::command]
#[specta::specta]
pub fn stop(app: State<'_, Arc<App>>) {
    app.pipeline.stop();
}
