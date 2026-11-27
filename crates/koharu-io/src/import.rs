//! Headless import of images, archives, PDFs, and directories into a Koharu
//! scene session.

use std::{
    cell::RefCell,
    io::{self, Read as _, Write},
    path::{Path, PathBuf},
    rc::Rc,
    sync::LazyLock,
};

use anyhow::{Context as _, Result, bail};
use fs_err::File;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use hayro::{
    PixmapSettings, RenderCache, RenderSettings,
    hayro_interpret::InterpreterSettings,
    hayro_syntax::Pdf,
    render,
    vello_cpu::{Pixels, color::palette::css::WHITE, peniko::ImageAlphaType},
};
use image::{ExtendedColorType, ImageEncoder as _, ImageFormat, codecs::png::PngEncoder};
use koharu_scene::{Edit, Hash, Op, Session};
use parking_lot::Mutex;
use rars::ArchiveReader;
use rayon::prelude::*;
use serde::Serialize;
use specta::Type;

use crate::{COMIC_INFO_KEY, ComicInfo};

/// The page file extensions supported by Koharu import.
pub const EXTENSIONS: [&str; 9] = [
    "png", "jpg", "jpeg", "webp", "zip", "cbz", "rar", "cbr", "pdf",
];

/// PDF points to pixels at 300 DPI, which keeps fine lettering and line art.
/// An A4 page is about 33 MiB of RGBA at this scale; each thread renders one
/// page at a time, so its buffer goes before the thread's next.
pub(crate) const PDF_SCALE: f32 = 300.0 / 72.0;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Type)]
pub struct ImportReport {
    pub added_pages: usize,
    pub skipped_files: Vec<String>,
}

