//! History panel commands: timeline jumps and history clearing.

use anyhow::Context as _;
use koharu_desktop::Desktop;
use tauri::State;

use super::{
    ChannelExt as _, Error, canvas::CanvasChannel, editing::synchronize_canvas,
    lifecycle::ProjectChannel, project::CurrentProject,
};

#[tracing::instrument(
    target = "koharu_metrics",
    name = "history_jump",
    skip_all,
    fields(origin = "user", target = index)
)]
#[tauri::command]
#[specta::specta]
pub(crate) async fn history_go_to(
    index: u32,
    desktop: State<'_, Desktop>,
    project: State<'_, CurrentProject>,
    canvas_channel: State<'_, CanvasChannel>,
    project_channel: State<'_, ProjectChannel>,
) -> Result<(), Error> {
    let updated = {
        let mut project = project.project.lock().await;
        let project = project.as_mut().context("no project is open")?;
        match project.jump_to(index as usize).await? {
            Some(commit) => (commit, project.active_page(), project.info()),
            // The cursor is already on the requested state.
            None => return Ok(()),
        }
    };
    let (commit, page, info) = updated;
    let canvas = synchronize_canvas(&desktop, &commit, page).await?;
    canvas_channel.channel.publish(canvas);
    project_channel.channel.publish(Some(info));
    Ok(())
}

#[tracing::instrument(
    target = "koharu_metrics",
    name = "history_cleared",
    skip_all,
    fields(origin = "user")
)]
#[tauri::command]
#[specta::specta]
pub(crate) async fn history_clear(
    project: State<'_, CurrentProject>,
    project_channel: State<'_, ProjectChannel>,
) -> Result<(), Error> {
    let info = {
        let mut project = project.project.lock().await;
        let project = project.as_mut().context("no project is open")?;
        project.clear_history();
        project.info()
    };
    project_channel.channel.publish(Some(info));
    Ok(())
}

#[tracing::instrument(
    target = "koharu_metrics",
    name = "snapshot_created",
    skip_all,
    fields(origin = "user")
)]
#[tauri::command]
#[specta::specta]
pub(crate) async fn snapshot_create(
    name: Option<String>,
    project: State<'_, CurrentProject>,
    project_channel: State<'_, ProjectChannel>,
) -> Result<(), Error> {
    let info = {
        let mut project = project.project.lock().await;
        let project = project.as_mut().context("no project is open")?;
        project.create_snapshot(name);
        project.info()
    };
    project_channel.channel.publish(Some(info));
    Ok(())
}

#[tracing::instrument(
    target = "koharu_metrics",
    name = "snapshot_restored",
    skip_all,
    fields(origin = "user", snapshot = id)
)]
#[tauri::command]
#[specta::specta]
pub(crate) async fn snapshot_restore(
    id: u32,
    desktop: State<'_, Desktop>,
    project: State<'_, CurrentProject>,
    canvas_channel: State<'_, CanvasChannel>,
    project_channel: State<'_, ProjectChannel>,
) -> Result<(), Error> {
    let (commit, page, info) = {
        let mut project = project.project.lock().await;
        let project = project.as_mut().context("no project is open")?;
        let commit = project.restore_snapshot(id).await?;
        (commit, project.active_page(), project.info())
    };
    let canvas = synchronize_canvas(&desktop, &commit, page).await?;
    canvas_channel.channel.publish(canvas);
    project_channel.channel.publish(Some(info));
    Ok(())
}

#[tracing::instrument(
    target = "koharu_metrics",
    name = "snapshot_deleted",
    skip_all,
    fields(origin = "user", snapshot = id)
)]
#[tauri::command]
#[specta::specta]
pub(crate) async fn snapshot_delete(
    id: u32,
    project: State<'_, CurrentProject>,
    project_channel: State<'_, ProjectChannel>,
) -> Result<(), Error> {
    let info = {
        let mut project = project.project.lock().await;
        let project = project.as_mut().context("no project is open")?;
        project.delete_snapshot(id);
        project.info()
    };
    project_channel.channel.publish(Some(info));
    Ok(())
}
