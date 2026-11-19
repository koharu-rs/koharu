//! Pages from files: images, ZIP, CBZ, RAR, and CBR archives of them, and PDFs
//! rendered with hayro, in name order. Each becomes a file of the project and
//! a page made from it.

use std::{
    cell::RefCell,
    io::{self, Cursor, Read as _, Write},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, LazyLock},
};

use anyhow::{Context as _, bail, ensure};
use fs_err::File;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use hayro::{
    PixmapSettings, RenderCache, RenderSettings, hayro_interpret::InterpreterSettings,
    hayro_syntax::Pdf, render, vello_cpu::color::palette::css::WHITE,
};
use image::ImageFormat;
use koharu_scene::{ArchiveMember, Edit, ExternalImage, Hash, MAX_SOURCE_BYTES, Op, Size};
use rars::ArchiveReader;
use rayon::prelude::*;
use serde::Deserialize;
use specta::Type;
use tauri::{State, Window};
use tauri_runtime_cef::CefRuntime;

use super::Result;
use crate::app::App;

#[derive(Clone, Copy, Debug, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ImportSource {
    Files,
    /// Every page file directly in a folder.
    Folder,
}

/// The page files the file dialog offers.
const EXTENSIONS: [&str; 9] = [
    "png", "jpg", "jpeg", "webp", "zip", "cbz", "rar", "cbr", "pdf",
];

/// PDF points to pixels at 300 DPI, which keeps fine lettering and line art.
/// An A4 page is about 33 MiB of RGBA at this scale; one page renders at a
/// time, so its buffer goes before the next.
const PDF_SCALE: f32 = 300.0 / 72.0;

/// The files an OS leaves in a folder it archives, which aren't pages:
/// macOS's resource forks and Finder settings, and Windows's thumbnail cache
/// and folder settings.
static METADATA: LazyLock<GlobSet> = LazyLock::new(|| {
    let mut set = GlobSetBuilder::new();
    for pattern in [
        "**/__MACOSX/**",
        "**/._*",
        "**/.DS_Store",
        "**/Thumbs.db",
        "**/desktop.ini",
    ] {
        set.add(
            GlobBuilder::new(pattern)
                .case_insensitive(true)
                .literal_separator(true)
                .build()
                .expect("the patterns are globs"),
        );
    }
    set.build().expect("the patterns are globs")
});

/// Whether a file or an archive member's name is an image a page can be.
fn raster(name: &Path) -> bool {
    matches!(
        ImageFormat::from_path(name),
        Ok(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)
    )
}

/// Reads a page file's pages: their images, encoded, in order.
type Reader = fn(&Path) -> anyhow::Result<Vec<Vec<u8>>>;

/// An archive's images by member name.
type Members = Vec<(String, Vec<u8>)>;

/// How a page file's pages are read, or `None` when it holds none.
fn reader(path: &Path) -> Option<Reader> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "zip" | "cbz" => Some(extract_zip),
        "rar" | "cbr" => Some(extract_rar),
        "pdf" => Some(render_pdf),
        _ if raster(path) => Some(|path| Ok(vec![fs_err::read(path)?])),
        _ => None,
    }
}