/// The files an OS leaves in a folder it archives, which aren't pages:
/// macOS's resource forks and Finder settings, and Windows's thumbnail cache
/// and folder settings.
static METADATA: LazyLock<GlobSet> = LazyLock::new(|| {
    let mut set = GlobSetBuilder::new();
    for pattern in [
        "**/__MACOSX",
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

fn is_comic_info(name: &Path) -> bool {
    !METADATA.is_match(name)
        && name
            .file_name()
            .and_then(|file| file.to_str())
            .is_some_and(|file| file.eq_ignore_ascii_case("comicinfo.xml"))
}

struct Extracted {
    pages: Vec<Vec<u8>>,
    comic_info: Option<ComicInfo>,
}

/// Reads a page file's pages: their images, encoded, in order, plus optional archive metadata.
type Reader = fn(&Path) -> Result<Extracted>;

/// An archive's images by member name.
type Members = Vec<(String, Vec<u8>)>;

/// How a page file's pages are read, or `None` when it holds none.
fn reader(path: &Path) -> Option<Reader> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "zip" | "cbz" => Some(extract_zip),
        "rar" | "cbr" => Some(extract_rar),
        "pdf" => Some(|path| {
            Ok(Extracted {
                pages: render_pdf(path)?,
                comic_info: None,
            })
        }),
        _ if raster(path) => Some(|path| {
            Ok(Extracted {
                pages: vec![fs_err::read(path)?],
                comic_info: None,
            })
        }),
        _ => None,
    }
}

fn collect_files(
    paths: &[PathBuf],
    supported: &mut Vec<PathBuf>,
    skipped_files: &mut Vec<String>,
) -> Result<()> {
    let mut sorted = paths.to_vec();
    alphanumeric_sort::sort_slice_by_os_str_key(&mut sorted, |path| {
        path.file_name().unwrap_or(path.as_os_str())
    });

    for path in sorted {
        if METADATA.is_match(&path) {
            continue;
        }
        if path.is_dir() {
            let mut entries = Vec::new();
            for entry in fs_err::read_dir(&path)? {
                entries.push(entry?.path());
            }
            collect_files(&entries, supported, skipped_files)?;
        } else if path.is_file() {
            if reader(&path).is_some() {
                supported.push(path);
            } else {
                let display = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                skipped_files.push(display);
            }
        } else {
            // Surface missing or unreadable paths rather than silently ignoring them.
            fs_err::metadata(&path)?;
        }
    }

    Ok(())
}

/// Imports images, archives, PDFs, and recursively expanded folders into `session`
/// starting at `insert_at` (or appended at the end when `None`), in a single undoable `Edit`.
///
/// All directory traversal and decoding runs before acquiring `session.lock()`.
pub fn import_paths(
    session: &Mutex<Session>,
    paths: &[PathBuf],
    insert_at: Option<usize>,
) -> Result<ImportReport> {
    let mut supported = Vec::new();
    let mut skipped_files = Vec::new();
    collect_files(paths, &mut supported, &mut skipped_files)?;

    if supported.is_empty() {
        bail!("no supported image, archive, or PDF files found");
    }

    let extracted: Vec<Extracted> = supported
        .into_par_iter()
        .map(|path| {
            let read = reader(&path).expect("path already verified as supported");
            read(&path)
        })
        .collect::<Result<Vec<_>>>()?;

    let adopted_comic_info = extracted
        .iter()
        .find_map(|item| item.comic_info.as_ref())
        .cloned();
    let pages: Vec<Vec<u8>> = extracted.into_iter().flat_map(|item| item.pages).collect();

    let mut session = session.lock();
    let should_adopt = session.project().metadata(COMIC_INFO_KEY).is_none();
    let page_count = session.project().pages().count();
    let start = insert_at.unwrap_or(page_count).min(page_count);
    let added_pages = pages.len();
    let mut ops = Vec::with_capacity(added_pages * 2 + usize::from(should_adopt));
    for (offset, bytes) in pages.into_iter().enumerate() {
        let background = Some(Hash::of(&bytes));
        ops.push(Op::AddFile { bytes });
        ops.push(Op::AddPage {
            id: None,
            index: start + offset,
            size: None,
            background,
        });
    }
    if should_adopt && let Some(comic_info) = adopted_comic_info {
        ops.push(Op::SetProjectMetadata {
            key: COMIC_INFO_KEY.to_owned(),
            value: Some(serde_json::to_value(comic_info)?),
        });
    }
    session.apply(Edit {
        label: "import".to_owned(),
        ops,
    })?;

    Ok(ImportReport {
        added_pages,
        skipped_files,
    })
}

/// An archive's images in name order, numbers by value.
fn in_name_order(path: &Path, mut images: Members) -> Result<Vec<Vec<u8>>> {
    if images.is_empty() {
        bail!("{} holds no images", path.display());
    }
    alphanumeric_sort::sort_slice_by_str_key(&mut images, |(name, _)| name);
    Ok(images.into_iter().map(|(_, bytes)| bytes).collect())
}

fn parse_comic_info_candidates(path: &Path, mut candidates: Members) -> Option<ComicInfo> {
    alphanumeric_sort::sort_slice_by_str_key(&mut candidates, |(name, _)| name);
    for (name, bytes) in candidates {
        match ComicInfo::from_xml_bytes(&bytes) {
            Ok(Some(info)) => return Some(info),
            Ok(None) => continue,
            Err(err) => {
                tracing::warn!(
                    archive = %path.display(),
                    member = %name,
                    error = %err,
                    "ignoring malformed ComicInfo.xml in archive"
                );
            }
        }
    }
    None
}

/// A ZIP or CBZ archive's images and optional ComicInfo.xml.
fn extract_zip(path: &Path) -> Result<Extracted> {
    let mut archive = zip::ZipArchive::new(File::open(path)?)
        .with_context(|| format!("failed to read the archive {}", path.display()))?;
    let mut images = Vec::new();
    let mut comic_info_candidates = Vec::new();
    for index in 0..archive.len() {
        let mut member = archive
            .by_index(index)
            .with_context(|| format!("failed to read member {index} of {}", path.display()))?;
        if member.is_dir() {
            continue;
        }
        let name = member
            .name()
            .with_context(|| format!("failed to read member {index}'s name in {}", path.display()))?
            .into_owned();
        let member_path = Path::new(&name);
        if METADATA.is_match(member_path) {
            continue;
        }
        let is_info = is_comic_info(member_path);
        if !raster(member_path) && !is_info {
            continue;
        }
        let mut bytes = Vec::new();
        member
            .read_to_end(&mut bytes)
            .with_context(|| format!("failed to read {name:?} in {}", path.display()))?;
        if is_info {
            comic_info_candidates.push((name, bytes));
        } else {
            images.push((name, bytes));
        }
    }
    Ok(Extracted {
        pages: in_name_order(path, images)?,
        comic_info: parse_comic_info_candidates(path, comic_info_candidates),
    })
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

/// A RAR or CBR archive's images and optional ComicInfo.xml.
fn extract_rar(path: &Path) -> Result<Extracted> {
    let archive = ArchiveReader::read_path(path)
        .with_context(|| format!("failed to open the archive {}", path.display()))?;
    let images = Rc::new(RefCell::new(Vec::new()));
    let comic_info_candidates = Rc::new(RefCell::new(Vec::new()));
    archive
        .extract_to(None, |entry| {
            let name = entry.name_lossy();
            let member = Path::new(&name);
            if entry.is_directory || METADATA.is_match(member) {
                return Ok(Box::new(io::sink()));
            }
            if is_comic_info(member) {
                comic_info_candidates.borrow_mut().push((name, Vec::new()));
                return Ok(Box::new(LastMember(Rc::clone(&comic_info_candidates))));
            }
            if !raster(member) {
                return Ok(Box::new(io::sink()));
            }
            images.borrow_mut().push((name, Vec::new()));
            Ok(Box::new(LastMember(Rc::clone(&images))))
        })
        .with_context(|| format!("failed to extract {}", path.display()))?;
    Ok(Extracted {
        pages: in_name_order(path, images.take())?,
        comic_info: parse_comic_info_candidates(path, comic_info_candidates.take()),
    })
}

/// A PDF's pages, each rendered with hayro, in parallel. hayro's cache isn't
/// thread-safe, so each thread keeps its own.
fn render_pdf(path: &Path) -> Result<Vec<Vec<u8>>> {
    let pdf = Pdf::new(fs_err::read(path)?)
        .map_err(|error| anyhow::anyhow!("failed to parse {}: {error:?}", path.display()))?;
    let pages = pdf.pages();
    if pages.is_empty() {
        bail!("{} has no pages", path.display());
    }
    let pixmap = PixmapSettings {
        x_scale: PDF_SCALE,
        y_scale: PDF_SCALE,
        bg_color: WHITE,
    };
    pages
        .par_iter()
        .enumerate()
        .map_init(RenderCache::new, |cache, (index, page)| {
            let rendered = render(
                page,
                cache,
                &InterpreterSettings::default(),
                &RenderSettings::default(),
                &pixmap,
            );
            let (width, height) = (rendered.width().into(), rendered.height().into());
            let (pixels, color) = match rendered.try_take_rgb8(ImageAlphaType::Alpha) {
                Pixels::Rgb8(pixels) => (pixels, ExtendedColorType::Rgb8),
                Pixels::Rgba8(pixels) => (pixels, ExtendedColorType::Rgba8),
            };
            let mut png = Vec::new();
            PngEncoder::new(&mut png)
                .write_image(&pixels, width, height, color)
                .with_context(|| {
                    format!("failed to encode page {} of {}", index + 1, path.display())
                })?;
            Ok(png)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use image::{ImageBuffer, Rgba};

    use super::*;

    fn tiny_png(r: u8) -> Vec<u8> {
        let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
            ImageBuffer::from_pixel(2, 2, Rgba([r, r, r, 255]));
        let mut buf = Vec::new();
        PngEncoder::new(&mut buf)
            .write_image(img.as_raw(), 2, 2, ExtendedColorType::Rgba8)
            .unwrap();
        buf
    }

    fn new_session(dir: &Path) -> Mutex<Session> {
        let path = dir.join("project.khr");
        Mutex::new(Session::create(&path, koharu_scene::Meta::default()).unwrap())
    }

    #[test]
    fn expands_directories_recursively_filters_metadata_and_reports_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("drop");
        let sub = root.join("chapter 2");
        let macosx = root.join("__MACOSX");
        fs_err::create_dir_all(&sub).unwrap();
        fs_err::create_dir_all(&macosx).unwrap();

        fs_err::write(root.join("page10.png"), tiny_png(10)).unwrap();
        fs_err::write(root.join("page2.png"), tiny_png(2)).unwrap();
        fs_err::write(sub.join("page1.png"), tiny_png(1)).unwrap();
        fs_err::write(root.join("notes.txt"), b"ignore me").unwrap();
        fs_err::write(root.join(".DS_Store"), b"junk").unwrap();
        fs_err::write(root.join("._page2.png"), b"junk").unwrap();
        fs_err::write(macosx.join("page99.png"), tiny_png(99)).unwrap();

        let session = new_session(tmp.path());
        let report = import_paths(&session, &[root], None).unwrap();

        assert_eq!(report.added_pages, 3);
        assert_eq!(report.skipped_files, vec!["notes.txt".to_owned()]);

        let locked = session.lock();
        let project = locked.project();
        let pages: Vec<_> = project.pages().collect();
        assert_eq!(pages.len(), 3);
        // Sorted alphanumerically inside root (`chapter 2` -> `page2.png` -> `page10.png`).
        let bytes0 = locked
            .file(&pages[0].background.as_ref().unwrap().file)
            .unwrap();
        let bytes1 = locked
            .file(&pages[1].background.as_ref().unwrap().file)
            .unwrap();
        let bytes2 = locked
            .file(&pages[2].background.as_ref().unwrap().file)
            .unwrap();
        assert_eq!(bytes0, tiny_png(1));
        assert_eq!(bytes1, tiny_png(2));
        assert_eq!(bytes2, tiny_png(10));
    }

    #[test]
    fn inserts_at_requested_index_and_clamps_to_page_count_as_single_undo_step() {
        let tmp = tempfile::tempdir().unwrap();
        let p1 = tmp.path().join("a.png");
        let p2 = tmp.path().join("b.png");
        let p3 = tmp.path().join("c.png");
        let p4 = tmp.path().join("d.png");
        fs_err::write(&p1, tiny_png(1)).unwrap();
        fs_err::write(&p2, tiny_png(2)).unwrap();
        fs_err::write(&p3, tiny_png(3)).unwrap();
        fs_err::write(&p4, tiny_png(4)).unwrap();

        let session = new_session(tmp.path());
        import_paths(&session, &[p1, p4], None).unwrap();
        // Insert [p2, p3] at index 1 (between 1 and 4) as a single undoable edit.
        let report = import_paths(&session, &[p2, p3], Some(1)).unwrap();
        assert_eq!(report.added_pages, 2);

        {
            let locked = session.lock();
            let project = locked.project();
            let values: Vec<_> = project
                .pages()
                .map(|p| locked.file(&p.background.as_ref().unwrap().file).unwrap())
                .collect();
            assert_eq!(
                values,
                vec![tiny_png(1), tiny_png(2), tiny_png(3), tiny_png(4)]
            );
        }

        // One undo reverts both inserted pages at once.
        session.lock().undo().unwrap();
        assert_eq!(session.lock().project().pages().count(), 2);

        // Out-of-bounds `insert_at` clamps cleanly to `page_count`.
        let extra = tmp.path().join("e.png");
        fs_err::write(&extra, tiny_png(5)).unwrap();
        let report = import_paths(&session, &[extra], Some(999)).unwrap();
        assert_eq!(report.added_pages, 1);
        assert_eq!(session.lock().project().pages().count(), 3);
    }

    #[test]
    fn fails_when_zero_supported_files_are_found_without_mutating_session() {
        let tmp = tempfile::tempdir().unwrap();
        let txt = tmp.path().join("readme.md");
        fs_err::write(&txt, b"hello").unwrap();

        let session = new_session(tmp.path());
        let err = import_paths(&session, &[txt], None).unwrap_err();
        assert!(err.to_string().contains("no supported"));
        assert_eq!(session.lock().project().pages().count(), 0);
    }

    #[test]
    fn extracts_cbz_archive_alphanumerically() {
        let tmp = tempfile::tempdir().unwrap();
        let cbz = tmp.path().join("vol.cbz");
        let mut writer = zip::ZipWriter::new(File::create(&cbz).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        writer.start_file("010.png", opts).unwrap();
        writer.write_all(&tiny_png(10)).unwrap();
        writer.start_file("002.png", opts).unwrap();
        writer.write_all(&tiny_png(2)).unwrap();
        writer.start_file("ComicInfo.xml", opts).unwrap();
        writer.write_all(b"<ComicInfo/>").unwrap();
        writer.finish().unwrap();

        let session = new_session(tmp.path());
        let report = import_paths(&session, &[cbz], None).unwrap();
        assert_eq!(report.added_pages, 2);
        assert!(report.skipped_files.is_empty());

        let locked = session.lock();
        let project = locked.project();
        let pages: Vec<_> = project.pages().collect();
        assert_eq!(
            locked
                .file(&pages[0].background.as_ref().unwrap().file)
                .unwrap(),
            tiny_png(2)
        );
        assert_eq!(
            locked
                .file(&pages[1].background.as_ref().unwrap().file)
                .unwrap(),
            tiny_png(10)
        );
        assert!(project.metadata(COMIC_INFO_KEY).is_none());
    }

    #[test]
    fn adopts_comic_info_from_first_archive_only_and_undoes_with_import() {
        let tmp = tempfile::tempdir().unwrap();
        let cbz1 = tmp.path().join("01.cbz");
        {
            let mut writer = zip::ZipWriter::new(File::create(&cbz1).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            writer.start_file("001.png", opts).unwrap();
            writer.write_all(&tiny_png(1)).unwrap();
            writer.start_file("__MACOSX/ComicInfo.xml", opts).unwrap();
            writer
                .write_all(b"<ComicInfo><Series>Wrong</Series></ComicInfo>")
                .unwrap();
            writer.start_file("comicinfo.xml", opts).unwrap();
            writer
                .write_all(
                    b"<ComicInfo><Series>First Series</Series><Number>1</Number></ComicInfo>",
                )
                .unwrap();
            writer.finish().unwrap();
        }
        let cbz2 = tmp.path().join("02.cbz");
        {
            let mut writer = zip::ZipWriter::new(File::create(&cbz2).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            writer.start_file("002.png", opts).unwrap();
            writer.write_all(&tiny_png(2)).unwrap();
            writer.start_file("ComicInfo.xml", opts).unwrap();
            writer
                .write_all(b"<ComicInfo><Series>Second Series</Series></ComicInfo>")
                .unwrap();
            writer.finish().unwrap();
        }

        let session = new_session(tmp.path());
        let report = import_paths(&session, &[cbz1, cbz2.clone()], None).unwrap();
        assert_eq!(report.added_pages, 2);
        assert!(report.skipped_files.is_empty());

        {
            let locked = session.lock();
            let raw = locked.project().metadata(COMIC_INFO_KEY).unwrap().clone();
            let info: ComicInfo = serde_json::from_value(raw).unwrap();
            assert_eq!(info.series.unwrap().original, "First Series");
            assert_eq!(info.number.as_deref(), Some("1"));
        }

        // Subsequent archive import does not overwrite existing ComicInfo metadata.
        import_paths(&session, &[cbz2], None).unwrap();
        {
            let locked = session.lock();
            let raw = locked.project().metadata(COMIC_INFO_KEY).unwrap().clone();
            let info: ComicInfo = serde_json::from_value(raw).unwrap();
            assert_eq!(info.series.unwrap().original, "First Series");
        }

        // Undoing the second import keeps metadata; undoing the first clears it.
        session.lock().undo().unwrap();
        assert!(session.lock().project().metadata(COMIC_INFO_KEY).is_some());
        session.lock().undo().unwrap();
        assert!(session.lock().project().metadata(COMIC_INFO_KEY).is_none());
    }
}
