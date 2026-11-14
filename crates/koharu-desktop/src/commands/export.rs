//! Export: every page as PNG or PSD files in a folder, or as a CBZ of PNGs.

use std::{
    io::{Cursor, Write as _},
    path::Path,
    sync::Arc,
};

use anyhow::Context as _;
use fs_err::File;
use image::ImageFormat;
use koharu_psd::Labels;
use koharu_renderer::{Frame, Renderer};
use koharu_scene::PageId;
use serde::Deserialize;
use specta::Type;
use tauri::{State, Window};
use tauri_runtime_cef::CefRuntime;

use super::Result;
use crate::app::App;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Png,
    Psd,
    Cbz,
}

/// Picks where the open project's pages go, and writes them there. `labels`
/// names a PSD's layers the scene names nothing, in the interface's language.
#[tauri::command]
#[specta::specta]
pub async fn export(
    app: State<'_, Arc<App>>,
    window: Window<CefRuntime>,
    format: ExportFormat,
    labels: Labels,
) -> Result<()> {
    let name = app
        .project
        .lock()
        .as_ref()
        .map(|project| project.name.clone())
        .context("no project is open")?;
    let dialog = rfd::AsyncFileDialog::new().set_parent(&window);
    let destination = match format {
        ExportFormat::Png | ExportFormat::Psd => dialog.pick_folder().await,
        ExportFormat::Cbz => {
            dialog
                .set_file_name(format!("{name}.cbz"))
                .add_filter("CBZ", &["cbz"])
                .save_file()
                .await
        }
    };
    let Some(destination) = destination.map(|picked| picked.path().to_owned()) else {
        return Ok(());
    };
    let app = Arc::clone(&app);
    Ok(tokio::task::spawn_blocking(move || write(&app, format, &labels, &destination)).await??)
}

fn write(
    app: &App,
    format: ExportFormat,
    labels: &Labels,
    destination: &Path,
) -> anyhow::Result<()> {
    let session = app.session()?;
    let pages: Vec<PageId> = session
        .lock()
        .project()
        .pages()
        .map(|page| page.id)
        .collect();
    let digits = pages.len().to_string().len().max(3);
    let mut archive = match format {
        ExportFormat::Cbz => Some(::zip::ZipWriter::new(File::create(destination)?)),
        ExportFormat::Png | ExportFormat::Psd => None,
    };
    for (index, &page) in pages.iter().enumerate() {
        let number = format!("{:0digits$}", index + 1);
        // The page as it finally looks, its bundled fonts in.
        let frame =
            tauri::async_runtime::block_on(app.renderer.render_loaded(Arc::clone(&session), page))?;
        match (format, &mut archive) {
            (ExportFormat::Psd, _) => {
                let psd = koharu_psd::export(&session.lock(), &app.renderer, page, labels)?;
                fs_err::write(destination.join(format!("{number}.psd")), psd)?;
            }
            (_, Some(archive)) => {
                // Already-compressed PNGs gain nothing from deflate.
                let options = ::zip::write::SimpleFileOptions::default()
                    .compression_method(::zip::CompressionMethod::Stored);
                archive.start_file(format!("{number}.png"), options)?;
                archive.write_all(&png(&app.renderer, &frame)?)?;
            }
            (_, None) => {
                fs_err::write(
                    destination.join(format!("{number}.png")),
                    png(&app.renderer, &frame)?,
                )?;
            }
        }
    }
    if let Some(archive) = archive {
        archive.finish()?;
    }
    Ok(())
}

fn png(renderer: &Renderer, frame: &Frame) -> anyhow::Result<Vec<u8>> {
    let raster = renderer.rasterize(frame)?;
    let mut png = Cursor::new(Vec::new());
    raster.image.write_to(&mut png, ImageFormat::Png)?;
    Ok(png.into_inner())
}
