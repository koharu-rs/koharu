//! The open project through the scene's own protocol: the reads and edits the
//! agent and MCP make, and the History panel's steps. Each edit is one undo
//! step, labelled by a key the interface translates.

use std::sync::Arc;

use koharu_scene::{Answer, Change, Edit, Query};
use tauri::State;

use super::{Result, blocking};
use crate::app::App;

#[tauri::command]
#[specta::specta]
pub async fn query(app: State<'_, Arc<App>>, query: Query) -> Result<Answer> {
    let app = Arc::clone(&app);
    blocking(move || Ok(app.session()?.lock().query(query)?)).await
}

#[tauri::command]
#[specta::specta]
pub async fn edit(app: State<'_, Arc<App>>, edit: Edit) -> Result<Change> {
    let app = Arc::clone(&app);
    blocking(move || app.edit(edit)).await
}

#[tauri::command]
#[specta::specta]
pub async fn undo(app: State<'_, Arc<App>>) -> Result<()> {
    let app = Arc::clone(&app);
    blocking(move || {
        app.session()?.lock().undo()?;
        Ok(())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn redo(app: State<'_, Arc<App>>) -> Result<()> {
    let app = Arc::clone(&app);
    blocking(move || {
        app.session()?.lock().redo()?;
        Ok(())
    })
    .await
}

/// Goes to the project as it was after `step` steps, as a click in
/// Photoshop's History panel does; 0 is the project as it opened.
#[tauri::command]
#[specta::specta]
pub async fn go_to_step(app: State<'_, Arc<App>>, step: usize) -> Result<()> {
    let app = Arc::clone(&app);
    blocking(move || {
        app.session()?.lock().go_to(step)?;
        Ok(())
    })
    .await
}
