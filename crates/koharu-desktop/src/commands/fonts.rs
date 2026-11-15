//! The fonts the Inspector offers. The bundled ones' catalog and faces are
//! downloaded on first use.

use koharu_renderer::fonts::{self, FontFamily};

use super::{Bytes, Result};

#[tauri::command]
#[specta::specta]
pub async fn list_fonts() -> Result<Vec<FontFamily>> {
    Ok(fonts::list().await?)
}

/// `family`'s name set in it, as WebP.
#[tauri::command]
#[specta::specta]
pub async fn get_font_preview(family: String) -> Result<Bytes> {
    Ok(Bytes(fonts::preview(&family).await?))
}
