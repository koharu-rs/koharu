//! The fonts the Inspector offers. The bundled ones' catalog and faces are
//! downloaded on first use.

use std::sync::Arc;

use koharu_renderer::FontFamily;
use tauri::State;

use super::{Bytes, Result};
use crate::app::App;

#[tauri::command]
#[specta::specta]
pub async fn get_fonts(app: State<'_, Arc<App>>) -> Result<Vec<FontFamily>> {
    Ok(app.renderer.available_fonts().await?)
}

/// `family`'s name set in it, as WebP.
#[tauri::command]
#[specta::specta]
pub async fn get_font_preview(app: State<'_, Arc<App>>, family: String) -> Result<Bytes> {
    Ok(Bytes(app.renderer.font_preview(&family).await?))
}
