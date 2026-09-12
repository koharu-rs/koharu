use std::{io::Write as _, path::Path, sync::Arc};

use anyhow::{Context as _, Result};
use futures::{StreamExt as _, stream};
use image::{ExtendedColorType, ImageEncoder as _, RgbaImage, codecs::jpeg::JpegEncoder};
use koharu_desktop::Desktop;
use koharu_pipeline::StopToken;
use koharu_rasterizer::RasterOptions;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager as _, State, WebviewWindow};
use tauri_runtime_cef::CefRuntime;

use super::{
    ChannelExt as _, Error,
    output::rasterize,
    processing::{Job, JobChannel, JobId, JobKind, JobState, Processing},
    project::CurrentProject,
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveImageFormat {
    Png,
    Jpeg,
    Webp,
}

impl ArchiveImageFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Type)]
#[serde(default)]
pub struct ExportConfig {
    pub jpeg_quality: u8,
    pub webp_quality: u8,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            jpeg_quality: 90,
            webp_quality: 85,
        }
    }
}

impl ExportConfig {
    fn normalized(self) -> Self {
        Self {
            jpeg_quality: self.jpeg_quality.clamp(1, 100),
            webp_quality: self.webp_quality.clamp(1, 100),
        }
    }
}

#[tauri::command]
#[specta::specta]
pub(crate) fn get_export_config() -> std::result::Result<ExportConfig, Error> {
    Ok(koharu_config::load::<ExportConfig>("export")?
        .read()?
        .normalized())
}

#[tauri::command]
#[specta::specta]
pub(crate) fn save_export_config(
    export_config: ExportConfig,
) -> std::result::Result<ExportConfig, Error> {
    let config = koharu_config::load::<ExportConfig>("export")?;
    let normalized = export_config.normalized();
    let mut current = config.write()?;
    *current = normalized;
    current.save()?;
    Ok(normalized)
}

fn member_name(index: usize, total: usize, format: ArchiveImageFormat) -> String {
    let width = total.to_string().len().max(3);
    format!("P{:0width$}.{}", index + 1, format.extension())
}

fn encode(image: RgbaImage, format: ArchiveImageFormat, config: ExportConfig) -> Result<Vec<u8>> {
    let config = config.normalized();
    let mut bytes = Vec::new();
    match format {
        ArchiveImageFormat::Png => super::output::encode_png(&image, &mut bytes)?,
        ArchiveImageFormat::Jpeg => {
            let mut rgb = image::RgbImage::new(image.width(), image.height());
            for (target, source) in rgb.pixels_mut().zip(image.pixels()) {
                let alpha = u32::from(source[3]);
                for channel in 0..3 {
                    target[channel] =
                        ((u32::from(source[channel]) * alpha + 255 * (255 - alpha) + 127) / 255)
                            as u8;
                }
            }
            JpegEncoder::new_with_quality(&mut bytes, config.jpeg_quality).write_image(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                ExtendedColorType::Rgb8,
            )?;
        }
        ArchiveImageFormat::Webp => {
            bytes = webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height())
                .encode(f32::from(config.webp_quality))
                .to_vec();
        }
    }
    Ok(bytes)
}

// The temporary file owns cleanup, including worker errors and unwinding.
struct Archive {
    writer: zip::ZipWriter<tempfile::NamedTempFile>,
}

impl Archive {
    fn new(path: &Path) -> Result<Self> {
        let file = tempfile::Builder::new()
            .prefix("koharu-cbz-")
            .suffix(".part")
            .tempfile_in(path.parent().context("archive has no parent directory")?)?;
        Ok(Self {
            writer: zip::ZipWriter::new(file),
        })
    }

    fn page(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        self.writer.start_file(
            name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )?;
        self.writer.write_all(bytes)?;
        Ok(())
    }

