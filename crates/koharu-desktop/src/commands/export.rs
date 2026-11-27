//! Export: every page, or some of them, as PNG, JPEG, WebP, JPEG XL, or PSD
//! files in a folder, as a CBZ of images, or as a PDF.

use std::sync::Arc;

use anyhow::Context as _;
use koharu_io::ExportFormat;
use koharu_scene::PageId;
use tauri::{State, Window};
use tauri_runtime_cef::CefRuntime;

use super::Result;
use crate::app::App;

/// Picks where the open project's pages go, and writes them there: `pages`
/// alone, or every page when it's `None`, in the project's order.
#[tauri::command]
#[specta::specta]
pub async fn export(
    app: State<'_, Arc<App>>,
    window: Window<CefRuntime>,
    pages: Option<Vec<PageId>>,
    format: ExportFormat,
) -> Result<()> {
    let name = app
        .project
        .lock()
        .as_ref()
        .map(|project| project.name.clone())
        .context("no project is open")?;
    let dialog = super::file_dialog(&window);
    let extension = match format {
        ExportFormat::Images(_) | ExportFormat::Psd(_) => None,
        ExportFormat::Cbz(_) => Some("cbz"),
        ExportFormat::Pdf => Some("pdf"),
    };
    let destination = match extension {
        None => dialog.pick_folder().await,
        Some(extension) => {
            dialog
                .set_file_name(format!("{name}.{extension}"))
                .add_filter(extension.to_uppercase(), &[extension])
                .save_file()
                .await
        }
    };
    let Some(destination) = destination.map(|picked| picked.path().to_owned()) else {
        return Ok(());
    };
    let app = Arc::clone(&app);
    Ok(tokio::task::spawn_blocking(move || {
        let session = app.session()?;
        let pipeline = koharu_config::get::<koharu_pipeline::PipelineConfig>()?;
        koharu_io::export(
            &session,
            &app.renderer,
            pages.as_deref(),
            format,
            Some(&pipeline.translator.target),
            &destination,
        )
    })
    .await??)
}
