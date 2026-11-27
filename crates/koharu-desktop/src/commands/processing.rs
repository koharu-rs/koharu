//! The pipeline's runs on the open project.

use std::{collections::BTreeSet, sync::Arc};

use koharu_pipeline::{Area, Event, Inpainter, PipelineConfig, RunId, RunPriority, Stage, Target};
use koharu_scene::PageId;
use tauri::State;

use super::Result;
use crate::app::App;

/// Runs `stages` on `target` at `priority` (queued normally when `None`);
/// each run's events go out as `pipeline`.
#[tauri::command]
#[specta::specta]
pub fn run_pipeline(
    app: State<'_, Arc<App>>,
    target: Target,
    stages: Vec<Stage>,
    priority: Option<RunPriority>,
) -> Result<RunId> {
    Ok(app.process(
        target,
        stages.into_iter().collect(),
        None,
        priority.unwrap_or_default(),
    )?)
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
        RunPriority::RerunNow,
    )?))
}

/// Cancels a single queued/running job `(run, page, stage)` when both `run`
/// and `stage` are given, an entire run when `run` is `Some` and `stage` is
/// `None`, or every run when `run` is `None`; what they finished stays.
#[tauri::command]
#[specta::specta]
pub fn cancel_run(
    app: State<'_, Arc<App>>,
    run: Option<RunId>,
    page: Option<PageId>,
    stage: Option<Stage>,
) {
    match (run, stage) {
        (Some(run), Some(stage)) => app.pipeline.cancel_job(run, page, stage),
        (run, None) => app.pipeline.cancel(run),
        (None, Some(_)) => app.pipeline.cancel(None),
    }
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
