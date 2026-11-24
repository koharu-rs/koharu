//! The pipeline's runs on the open project.

use std::{collections::BTreeSet, sync::Arc};

use koharu_pipeline::{Area, Event, Inpainter, PipelineConfig, RunId, Stage, Target};
use tauri::State;

use super::Result;
use crate::app::App;

/// Runs `stages` on `target`; each run's events go out as `pipeline`.
#[tauri::command]
#[specta::specta]
pub fn run_pipeline(app: State<'_, Arc<App>>, target: Target, stages: Vec<Stage>) -> Result<RunId> {
    Ok(app.process(target, stages.into_iter().collect(), None)?)
}

/// Fills the area the marquee and the lasso marked with `model` after
/// `prompt`, or the inpainter's own prompt when there's none, as Photoshop's
/// Generative Fill does with the model its bar shows; `None` while no area is
/// marked. The model is this run's only, in the settings' place.
#[tauri::command]
#[specta::specta]
pub async fn fill(
    app: State<'_, Arc<App>>,
    prompt: Option<String>,
    model: Inpainter,
) -> Result<Option<RunId>> {
    tracing::info!(prompted = prompt.is_some(), %model, "fill");
    let Some((page, rect, pixels)) = app.screen.area().await.ok().flatten() else {
        return Ok(None);
    };
    let mut config = koharu_config::get::<PipelineConfig>()?;
    config.inpainter.model = model;
    let target = Target::Area {
        page,
        area: Area { rect, pixels },
        prompt,
        layer: None,
    };
    Ok(Some(app.process(
        target,
        BTreeSet::from([Stage::Inpaint]),
        Some(config),
    )?))
}

/// Stops every run, running and queued; what they finished stays.
#[tauri::command]
#[specta::specta]
pub fn stop_pipeline(app: State<'_, Arc<App>>) {
    app.pipeline.stop();
}

/// Unloads every model the pipeline loaded once the GPU's current call ends,
/// freeing its memory as Photoshop's Edit → Purge frees memory; each loads
/// again when a stage next needs it.
#[tauri::command]
#[specta::specta]
pub fn unload(app: State<'_, Arc<App>>) {
    tracing::info!("unload");
    app.pipeline.unload();
}

/// What every run told so far, for an interface that loaded late.
#[tauri::command]
#[specta::specta]
pub fn get_pipeline_events(app: State<'_, Arc<App>>) -> Vec<Event> {
    app.pipeline.told()
}
