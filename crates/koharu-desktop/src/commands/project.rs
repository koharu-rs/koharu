//! The projects folder and the open project.

use std::sync::Arc;

use tauri::State;

use super::Result;
use crate::app::App;

#[tauri::command]
#[specta::specta]
pub fn list_projects(app: State<'_, Arc<App>>) -> Result<Vec<String>> {
    Ok(app.library.list()?)
}

/// Creates a project and opens it.
#[tauri::command]
#[specta::specta]
pub async fn create(app: State<'_, Arc<App>>, name: String) -> Result<()> {
    let app = Arc::clone(&app);
    Ok(tokio::task::spawn_blocking(move || {
        let session = app.library.create(&name)?;
        app.open(name, session)
    })
    .await??)
}

#[tauri::command]
#[specta::specta]
pub async fn open(app: State<'_, Arc<App>>, name: String) -> Result<()> {
    let app = Arc::clone(&app);
    Ok(tokio::task::spawn_blocking(move || {
        let session = app.library.open(&name)?;
        app.open(name, session)
    })
    .await??)
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
pub fn close(app: State<'_, Arc<App>>) {
    app.close();
}

/// The open project's name, for an interface that loaded after it opened.
#[tauri::command]
#[specta::specta]
pub fn get_open_project(app: State<'_, Arc<App>>) -> Option<String> {
    app.project
        .lock()
        .as_ref()
        .map(|project| project.name.clone())
}