/// Picks files or a folder and adds their pages after the project's last: the
/// files in name order, numbers by value, an archive's or a PDF's own pages in
/// theirs.
#[tauri::command]
#[specta::specta]
pub async fn import(
    app: State<'_, Arc<App>>,
    window: Window<CefRuntime>,
    source: ImportSource,
) -> Result<()> {
    let dialog = rfd::AsyncFileDialog::new().set_parent(&window);
    let mut paths: Vec<PathBuf> = match source {
        ImportSource::Files => match dialog.add_filter("Pages", &EXTENSIONS).pick_files().await {
            Some(files) => files.iter().map(|file| file.path().to_owned()).collect(),
            None => return Ok(()),
        },
        ImportSource::Folder => match dialog.pick_folder().await {
            Some(folder) => fs_err::read_dir(folder.path())?
                .filter_map(|entry| Some(entry.ok()?.path()))
                .filter(|path| path.is_file() && reader(path).is_some())
                .collect(),
            None => return Ok(()),
        },
    };
    alphanumeric_sort::sort_slice_by_os_str_key(&mut paths, |path| {
        path.file_name().unwrap_or(path.as_os_str())
    });
    let app = Arc::clone(&app);
    Ok(tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let pages: Vec<Vec<u8>> = paths
            .into_par_iter()
            .map(|path| {
                let read = reader(&path).with_context(|| {
                    format!("{} isn't an image, an archive, or a PDF", path.display())
                })?;
                read(&path)
            })
            .collect::<anyhow::Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect();
        let session = app.session()?;
        let start = session.lock().project().pages().count();
        let mut ops = Vec::with_capacity(pages.len() * 2);
        for (index, bytes) in pages.into_iter().enumerate() {
            let background = Some(Hash::of(&bytes));
            ops.push(Op::AddFile { bytes });
            ops.push(Op::AddPage {
                id: None,
                index: start + index,
                size: None,
                background,
            });
        }
        app.edit(Edit {
            label: "import".to_owned(),
            ops,
        })?;
        Ok(())
    })
    .await??)
}

/// An archive's images in name order, numbers by value.
fn in_name_order(path: &Path, mut images: Members) -> anyhow::Result<Vec<Vec<u8>>> {
    if images.is_empty() {
        bail!("{} holds no images", path.display());
    }
    alphanumeric_sort::sort_slice_by_str_key(&mut images, |(name, _)| name);
    Ok(images.into_iter().map(|(_, bytes)| bytes).collect())
}

/// A ZIP or CBZ archive's images.
fn extract_zip(path: &Path) -> anyhow::Result<Vec<Vec<u8>>> {
    let mut images = Vec::new();
    visit_zip(path, u64::MAX, |member, bytes| {
        images.push((member.name, bytes));
        Ok(())
    })?;
    in_name_order(path, images)
}

fn visit_zip(
    path: &Path,
    max_bytes: u64,
    mut visit: impl FnMut(ArchiveMember, Vec<u8>) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let mut archive = zip::ZipArchive::new(File::open(path)?)
        .with_context(|| format!("failed to read the archive {}", path.display()))?;
    for index in 0..archive.len() {
        let member = archive
            .by_index(index)
            .with_context(|| format!("failed to read member {index} of {}", path.display()))?;
        if member.is_dir() {
            continue;
        }
        let name = member
            .name()
            .with_context(|| format!("failed to read member {index}'s name in {}", path.display()))?
            .into_owned();
        if !raster(Path::new(&name)) || METADATA.is_match(&name) {
            continue;
        }
        let size = member.size();
        ensure!(
            size != 0 && size <= max_bytes,
            "image {name:?} has an invalid size"
        );
        let mut bytes = Vec::new();
        member
            .take(size.saturating_add(1))
            .read_to_end(&mut bytes)
            .with_context(|| format!("failed to read {name:?} in {}", path.display()))?;
        ensure!(
            bytes.len() as u64 == size,
            "archive member {name:?} is truncated"
        );
        visit(ArchiveMember { index, name }, bytes)?;
    }
    Ok(())
}

#[derive(Clone, Copy, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum LinkedSource {
    Zip,
    Folder,
}

pub(super) async fn pick_originals(
    window: &Window<CefRuntime>,
    kind: LinkedSource,
) -> Option<PathBuf> {
    let dialog = rfd::AsyncFileDialog::new().set_parent(window);
    match kind {
        LinkedSource::Zip => {
            dialog
                .add_filter("ZIP / CBZ", &["zip", "cbz"])
                .pick_file()
                .await
        }
        LinkedSource::Folder => dialog.pick_folder().await,
    }
    .map(|file| file.path().to_owned())
}

