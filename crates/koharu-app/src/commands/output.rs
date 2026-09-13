use anyhow::{Context as _, Result};
use futures::{StreamExt as _, stream};
use image::{
    ExtendedColorType, ImageEncoder as _, RgbaImage,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};
use koharu_pipeline::StopToken;
use koharu_psd::{PsdExportOptions, export_page};
use koharu_rasterizer::{Raster, RasterOptions, Rasterizer};
use koharu_renderer::{Frame, Renderer};
use koharu_scene::{AssetRole, EntityId, Snapshot};
use serde::Deserialize;
use specta::Type;
use std::{
    io::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::{AppHandle, Manager as _, State, WebviewWindow, ipc::IpcResponse};
use tauri_runtime_cef::CefRuntime;

use super::{
    ChannelExt as _, Error,
    processing::{Job, JobChannel, JobId, JobState, Processing},
    project::CurrentProject,
};
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

/// Where a single export run writes its output.
///
/// Page formats produce one file per page inside a chosen directory; archive
/// formats collect every page into one chosen file.
enum Destination {
    Directory(PathBuf),
    Archive(PathBuf),
}

/// Replaces characters that are invalid in common filesystem names.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|character| {
            if matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            ) {
                '_'
            } else {
                character
            }
        })
        .collect()
}

/// Output file stem for one page, without an extension.
///
/// Archive members follow the comic archive convention of `P001`, `P002`, and
/// are numbered by position and nothing else. A page label is not ordered --
/// pages can be reordered after import -- and usually already carries its own
/// number from the source file, so including it would both misorder the archive
/// and repeat the numbering. Loose files keep the label, where it is what makes
/// a file identifiable on disk.
fn page_stem(format: ExportFormat, index: usize, total: usize, label: &str) -> String {
    let number = index + 1;
    match format {
        ExportFormat::Cbz => {
            // Widen past three digits for a project that needs it, so members
            // still sort correctly. Matches how PDF import numbers its pages.
            let width = total.to_string().len().max(3);
            format!("P{number:0width$}")
        }
        ExportFormat::Png | ExportFormat::Psd => {
            let name = label
                .trim()
                .trim_end_matches(|character: char| character == '.' || character.is_whitespace());
            let name = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
            let name = sanitize_filename(name);
            format!(
                "{number:04}_{}",
                if name.is_empty() { "page" } else { &name }
            )
        }
    }
}

/// Writes received members into a CBZ archive, in the order they arrive.
///
/// `ZipWriter` is a single sequential writer, so one task owns it and pages are
/// handed over through a channel rather than written concurrently.
fn write_cbz(
    path: &Path,
    receiver: &mut tokio::sync::mpsc::Receiver<(String, Vec<u8>)>,
) -> Result<()> {
    let file = std::fs::File::create(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    let mut archive = zip::ZipWriter::new(file);
    // Page images are already PNG-compressed; deflating them again costs time
    // for no meaningful size reduction.
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    while let Some((name, bytes)) = receiver.blocking_recv() {
        archive.start_file(name, options)?;
        archive.write_all(&bytes)?;
    }
    archive.finish()?;
    Ok(())
}

/// Encodes a page as PNG, the only encoding either destination writes.
async fn encode_page(image: RgbaImage) -> Result<Vec<u8>> {
    tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        PngEncoder::new_with_quality(&mut bytes, CompressionType::Best, FilterType::Adaptive)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                ExtendedColorType::Rgba8,
            )?;
        Ok(bytes)
    })
    .await
    .context("page encode worker stopped unexpectedly")?
}

/// Advances a running export job and publishes the update.
fn advance_export(handle: &AppHandle<CefRuntime>, id: JobId, completed: usize, page: EntityId) {
    let job = {
        let processing = handle.state::<Processing>();
        let mut jobs = processing.jobs.lock();
        jobs.get_mut(&id).map(|job| {
            job.completed = completed;
            job.page = Some(page);
            job.clone()
        })
    };
    if let Some(job) = job {
        handle.state::<JobChannel>().channel.publish(job);
    }
}

