//! Export: every page, or some of them, as PNG or PSD files in a folder, as a
//! CBZ of PNGs, or as a PDF.

use std::{
    io::{Cursor, Write as _},
    path::Path,
    sync::Arc,
};

use anyhow::Context as _;
use fs_err::File;
use image::{ImageFormat, RgbaImage};
use koharu_psd::Labels;
use koharu_scene::PageId;
use krilla::{Document, geom::Size, image::Image, page::PageSettings};
use serde::Deserialize;
use specta::Type;
use tauri::{State, Window};
use tauri_runtime_cef::CefRuntime;

use super::{Result, import::PDF_SCALE};
use crate::app::App;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Png,
    Psd,
    Cbz,
    Pdf,
}

/// Picks where the open project's pages go, and writes them there: `pages`
/// alone, or every page when it's `None`, in the project's order. `labels`
/// names a PSD's layers the scene names nothing, in the interface's language.
#[tauri::command]
#[specta::specta]
pub async fn export(
    app: State<'_, Arc<App>>,
    window: Window<CefRuntime>,
    pages: Option<Vec<PageId>>,
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
    let extension = match format {
        ExportFormat::Png | ExportFormat::Psd => None,
        ExportFormat::Cbz => Some("cbz"),
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
        write(&app, pages.as_deref(), format, &labels, &destination)
    })
    .await??)
}

fn write(
    app: &App,
    pages: Option<&[PageId]>,
    format: ExportFormat,
    labels: &Labels,
    destination: &Path,
) -> anyhow::Result<()> {
    let session = app.session()?;
    let order: Vec<PageId> = session
        .lock()
        .project()
        .pages()
        .map(|page| page.id)
        .collect();
    let digits = order.len().to_string().len().max(3);
    // A page keeps its number in the project whichever pages go with it, so
    // one exported again replaces its own file.
    let pages = order
        .iter()
        .enumerate()
        .filter(|(_, page)| pages.is_none_or(|pages| pages.contains(page)))
        .map(|(index, &page)| (format!("{:0digits$}", index + 1), page));
    let draw = |page| -> anyhow::Result<RgbaImage> {
        let frame = app.renderer.render(&session.lock(), page)?;
        Ok(frame.rasterize()?.image)
    };
    match format {
        ExportFormat::Png => {
            for (number, page) in pages {
                fs_err::write(
                    destination.join(format!("{number}.png")),
                    png(&draw(page)?)?,
                )?;
            }
        }
        ExportFormat::Psd => {
            for (number, page) in pages {
                let psd = koharu_psd::export(&session.lock(), &app.renderer, page, labels)?;
                fs_err::write(destination.join(format!("{number}.psd")), psd)?;
            }
        }
        ExportFormat::Cbz => {
            let mut archive = ::zip::ZipWriter::new(File::create(destination)?);
            // Already-compressed PNGs gain nothing from deflate.
            let options = ::zip::write::SimpleFileOptions::default()
                .compression_method(::zip::CompressionMethod::Stored);
            for (number, page) in pages {
                archive.start_file(format!("{number}.png"), options)?;
                archive.write_all(&png(&draw(page)?)?)?;
            }
            archive.finish()?;
        }
        ExportFormat::Pdf => {
            let mut document = Document::new();
            for (_, page) in pages {
                let image = draw(page)?;
                let (width, height) = image.dimensions();
                // Import's own scale, so a page read back keeps its pixels.
                let size = Size::from_wh(width as f32 / PDF_SCALE, height as f32 / PDF_SCALE)
                    .context("a page has no pixels")?;
                document
                    .start_page_with(PageSettings::new(size))
                    .surface()
                    .draw_image(Image::from_rgba8(image.into_raw(), width, height), size);
            }
            fs_err::write(destination, document.finish()?)?;
        }
    }
    Ok(())
}

fn png(image: &RgbaImage) -> anyhow::Result<Vec<u8>> {
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png)?;
    Ok(png.into_inner())
}