/// Reads each original once for its fingerprint and dimensions. Only the
/// metadata survives the callback, including the original ZIP member index.
pub(super) fn linked_images(
    selected: &Path,
    project: &Path,
    kind: LinkedSource,
) -> anyhow::Result<Vec<(Hash, ExternalImage)>> {
    let selected = selected.canonicalize()?;
    let parent = project
        .parent()
        .context("the project needs a parent directory")?
        .canonicalize()?;
    let mut found = Vec::new();
    let mut add =
        |path: &Path, member: Option<ArchiveMember>, bytes: Vec<u8>| -> anyhow::Result<()> {
            let (width, height) = image::ImageReader::new(Cursor::new(&bytes))
                .with_guessed_format()?
                .into_dimensions()
                .with_context(|| format!("invalid image in {}", path.display()))?;
            let path = path.strip_prefix(&parent).unwrap_or(path).to_owned();
            found.push((
                Hash::of(&bytes),
                ExternalImage {
                    path,
                    member,
                    size: Size { width, height },
                    bytes: bytes.len() as u64,
                },
            ));
            Ok(())
        };
    match kind {
        LinkedSource::Zip => visit_zip(&selected, MAX_SOURCE_BYTES, |member, bytes| {
            add(&selected, Some(member), bytes)
        })?,
        LinkedSource::Folder => {
            for item in fs_err::read_dir(&selected)? {
                let item = item?;
                let path = item.path();
                if !item.file_type()?.is_file() || !raster(&path) {
                    continue;
                }
                let path = path.canonicalize()?;
                ensure!(
                    path.starts_with(&selected),
                    "an image is outside the selected folder"
                );
                let file = File::open(&path)?;
                let length = file.metadata()?.len();
                ensure!(
                    length != 0 && length <= MAX_SOURCE_BYTES,
                    "{} has an invalid size",
                    path.display()
                );
                let mut bytes = Vec::new();
                file.take(length + 1).read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() as u64 == length,
                    "{} changed while it was read",
                    path.display()
                );
                add(&path, None, bytes)?;
            }
        }
    }
    ensure!(!found.is_empty(), "{} holds no images", selected.display());
    // The selected archive or folder is shared by all the returned paths.
    found[0].1.check_location()?;
    alphanumeric_sort::sort_slice_by_os_str_key(&mut found, |(_, image)| match &image.member {
        Some(member) => std::ffi::OsStr::new(&member.name),
        None => image.path.file_name().unwrap_or(image.path.as_os_str()),
    });
    Ok(found)
}

/// Writes into the RAR member opened last, which is the one being extracted:
/// `extract_to` writes each member whole before it opens the next.
struct LastMember(Rc<RefCell<Members>>);

impl Write for LastMember {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .borrow_mut()
            .last_mut()
            .expect("a member is opened before it's written")
            .1
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A RAR or CBR archive's images.
fn extract_rar(path: &Path) -> anyhow::Result<Vec<Vec<u8>>> {
    let archive = ArchiveReader::read_path(path)
        .with_context(|| format!("failed to open the archive {}", path.display()))?;
    let images = Rc::new(RefCell::new(Vec::new()));
    archive
        .extract_to(None, |entry| {
            let name = entry.name_lossy();
            let member = Path::new(&name);
            if entry.is_directory || !raster(member) || METADATA.is_match(member) {
                return Ok(Box::new(io::sink()));
            }
            images.borrow_mut().push((name, Vec::new()));
            Ok(Box::new(LastMember(Rc::clone(&images))))
        })
        .with_context(|| format!("failed to extract {}", path.display()))?;
    in_name_order(path, images.take())
}

/// A PDF's pages, each rendered with hayro.
fn render_pdf(path: &Path) -> anyhow::Result<Vec<Vec<u8>>> {
    let pdf = Pdf::new(fs_err::read(path)?)
        .map_err(|error| anyhow::anyhow!("failed to parse {}: {error:?}", path.display()))?;
    let pages = pdf.pages();
    if pages.is_empty() {
        bail!("{} has no pages", path.display());
    }
    let (interpreter, settings, cache) = (
        InterpreterSettings::default(),
        RenderSettings::default(),
        RenderCache::new(),
    );
    let pixmap = PixmapSettings {
        x_scale: PDF_SCALE,
        y_scale: PDF_SCALE,
        bg_color: WHITE,
    };
    pages
        .iter()
        .enumerate()
        .map(|(index, page)| {
            render(page, &cache, &interpreter, &settings, &pixmap)
                .into_png()
                .with_context(|| {
                    format!("failed to encode page {} of {}", index + 1, path.display())
                })
        })
        .collect()
}
