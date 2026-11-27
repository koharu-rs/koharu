//! Export: every page, or some of them, as PNG, JPEG, WebP, or PSD files in a
//! folder, as a CBZ of images, or as a PDF.

use std::{io::Write as _, path::Path, sync::Arc};

use anyhow::{Context as _, anyhow, ensure};
use fs_err::File;
use image::{
    DynamicImage, Rgba, RgbaImage,
    codecs::{
        jpeg::JpegEncoder,
        png::{CompressionType, FilterType, PngEncoder},
    },
    imageops,
};
use koharu_psd::Labels;
use koharu_scene::PageId;
use krilla::{Document, geom::Size, image::Image, page::PageSettings};
use serde::Deserialize;
use specta::Type;
use tauri::{State, Window};
use tauri_runtime_cef::CefRuntime;

use super::{Result, import::PDF_SCALE};
use crate::app::App;

/// What the pages go out as: a folder of images or of PSDs, a CBZ of
/// images, or a PDF.
#[derive(Clone, Debug, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Images(ImageFormat),
    /// With the names of the layers the scene names nothing, in the
    /// interface's language.
    Psd(Labels),
    Cbz(ImageFormat),
    Pdf,
}

/// How a flattened page is written: PNG's without loss, JPEG's and WebP's at
/// a quality from 1 to 100.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    Png,
    Jpeg { quality: u8 },
    Webp { quality: u8 },
}

impl ImageFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg { .. } => "jpg",
            Self::Webp { .. } => "webp",
        }
    }

    fn encode(self, image: RgbaImage) -> anyhow::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        match self {
            // The png crate's own default compression: image's, the fastest,
            // leaves a page in near-grays twice the size.
            Self::Png => {
                let encoder = PngEncoder::new_with_quality(
                    &mut bytes,
                    CompressionType::Default,
                    FilterType::Adaptive,
                );
                if image.pixels().all(|pixel| pixel[3] == u8::MAX) {
                    opaque(image).write_with_encoder(encoder)?
                } else {
                    image.write_with_encoder(encoder)?
                }
            }
            // JPEG has no transparency, so the page is drawn over white, as
            // Photoshop's JPEG export does. Overlay's rounding can leave that
            // white's alpha at 254.
            Self::Jpeg { quality } => {
                let mut white =
                    RgbaImage::from_pixel(image.width(), image.height(), Rgba([255; 4]));
                imageops::overlay(&mut white, &image, 0, 0);
                opaque(white)
                    .write_with_encoder(JpegEncoder::new_with_quality(&mut bytes, quality))?
            }
            // libwebp leaves out the alpha of a page with no transparency.
            Self::Webp { quality } => {
                let (width, height) = image.dimensions();
                // libwebp's `WEBP_MAX_DIMENSION`.
                ensure!(
                    width.max(height) <= 16383,
                    "a WebP image is at most 16383 pixels a side, and the page is {width}×{height}"
                );
                bytes = webp::Encoder::from_rgba(image.as_raw(), width, height)
                    .encode_simple(false, quality.into())
                    .map_err(|error| anyhow!("libwebp failed with {error:?}"))?
                    .to_vec();
            }
        }
        Ok(bytes)
    }
}

/// An opaque `image`, its alpha dropped unread, in the fewest channels that
/// hold it exactly: gray when every pixel is a gray, RGB otherwise.
fn opaque(image: RgbaImage) -> DynamicImage {
    let gray = image.pixels().all(|Rgba([r, g, b, _])| r == g && g == b);
    let image = DynamicImage::ImageRgba8(image);
    if gray {
        DynamicImage::ImageLuma8(image.to_luma8())
    } else {
        DynamicImage::ImageRgb8(image.to_rgb8())
    }
}

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
    let dialog = rfd::AsyncFileDialog::new().set_parent(&window);
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
    Ok(
        tokio::task::spawn_blocking(move || write(&app, pages.as_deref(), format, &destination))
            .await??,
    )
}

fn write(
    app: &App,
    pages: Option<&[PageId]>,
    format: ExportFormat,
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
        ExportFormat::Images(format) => {
            for (number, page) in pages {
                fs_err::write(
                    destination.join(format!("{number}.{}", format.extension())),
                    format.encode(draw(page)?)?,
                )?;
            }
        }
        ExportFormat::Psd(labels) => {
            for (number, page) in pages {
                let psd = koharu_psd::export(&session.lock(), &app.renderer, page, &labels)?;
                fs_err::write(destination.join(format!("{number}.psd")), psd)?;
            }
        }
        ExportFormat::Cbz(format) => {
            let mut archive = ::zip::ZipWriter::new(File::create(destination)?);
            // Already-compressed images gain nothing from deflate.
            let options = ::zip::write::SimpleFileOptions::default()
                .compression_method(::zip::CompressionMethod::Stored);
            for (number, page) in pages {
                archive.start_file(format!("{number}.{}", format.extension()), options)?;
                archive.write_all(&format.encode(draw(page)?)?)?;
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
