//! Undo, redo, and the History panel.

use std::sync::Arc;

use tauri::State;

use super::{Result, blocking};
use crate::app::App;

#[tauri::command]
#[specta::specta]
pub async fn undo(app: State<'_, Arc<App>>) -> Result<()> {
    let app = Arc::clone(&app);
    blocking(move || {
        app.session()?.lock().undo()?;
        app.changed()
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn redo(app: State<'_, Arc<App>>) -> Result<()> {
    let app = Arc::clone(&app);
    blocking(move || {
        app.session()?.lock().redo()?;
        app.changed()
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
        app.changed()
    })
    .await
}