/// Retires an export job in its terminal state and publishes the update.
fn finish_export(
    handle: &AppHandle<CefRuntime>,
    id: JobId,
    state: JobState,
    error: Option<String>,
) {
    let processing = handle.state::<Processing>();
    processing.stops.lock().remove(&id);
    let job = processing.jobs.lock().remove(&id).map(|mut job| {
        job.state = state;
        job.error = error;
        job
    });
    if let Some(job) = job {
        handle.state::<JobChannel>().channel.publish(job);
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_export(
    handle: &AppHandle<CefRuntime>,
    id: JobId,
    stop: &StopToken,
    format: ExportFormat,
    destination: Destination,
    snapshot: Snapshot,
    renderer: Renderer,
    rasterizer: Arc<Rasterizer>,
    jobs: Vec<(EntityId, String)>,
) -> Result<JobState> {
    // Renders one page and returns its output file name and encoded bytes. Both
    // destinations share this stage; only the write side differs.
    let render_page = move |(page_id, stem): (EntityId, String)| {
        let renderer = renderer.clone();
        let rasterizer = Arc::clone(&rasterizer);
        let snapshot = snapshot.clone();
        async move {
            let frame = renderer.render(&snapshot, page_id).await?;
            let (name, bytes) = match format {
                ExportFormat::Psd => {
                    let bytes =
                        export_page(rasterizer, &snapshot, &frame, &PsdExportOptions::default())
                            .await?;
                    (format!("{stem}.psd"), bytes)
                }
                ExportFormat::Png | ExportFormat::Cbz => {
                    let image = rasterize(rasterizer, &frame, RasterOptions::default())
                        .await?
                        .image;
                    (format!("{stem}.png"), encode_page(image).await?)
                }
            };
            tracing::info!(
                target: "koharu_metrics",
                metric = "page_exported",
                format = ?format,
            );
            Ok::<_, anyhow::Error>((page_id, name, bytes))
        }
    };
    match destination {
        Destination::Directory(directory) => {
            let mut pages = stream::iter(jobs).map(render_page).buffer_unordered(4);
            let mut completed = 0_usize;
            while let Some(page) = pages.next().await {
                if stop.stopped() {
                    return Ok(JobState::Stopped);
                }
                let (page_id, name, bytes) = page?;
                let path = directory.join(name);
                tokio::fs::write(&path, bytes)
                    .await
                    .with_context(|| format!("failed to write {}", path.display()))?;
                completed += 1;
                advance_export(handle, id, completed, page_id);
            }
            Ok(JobState::Finished)
        }
        Destination::Archive(path) => {
            // `ZipWriter` is a single sequential writer, so pages are rendered
            // concurrently but handed over in page order to one owning task.
            let (sender, mut receiver) = tokio::sync::mpsc::channel::<(String, Vec<u8>)>(4);
            let archive_path = path.clone();
            let writer =
                tokio::task::spawn_blocking(move || write_cbz(&archive_path, &mut receiver));
            let mut outcome = Ok(JobState::Finished);
            {
                let mut pages = stream::iter(jobs).map(render_page).buffered(4);
                let mut completed = 0_usize;
                while let Some(page) = pages.next().await {
                    if stop.stopped() {
                        outcome = Ok(JobState::Stopped);
                        break;
                    }
                    match page {
                        Ok((page_id, name, bytes)) => {
                            if sender.send((name, bytes)).await.is_err() {
                                outcome = Err(anyhow::anyhow!("CBZ writer stopped unexpectedly"));
                                break;
                            }
                            completed += 1;
                            advance_export(handle, id, completed, page_id);
                        }
                        Err(error) => {
                            outcome = Err(error);
                            break;
                        }
                    }
                }
            }
            drop(sender);
            let written = writer.await.context("CBZ writer stopped unexpectedly")?;
            let outcome = match (outcome, written) {
                (Err(error), _) | (Ok(_), Err(error)) => Err(error),
                (Ok(outcome), Ok(())) => Ok(outcome),
            };
            if !matches!(outcome, Ok(JobState::Finished)) {
                // A partial archive is unreadable; do not leave one behind.
                let _ = tokio::fs::remove_file(&path).await;
            }
            outcome
        }
    }
}

#[tracing::instrument(
    target = "koharu_metrics",
    name = "export",
    skip_all,
    fields(origin = "user", format = ?format),
)]
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn export_pages(
    handle: AppHandle<CefRuntime>,
    window: WebviewWindow<CefRuntime>,
    pages: Vec<EntityId>,
    format: ExportFormat,
    project: State<'_, CurrentProject>,
    desktop: State<'_, Desktop>,
    processing: State<'_, Processing>,
    job_channel: State<'_, JobChannel>,
) -> std::result::Result<Option<JobId>, Error> {
    let (snapshot, project_name) = {
        let project = project.project.lock().await;
        let project = project.as_ref().context("no project is open")?;
        (project.snapshot(), project.name.clone())
    };
    let pages = if pages.is_empty() {
        snapshot.pages().map(|page| page.id()).collect()
    } else {
        pages
    };
    if pages.is_empty() {
        return Err(anyhow::anyhow!("there are no pages to export").into());
    }
    let dialog = rfd::AsyncFileDialog::new().set_parent(&window);
    let destination = match format {
        ExportFormat::Png | ExportFormat::Psd => {
            let Some(directory) = dialog
                .pick_folder()
                .await
                .map(|directory| directory.path().to_owned())
            else {
                return Ok(None);
            };
            Destination::Directory(directory)
        }
        ExportFormat::Cbz => {
            let name = sanitize_filename(project_name.trim());
            let name = if name.is_empty() { "export" } else { &name };
            let Some(path) = dialog
                .add_filter("Comic archive", &["cbz"])
                .set_file_name(format!("{name}.cbz"))
                .save_file()
                .await
                .map(|path| path.path().to_owned())
            else {
                return Ok(None);
            };
            Destination::Archive(path)
        }
    };
    let renderer = desktop.renderer();
    let rasterizer = desktop.rasterizer().await?;
    let total = pages.len();
    let jobs = pages
        .into_iter()
        .enumerate()
        .map(|(index, page_id)| {
            let page = snapshot.page(page_id)?.page()?;
            Ok::<_, anyhow::Error>((page_id, page_stem(format, index, total, &page.label)))
        })
        .collect::<Result<Vec<_>>>()?;

    let id = JobId::new();
    let stop = StopToken::default();
    processing.stops.lock().insert(id, stop.clone());
    let job = Job {
        id,
        state: JobState::Running,
        completed: 0,
        total: jobs.len(),
        page: None,
        stage: None,
        model: None,
        error: None,
        // Export writes every page or none; there is no per-page failure.
    };
    processing.jobs.lock().insert(id, job.clone());
    job_channel.channel.publish(job);

    let task_handle = handle.clone();
    drop(tokio::spawn(async move {
        let result = run_export(
            &task_handle,
            id,
            &stop,
            format,
            destination,
            snapshot,
            renderer,
            rasterizer,
            jobs,
        )
        .await;
        let (state, error) = match result {
            Ok(state) => (state, None),
            Err(error) => {
                tracing::error!(%error, "export failed");
                (JobState::Failed, Some(format!("{error:#}")))
            }
        };
        tracing::info!(
            target: "koharu_metrics",
            metric = "export_result",
            outcome = match state {
                JobState::Stopped => "stopped",
                JobState::Failed => "failed",
                _ => "completed",
            },
        );
        finish_export(&task_handle, id, state, error);
    }));
    Ok(Some(id))
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
    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let image = image::load_from_memory(&bytes).context("failed to decode source image")?;
        if image.width() == 0 || image.height() == 0 {
            return Err(anyhow::anyhow!("source image is empty"));
        }
        let image = image.thumbnail(THUMBNAIL_EDGE, THUMBNAIL_EDGE).to_rgba8();
        let encoder = webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height());
        Ok(encoder.encode(80.0).to_vec())
    })
    .await
    .context("thumbnail worker stopped unexpectedly")??;
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
    tokio::task::spawn_blocking(move || {
        let image = image::DynamicImage::ImageRgba8(image)
            .resize(1024, 1024, image::imageops::FilterType::Lanczos3)
            .to_rgba8();
        let encoder = webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height());
        Ok::<_, anyhow::Error>(encoder.encode(85.0).to_vec())
    })
    .await
    .context("preview encode worker stopped unexpectedly")?
}