    fn finish(self, path: &Path, stop: &StopToken) -> Result<bool> {
        let file = self.writer.finish()?;
        file.as_file().sync_all()?;
        if stop.stopped() {
            return Ok(false);
        }
        // Persist is the commit point; an existing destination survives earlier failures.
        file.persist(path).map_err(|error| error.error)?;
        Ok(true)
    }
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn export_cbz(
    handle: AppHandle<CefRuntime>,
    window: WebviewWindow<CefRuntime>,
    format: ArchiveImageFormat,
    project: State<'_, CurrentProject>,
) -> std::result::Result<Option<JobId>, Error> {
    let snapshot = project
        .project
        .lock()
        .await
        .as_ref()
        .context("no project is open")?
        .snapshot();
    let pages = snapshot.pages().map(|page| page.id()).collect::<Vec<_>>();
    if pages.is_empty() {
        return Err(anyhow::anyhow!("there are no pages to export").into());
    }
    let Some(path) = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .add_filter("Comic book archive", &["cbz"])
        .set_file_name("chapter.cbz")
        .save_file()
        .await
        .map(|file| file.path().to_owned())
    else {
        return Ok(None);
    };
    let current = project.project.lock().await;
    if current
        .as_ref()
        .is_none_or(|current| current.snapshot().project_id() != snapshot.project_id())
    {
        return Err(anyhow::anyhow!(
            "the active project changed while choosing the export destination"
        )
        .into());
    }
    drop(current);
    let config = get_export_config()?;
    let id = JobId::new();
    let stop = StopToken::default();
    let job = Job {
        id,
        kind: JobKind::Export,
        workflow: None,
        state: JobState::Running,
        completed: 0,
        total: pages.len(),
        page: None,
        stage: None,
        model: None,
        error: None,
    };
    handle
        .state::<Processing>()
        .stops
        .lock()
        .insert(id, stop.clone());
    publish(&handle, job.clone());
    drop(tokio::spawn(async move {
        let mut job = job;
        let result = async {
            let renderer = handle.state::<Desktop>().renderer();
            let rasterizer = handle.state::<Desktop>().rasterizer().await?;
            let destination = path.clone();
            let mut archive =
                tokio::task::spawn_blocking(move || Archive::new(&destination)).await??;
            job.model = Some(rasterizer.adapter_info().name.clone());
            let mut prepared = stream::iter(pages.into_iter().enumerate())
                .map(|(index, page)| {
                    let renderer = renderer.clone();
                    let rasterizer = Arc::clone(&rasterizer);
                    let snapshot = snapshot.clone();
                    let stop = stop.clone();
                    async move {
                        if stop.stopped() {
                            return Ok(None);
                        }
                        let frame = renderer.render(&snapshot, page).await?;
                        if stop.stopped() {
                            return Ok(None);
                        }
                        let image = rasterize(rasterizer, &frame, RasterOptions::default())
                            .await?
                            .image;
                        let bytes = tokio::task::spawn_blocking(move || {
                            if stop.stopped() {
                                return Ok(None);
                            }
                            encode(image, format, config).map(Some)
                        })
                        .await??;
                        Ok::<_, anyhow::Error>(bytes.map(|bytes| (index, page, bytes)))
                    }
                })
                // Ordered buffering overlaps encoding and rendering without parallel ZIP writes.
                .buffered(4);
            while let Some(prepared) = prepared.next().await {
                let Some((index, page, bytes)) = prepared? else {
                    return Ok(false);
                };
                if stop.stopped() {
                    return Ok(false);
                }
                job.page = Some(page);
                publish(&handle, job.clone());
                let name = member_name(index, job.total, format);
                let worker_stop = stop.clone();
                archive = tokio::task::spawn_blocking(move || -> Result<Archive> {
                    if !worker_stop.stopped() {
                        archive.page(&name, &bytes)?;
                    }
                    Ok(archive)
                })
                .await??;
                if stop.stopped() {
                    return Ok(false);
                }
                job.completed += 1;
                publish(&handle, job.clone());
            }
            drop(prepared);
            tokio::task::spawn_blocking(move || archive.finish(&path, &stop)).await?
        }
        .await;
        job.state = match result {
            Ok(true) => JobState::Finished,
            Ok(false) => JobState::Stopped,
            Err(error) => {
                job.error = Some(format!("{error:#}"));
                JobState::Failed
            }
        };
        handle.state::<Processing>().stops.lock().remove(&id);
        handle.state::<Processing>().jobs.lock().remove(&id);
        handle.state::<JobChannel>().channel.publish(job);
    }));
    Ok(Some(id))
}

fn publish(handle: &AppHandle<CefRuntime>, job: Job) {
    handle
        .state::<Processing>()
        .jobs
        .lock()
        .insert(job.id, job.clone());
    handle.state::<JobChannel>().channel.publish(job);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodings_decode_and_jpeg_has_white_matte() {
        for format in [
            ArchiveImageFormat::Png,
            ArchiveImageFormat::Jpeg,
            ArchiveImageFormat::Webp,
        ] {
            let bytes = encode(RgbaImage::new(8, 8), format, ExportConfig::default()).unwrap();
            let decoded = image::load_from_memory(&bytes).unwrap().to_rgba8();
            assert_eq!(decoded.dimensions(), (8, 8));
            if matches!(format, ArchiveImageFormat::Jpeg) {
                assert!(
                    decoded
                        .pixels()
                        .all(|pixel| pixel[0] >= 254 && pixel[1] >= 254 && pixel[2] >= 254)
                );
            }
        }
        let image = RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 128]));
        let bytes = encode(image, ArchiveImageFormat::Jpeg, ExportConfig::default()).unwrap();
        let pixel = image::load_from_memory(&bytes)
            .unwrap()
            .to_rgb8()
            .get_pixel(0, 0)
            .0;
        assert!(
            pixel[0] >= 250 && (123..=131).contains(&pixel[1]) && (123..=131).contains(&pixel[2])
        );
    }

    #[test]
    fn archives_preserve_order_and_widen_names() {
        for count in [1, 3, 1000] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("chapter.cbz");
            let mut archive = Archive::new(&path).unwrap();
            let bytes = encode(
                RgbaImage::new(1, 1),
                ArchiveImageFormat::Png,
                ExportConfig::default(),
            )
            .unwrap();
            for index in 0..count {
                archive
                    .page(&member_name(index, count, ArchiveImageFormat::Png), &bytes)
                    .unwrap();
            }
            assert!(archive.finish(&path, &StopToken::default()).unwrap());
            let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
            assert_eq!(zip.len(), count);
            let mut names = Vec::new();
            for index in 0..count {
                let member = zip.by_index(index).unwrap();
                assert_eq!(member.compression(), zip::CompressionMethod::Stored);
                assert_eq!(
                    member.name(),
                    member_name(index, count, ArchiveImageFormat::Png)
                );
                names.push(member.name().to_owned());
            }
            let mut sorted = names.clone();
            sorted.sort();
            assert_eq!(names, sorted);
        }
    }

    #[test]
    fn cancellation_and_failure_clean_partial_and_preserve_destination() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chapter.cbz");
        std::fs::write(&path, b"existing").unwrap();
        let mut archive = Archive::new(&path).unwrap();
        archive.page("P001.png", b"partial page").unwrap();
        let stop = StopToken::default();
        stop.stop();
        assert!(!archive.finish(&path, &stop).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), b"existing");
        drop(Archive::new(&path).unwrap());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        let fresh = directory.path().join("cancelled.cbz");
        let mut partial = Archive::new(&fresh).unwrap();
        partial.page("P001.png", b"partial page").unwrap();
        assert!(!partial.finish(&fresh, &stop).unwrap());
        assert!(!fresh.exists());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        let occupied = directory.path().join("occupied");
        std::fs::create_dir(&occupied).unwrap();
        assert!(
            Archive::new(&path)
                .unwrap()
                .finish(&occupied, &StopToken::default())
                .is_err()
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[test]
    fn webp_twenty_page_archive_reopens_with_decodable_members() {
        use std::io::Read as _;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chapter.cbz");
        let mut archive = Archive::new(&path).unwrap();
        for index in 0..20 {
            let bytes = encode(
                RgbaImage::from_pixel(8, 8, image::Rgba([index as u8 * 10, 20, 30, 255])),
                ArchiveImageFormat::Webp,
                ExportConfig {
                    webp_quality: 85,
                    ..Default::default()
                },
            )
            .unwrap();
            archive
                .page(&member_name(index, 20, ArchiveImageFormat::Webp), &bytes)
                .unwrap();
        }
        assert!(archive.finish(&path, &StopToken::default()).unwrap());
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        assert_eq!(zip.len(), 20);
        for index in 0..20 {
            let mut member = zip.by_index(index).unwrap();
            assert_eq!(member.name(), format!("P{:03}.webp", index + 1));
            let mut bytes = Vec::new();
            member.read_to_end(&mut bytes).unwrap();
            assert_eq!(
                image::load_from_memory(&bytes)
                    .unwrap()
                    .to_rgba8()
                    .dimensions(),
                (8, 8)
            );
        }
    }

    #[test]
    fn settings_defaults_roundtrip_and_boundaries() {
        let defaults: ExportConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(defaults, ExportConfig::default());
        assert_eq!(
            serde_json::from_str::<ExportConfig>(&serde_json::to_string(&defaults).unwrap())
                .unwrap(),
            defaults
        );
        let config = ExportConfig {
            jpeg_quality: 0,
            webp_quality: 255,
        }
        .normalized();
        assert_eq!((config.jpeg_quality, config.webp_quality), (1, 100));
        for quality in [0, 1, 100, 255] {
            for format in [ArchiveImageFormat::Jpeg, ArchiveImageFormat::Webp] {
                assert!(
                    encode(
                        RgbaImage::new(1, 1),
                        format,
                        ExportConfig {
                            jpeg_quality: quality,
                            webp_quality: quality
                        }
                    )
                    .is_ok()
                );
            }
        }
    }
}
