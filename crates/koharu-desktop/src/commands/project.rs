//! The projects folder and the open project.

use std::sync::Arc;

use koharu_runtime::Transfer;
use koharu_scene::{Page, PageId};
use serde::Serialize;
use specta::Type;
use tauri::{State, ipc::Channel};

use super::{Result, blocking};
use crate::{
    app::{App, ProjectInfo},
    events::CanvasEvent,
    library::ProjectSummary,
    runs::Job,
};

/// What the interface needs on load, before any event arrives.
#[derive(Clone, Debug, Serialize, Type)]
pub struct StartupState {
    pub project: Option<ProjectInfo>,
    /// The pipeline's runs so far.
    pub jobs: Vec<Job>,
}

/// Hands over the channels the desktop's own events go through.
#[tauri::command]
#[specta::specta]
pub fn subscribe(
    app: State<'_, Arc<App>>,
    on_project: Channel<Option<ProjectInfo>>,
    on_canvas: Channel<CanvasEvent>,
    on_job: Channel<Job>,
    on_download: Channel<Transfer>,
) -> StartupState {
    app.channels
        .subscribe(on_project, on_canvas, on_job, on_download);
    StartupState {
        project: app.info(),
        jobs: app.runs.jobs(),
    }
}

#[tauri::command]
#[specta::specta]
pub fn list_projects(app: State<'_, Arc<App>>) -> Result<Vec<ProjectSummary>> {
    Ok(app.library.list()?)
}

/// Creates a project and opens it.
#[tauri::command]
#[specta::specta]
pub async fn create_project(app: State<'_, Arc<App>>, name: String) -> Result<()> {
    let app = Arc::clone(&app);
    blocking(move || {
        let session = app.library.create(&name)?;
        app.open(name, session)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn open_project(app: State<'_, Arc<App>>, name: String) -> Result<()> {
    let app = Arc::clone(&app);
    blocking(move || {
        let session = app.library.open(&name)?;
        app.open(name, session)
    })
    .await
}

/// Deletes a project, closing it first if it's the open one.
#[tauri::command]
#[specta::specta]
pub fn delete_project(app: State<'_, Arc<App>>, name: String) -> Result<()> {
    if app
        .project
        .lock()
        .as_ref()
        .is_some_and(|project| project.name == name)
    {
        app.close();
    }
    Ok(app.library.delete(&name)?)
}

#[tauri::command]
#[specta::specta]
pub fn close_project(app: State<'_, Arc<App>>) {
    app.close();
}

#[tauri::command]
#[specta::specta]
pub fn get_project(app: State<'_, Arc<App>>) -> Option<ProjectInfo> {
    app.info()
}

/// The page the editor shows, with its layers.
#[tauri::command]
#[specta::specta]
pub fn get_page(app: State<'_, Arc<App>>) -> Result<Option<Page>> {
    let project = app.project.lock();
    let Some(project) = project.as_ref() else {
        return Ok(None);
    };
    let Some(page) = project.page else {
        return Ok(None);
    };
    Ok(Some(project.session.lock().project().page(page)?.clone()))
}

#[tauri::command]
#[specta::specta]
pub async fn select_page(app: State<'_, Arc<App>>, page: PageId) -> Result<()> {
    let app = Arc::clone(&app);
    blocking(move || app.select_page(page)).await
}