async fn rasterize(
    rasterizer: Arc<Rasterizer>,
    frame: &Frame,
    options: RasterOptions,
) -> Result<Raster> {
    let frame = frame.raster_frame()?;
    tokio::task::spawn_blocking(move || rasterizer.rasterize(&frame, options))
        .await
        .context("rasterizer worker stopped unexpectedly")?
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_filename_replaces_path_and_reserved_characters() {
        assert_eq!(sanitize_filename("ch1/page:2"), "ch1_page_2");
        assert_eq!(sanitize_filename(r#"a<b>c"d|e?f*g\h"#), "a_b_c_d_e_f_g_h");
        assert_eq!(sanitize_filename("plain name"), "plain name");
    }

    fn sample() -> RgbaImage {
        RgbaImage::from_fn(8, 8, |x, y| {
            // Half opaque, half fully transparent, so the JPEG matte is exercised.
            let alpha = if x < 4 { 255 } else { 0 };
            image::Rgba([10 * x as u8, 10 * y as u8, 200, alpha])
        })
    }

    /// Koharu must be able to import what it exports.
    #[tokio::test]
    async fn an_exported_page_is_importable() {
        let bytes = encode_page(sample()).await.expect("encode page");

        assert_eq!(
            image::guess_format(&bytes).expect("identify encoded page"),
            image::ImageFormat::Png,
        );
        image::load_from_memory(&bytes).expect("decode encoded page");
    }

    #[test]
    fn archive_members_follow_the_comic_archive_convention() {
        let cbz = ExportFormat::Cbz;
        // A label that already carries its own number must not repeat it.
        assert_eq!(page_stem(cbz, 0, 20, "001.jpg"), "P001");
        assert_eq!(page_stem(cbz, 9, 20, "010.jpg"), "P010");
        // Nor should a label leak into the archive in any other form.
        assert_eq!(page_stem(cbz, 1, 20, "pages/page2.png"), "P002");
        assert_eq!(page_stem(cbz, 2, 20, ""), "P003");
    }

    #[test]
    fn archive_numbering_widens_for_large_projects() {
        let cbz = ExportFormat::Cbz;
        // Three digits is the convention, but members must still sort.
        assert_eq!(page_stem(cbz, 0, 999, "a"), "P001");
        assert_eq!(page_stem(cbz, 0, 1000, "a"), "P0001");
        assert_eq!(page_stem(cbz, 1233, 1500, "a"), "P1234");
    }

    #[test]
    fn loose_files_keep_the_page_label() {
        assert_eq!(
            page_stem(ExportFormat::Png, 0, 3, "cover.png"),
            "0001_cover",
            "the label is what identifies a loose file on disk",
        );
        assert_eq!(
            page_stem(ExportFormat::Psd, 1, 3, "ch1/page.psd"),
            "0002_ch1_page"
        );
        assert_eq!(page_stem(ExportFormat::Png, 2, 3, "   "), "0003_page");
    }

    /// Order, compression and payload are properties of one archive, so one
    /// archive proves all three.
    #[tokio::test]
    async fn write_cbz_preserves_members_and_stores_them_intact() {
        let members = ["0001_a.png", "0002_b.png", "0010_c.png"];
        let payload = b"payload".to_vec();
        let path = std::env::temp_dir().join(format!(
            "koharu-export-archive-{}-{:?}.cbz",
            std::process::id(),
            std::thread::current().id()
        ));

        let (sender, mut receiver) = tokio::sync::mpsc::channel::<(String, Vec<u8>)>(4);
        let archive_path = path.clone();
        let writer = tokio::task::spawn_blocking(move || write_cbz(&archive_path, &mut receiver));
        for name in members {
            sender
                .send((name.to_owned(), payload.clone()))
                .await
                .expect("send member");
        }
        drop(sender);
        writer.await.expect("writer task").expect("write archive");

        let file = std::fs::File::open(&path).expect("open archive");
        let mut archive = zip::ZipArchive::new(file).expect("read archive");
        let mut names = Vec::new();
        let mut compressions = Vec::new();
        let mut bytes = Vec::new();
        for index in 0..archive.len() {
            let mut member = archive.by_index(index).expect("member");
            names.push(member.name().to_owned());
            compressions.push(member.compression());
            let mut content = Vec::new();
            std::io::Read::read_to_end(&mut member, &mut content).expect("read member");
            bytes.push(content);
        }
        drop(archive);
        std::fs::remove_file(&path).expect("remove archive");

        // Page order, not archive-internal sorting, is what readers rely on.
        assert_eq!(names, members);
        // Every page format is already compressed, so deflating again would
        // cost time for nothing.
        assert!(
            compressions
                .iter()
                .all(|method| *method == zip::CompressionMethod::Stored)
        );
        assert!(bytes.iter().all(|content| *content == payload));
    }
}
