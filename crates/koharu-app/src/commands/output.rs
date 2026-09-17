use anyhow::{Context as _, Result};
use futures::future::try_join_all;
use image::{
    ExtendedColorType, ImageEncoder as _,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};
use koharu_psd::{PsdExportOptions, export_page};
use koharu_rasterizer::{Raster, RasterOptions, Rasterizer};
use koharu_renderer::{Frame, Renderer};
use koharu_scene::{AssetRole, EntityId, Snapshot};
use rayon::prelude::*;
use serde::Deserialize;
use specta::Type;
use std::{io::Write as _, path::PathBuf, sync::Arc};
use tauri::WebviewWindow;
use tauri_runtime_cef::CefRuntime;

use super::{Error, project::CurrentProject};
use koharu_desktop::Desktop;

const THUMBNAIL_EDGE: u32 = 128;

#[derive(Type)]
#[specta(transparent)]
pub(crate) struct ThumbnailBytes(#[specta(type = Vec<u8>)] Vec<u8>);

impl From<ThumbnailBytes> for Vec<u8> {
    fn from(value: ThumbnailBytes) -> Self {
        value.0
    }
}

pub(crate) struct ExportBytes {
    bytes: Vec<u8>,
    content_type: &'static str,
    filename: String,
}

impl From<ExportBytes> for crate::host::HttpFile {
    fn from(value: ExportBytes) -> Self {
        crate::host::HttpFile {
            bytes: value.bytes,
            content_type: value.content_type,
            filename: Some(value.filename),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Png,
    Psd,
    Cbz,
}

#[tracing::instrument(
    target = "koharu_metrics",
    name = "export",
    skip_all,
    fields(origin = "user", format = ?format),
)]
#[koharu_macros::command]
pub(crate) async fn export(
    window: WebviewWindow<CefRuntime>,
    format: ExportFormat,
    destination: Option<String>,
    project: CurrentProject,
    desktop: Desktop,
) -> std::result::Result<Option<ExportBytes>, Error> {
    let (name, snapshot) = {
        let project = project.project.lock().await;
        let project = project.as_ref().context("no project is open")?;
        (project.name.clone(), project.snapshot())
    };
    let pages = snapshot.pages().map(|page| page.id()).collect::<Vec<_>>();
    if pages.is_empty() {
        return Err(anyhow::anyhow!("there are no pages to export").into());
    }
    let destination = match destination {
        Some(destination) => Some(PathBuf::from(destination)),
        None => match window {
            Some(window) => {
                let dialog = rfd::AsyncFileDialog::new().set_parent(&window);
                let picked = match format {
                    ExportFormat::Png | ExportFormat::Psd => dialog.pick_folder().await,
                    ExportFormat::Cbz => {
                        dialog
                            .add_filter("Comic Book Archive", &["cbz"])
                            .set_file_name(format!("{name}.cbz"))
                            .save_file()
                            .await
                    }
                };
                let Some(destination) = picked.map(|destination| destination.path().to_owned())
                else {
                    return Ok(None);
                };
                Some(destination)
            }
            None => None,
        },
    };
    let renderer = desktop.renderer();
    let rasterizer = desktop.rasterizer().await?;
    let frames = try_join_all(pages.iter().map(|&page| renderer.render(&snapshot, page))).await?;
    let (extension, images) = match format {
        ExportFormat::Png | ExportFormat::Cbz => {
            let images = tokio_rayon::spawn(move || {
                frames
                    .par_iter()
                    .map(|frame| -> Result<_> {
                        let image = rasterizer
                            .rasterize(&frame.raster_frame()?, RasterOptions::default())?
                            .image;
                        let mut bytes = Vec::new();
                        PngEncoder::new_with_quality(
                            &mut bytes,
                            CompressionType::Best,
                            FilterType::Adaptive,
                        )
                        .write_image(
                            image.as_raw(),
                            image.width(),
                            image.height(),
                            ExtendedColorType::Rgba8,
                        )?;
                        Ok(bytes)
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .await?;
            ("png", images)
        }
        ExportFormat::Psd => {
            let options = PsdExportOptions::default();
            let images = try_join_all(
                frames
                    .iter()
                    .map(|frame| export_page(Arc::clone(&rasterizer), &snapshot, frame, &options)),
            )
            .await?;
            ("psd", images)
        }
    };
    let entries = export_entries(&snapshot, &pages, &images, extension)?;
    if let Some(destination) = destination {
        tokio_rayon::spawn(move || write_export(format, destination, entries)).await?;
        Ok(None)
    } else {
        Ok(Some(pack_export_download(&name, format, entries)?))
    }
}

fn export_entries(
    snapshot: &koharu_scene::Snapshot,
    pages: &[koharu_scene::EntityId],
    images: &[Vec<u8>],
    extension: &str,
) -> Result<Vec<(String, Vec<u8>)>> {
    let width = pages.len().to_string().len().max(4);
    pages
        .iter()
        .zip(images.iter())
        .enumerate()
        .map(|(index, (page_id, bytes))| {
            let page = snapshot.page(*page_id)?.page()?;
            let name = page
                .label
                .trim()
                .trim_end_matches(|character: char| character == '.' || character.is_whitespace());
            let name = name
                .rsplit_once('.')
                .map_or(name, |(stem, _)| stem)
                .replace(['<', '>', ':', '"', '/', '\\', '|', '?', '*'], "_");
            let name = format!(
                "{:0width$}_{}.{extension}",
                index + 1,
                if name.is_empty() { "page" } else { &name }
            );
            Ok((name, bytes.clone()))
        })
        .collect()
}

fn write_export(
    format: ExportFormat,
    destination: PathBuf,
    entries: Vec<(String, Vec<u8>)>,
) -> Result<()> {
    let mut archive = if matches!(format, ExportFormat::Cbz) {
        let directory = destination.parent().context("archive path has no parent")?;
        Some(zip::ZipWriter::new(tempfile::NamedTempFile::new_in(
            directory,
        )?))
    } else {
        None
    };
    for (name, bytes) in entries {
        if let Some(archive) = &mut archive {
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            archive.start_file(name, options)?;
            archive.write_all(&bytes)?;
        } else {
            std::fs::write(destination.join(name), bytes)?;
        }
        tracing::info!(
            target: "koharu_metrics",
            metric = "page_exported",
            format = ?format,
        );
    }
    if let Some(archive) = archive {
        archive.finish()?.persist(destination)?;
    }
    Ok(())
}

fn pack_export_download(
    name: &str,
    format: ExportFormat,
    entries: Vec<(String, Vec<u8>)>,
) -> Result<ExportBytes> {
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (entry_name, bytes) in entries {
        archive.start_file(entry_name, stored)?;
        archive.write_all(&bytes)?;
        tracing::info!(
            target: "koharu_metrics",
            metric = "page_exported",
            format = ?format,
        );
    }
    let bytes = archive.finish()?.into_inner();
    Ok(match format {
        ExportFormat::Cbz => ExportBytes {
            bytes,
            content_type: "application/vnd.comicbook+zip",
            filename: format!("{name}.cbz"),
        },
        ExportFormat::Png | ExportFormat::Psd => ExportBytes {
            bytes,
            content_type: "application/zip",
            filename: format!("{name}.zip"),
        },
    })
}

#[koharu_macros::command]
pub(crate) async fn get_thumbnail(
    page: EntityId,
    project: CurrentProject,
) -> std::result::Result<ThumbnailBytes, Error> {
    let snapshot = project
        .project
        .lock()
        .await
        .as_ref()
        .context("no project is open")?
        .snapshot();
    snapshot.page(page)?;
    let blob = snapshot
        .asset(page, &AssetRole::new("source")?)?
        .with_context(|| format!("page {page} has no source image"))?
        .blob;
    let bytes = snapshot.read_blob(blob).await?;
    let bytes = tokio_rayon::spawn(move || -> Result<Vec<u8>> {
        let image = image::load_from_memory(&bytes).context("failed to decode source image")?;
        if image.width() == 0 || image.height() == 0 {
            return Err(anyhow::anyhow!("source image is empty"));
        }
        let image = image.thumbnail(THUMBNAIL_EDGE, THUMBNAIL_EDGE).to_rgba8();
        let encoder = webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height());
        Ok(encoder.encode(80.0).to_vec())
    })
    .await?;
    Ok(ThumbnailBytes(bytes))
}

pub(crate) async fn rendered_preview(
    renderer: &Renderer,
    rasterizer: Arc<Rasterizer>,
    snapshot: &Snapshot,
    page: EntityId,
) -> Result<Vec<u8>> {
    snapshot.page(page)?;
    let frame = renderer.render(snapshot, page).await?;
    let image = rasterize(rasterizer, &frame, RasterOptions::default())
        .await?
        .image;
    tokio_rayon::spawn(move || {
        let image = image::DynamicImage::ImageRgba8(image)
            .resize(1024, 1024, image::imageops::FilterType::Lanczos3)
            .to_rgba8();
        let encoder = webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height());
        Ok::<_, anyhow::Error>(encoder.encode(85.0).to_vec())
    })
    .await
}

async fn rasterize(
    rasterizer: Arc<Rasterizer>,
    frame: &Frame,
    options: RasterOptions,
) -> Result<Raster> {
    let frame = frame.raster_frame()?;
    tokio_rayon::spawn(move || rasterizer.rasterize(&frame, options))
        .await
        .map_err(Into::into)
}
