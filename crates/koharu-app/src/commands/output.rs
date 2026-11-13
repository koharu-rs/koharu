use anyhow::{Context as _, Result};
use futures::{StreamExt as _, TryStreamExt as _, stream};
use image::{
    ExtendedColorType, ImageEncoder as _,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};
use koharu_psd::{PsdExportOptions, export_page};
use koharu_rasterizer::{Raster, RasterOptions, Rasterizer};
use koharu_renderer::{Frame, Renderer};
use koharu_scene::{AssetRole, EntityId, Snapshot};
use serde::Deserialize;
use specta::Type;
use std::{io::Write as _, sync::Arc};
use tauri::{State, WebviewWindow, ipc::IpcResponse};
use tauri_runtime_cef::CefRuntime;

use super::{Error, project::CurrentProject};
use koharu_desktop::Desktop;

const THUMBNAIL_EDGE: u32 = 128;

#[derive(Type)]
#[specta(transparent)]
pub(crate) struct ThumbnailBytes(#[specta(type = Vec<u8>)] Vec<u8>);

impl IpcResponse for ThumbnailBytes {
    fn body(self) -> tauri::Result<tauri::ipc::InvokeResponseBody> {
        Ok(self.0.into())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Png,
    Psd,
    Cbz,
}

/// Pages prepared at once during an export. Each one holds a full-resolution
/// frame and its encoded file, so preparing a whole long project together
/// exhausts memory.
const EXPORT_CONCURRENCY: usize = 4;

#[tauri::command]
#[specta::specta]
pub(crate) async fn export(
    window: WebviewWindow<CefRuntime>,
    format: ExportFormat,
    project: State<'_, CurrentProject>,
    desktop: State<'_, Desktop>,
) -> std::result::Result<(), Error> {
    let (name, snapshot) = {
        let project = project.project.lock().await;
        let project = project.as_ref().context("no project is open")?;
        (project.name.clone(), project.snapshot())
    };
    let pages = snapshot.pages().map(|page| page.id()).collect::<Vec<_>>();
    if pages.is_empty() {
        return Err(anyhow::anyhow!("there are no pages to export").into());
    }
    let dialog = rfd::AsyncFileDialog::new().set_parent(&window);
    let destination = match format {
        ExportFormat::Png | ExportFormat::Psd => dialog.pick_folder().await,
        ExportFormat::Cbz => {
            dialog
                .add_filter("Comic Book Archive", &["cbz"])
                .set_file_name(format!("{name}.cbz"))
                .save_file()
                .await
        }
    };
    let Some(destination) = destination.map(|destination| destination.path().to_owned()) else {
        return Ok(());
    };
    let renderer = desktop.renderer();
    let rasterizer = desktop.rasterizer().await?;
    let options = PsdExportOptions::default();
    let extension = match format {
        ExportFormat::Png | ExportFormat::Cbz => "png",
        ExportFormat::Psd => "psd",
    };
    let mut archive = if matches!(format, ExportFormat::Cbz) {
        let directory = destination.parent().context("archive path has no parent")?;
        Some(zip::ZipWriter::new(tempfile::NamedTempFile::new_in(
            directory,
        )?))
    } else {
        None
    };
    let width = pages.len().to_string().len().max(4);
    // `buffered` yields in project order while bounding the pages in flight.
    let mut encoded = stream::iter(pages.iter().copied())
        .map(|page| {
            let (renderer, snapshot, options) = (&renderer, &snapshot, &options);
            let rasterizer = Arc::clone(&rasterizer);
            async move {
                let frame = renderer.render(snapshot, page).await?;
                match format {
                    ExportFormat::Png | ExportFormat::Cbz => {
                        tokio_rayon::spawn(move || encode_png(&rasterizer, &frame)).await
                    }
                    ExportFormat::Psd => {
                        Ok(export_page(rasterizer, snapshot, &frame, options).await?)
                    }
                }
            }
        })
        .buffered(EXPORT_CONCURRENCY);
    let mut index = 0;
    while let Some(bytes) = encoded.try_next().await? {
        let page = snapshot.page(pages[index])?.page()?;
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
        if let Some(mut writer) = archive.take() {
            // Archive I/O is synchronous, so it runs off the async workers.
            archive = Some(
                tokio::task::spawn_blocking(move || -> Result<_> {
                    // PNG data is already compressed.
                    let options = zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Stored);
                    writer.start_file(name, options)?;
                    writer.write_all(&bytes)?;
                    Ok(writer)
                })
                .await??,
            );
        } else {
            tokio::fs::write(destination.join(name), bytes).await?;
        }
        index += 1;
    }
    if let Some(writer) = archive {
        tokio::task::spawn_blocking(move || -> Result<()> {
            writer.finish()?.persist(destination)?;
            Ok(())
        })
        .await??;
    }
    Ok(())
}

fn encode_png(rasterizer: &Rasterizer, frame: &Frame) -> Result<Vec<u8>> {
    let image = rasterizer
        .rasterize(&frame.raster_frame()?, RasterOptions::default())?
        .image;
    let mut bytes = Vec::new();
    PngEncoder::new_with_quality(&mut bytes, CompressionType::Best, FilterType::Adaptive)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            ExtendedColorType::Rgba8,
        )?;
    Ok(bytes)
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn get_thumbnail(
    page: EntityId,
    project: State<'_, CurrentProject>,
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
