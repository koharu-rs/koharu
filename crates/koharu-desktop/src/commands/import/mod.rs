//! Pages from files: images, archives of them, and PDFs, in name order, as
//! `koharu-app` read them. Each becomes a file of the project and a page
//! made from it.

mod pdf;
mod rar;
mod zip;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, bail};
use koharu_scene::{Edit, Hash, Op};
use rayon::prelude::*;
use serde::Deserialize;
use specta::Type;
use tauri::{State, Window};
use tauri_runtime_cef::CefRuntime;

use super::{Result, blocking};
use crate::app::App;

#[derive(Clone, Copy, Debug, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ImportSource {
    Files,
    /// Every page file directly in a folder.
    Folder,
}

/// What a page file may be.
const EXTENSIONS: [&str; 9] = [
    "png", "jpg", "jpeg", "webp", "zip", "cbz", "rar", "cbr", "pdf",
];

/// One page's encoded image, named for its order.
struct EncodedPage {
    name: String,
    bytes: Vec<u8>,
}

/// Whether a file or an archive member's name is an image a page can be.
fn raster(name: &Path) -> bool {
    name.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["png", "jpg", "jpeg", "webp"]
                .iter()
                .any(|raster| extension.eq_ignore_ascii_case(raster))
        })
}

/// Whether an archive member is a system's metadata, not a page.
fn metadata(name: &Path) -> bool {
    name.file_name()
        .and_then(|file_name| file_name.to_str())
        .is_none_or(|file_name| {
            file_name.starts_with("._")
                || [".ds_store", "thumbs.db", "desktop.ini"]
                    .iter()
                    .any(|candidate| file_name.eq_ignore_ascii_case(candidate))
        })
        || name.components().any(|component| {
            component
                .as_os_str()
                .to_str()
                .is_some_and(|component| component.eq_ignore_ascii_case("__macosx"))
        })
}

/// Picks files or a folder and adds their pages after the project's last.
#[tauri::command]
#[specta::specta]
pub async fn import(
    app: State<'_, Arc<App>>,
    window: Window<CefRuntime>,
    source: ImportSource,
) -> Result<()> {
    let dialog = rfd::AsyncFileDialog::new().set_parent(&window);
    let paths = match source {
        ImportSource::Files => dialog
            .add_filter("Pages", &EXTENSIONS)
            .pick_files()
            .await
            .map(|files| files.iter().map(|file| file.path().to_owned()).collect()),
        ImportSource::Folder => match dialog.pick_folder().await {
            Some(folder) => Some(pages_in(folder.path())?),
            None => None,
        },
    };
    match paths {
        Some(paths) => add(&app, paths).await,
        None => Ok(()),
    }
}

/// Adds the pages of files dropped on the window.
#[tauri::command]
#[specta::specta]
pub async fn import_paths(app: State<'_, Arc<App>>, paths: Vec<PathBuf>) -> Result<()> {
    add(&app, paths).await
}

/// The page files directly in `folder`.
fn pages_in(folder: &Path) -> anyhow::Result<Vec<PathBuf>> {
    Ok(std::fs::read_dir(folder)
        .with_context(|| format!("failed to read {}", folder.display()))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    EXTENSIONS
                        .iter()
                        .any(|known| extension.eq_ignore_ascii_case(known))
                })
        })
        .collect())
}

async fn add(app: &State<'_, Arc<App>>, paths: Vec<PathBuf>) -> Result<()> {
    let app = Arc::clone(app);
    blocking(move || {
        let pages = read(paths)?;
        let session = app.session()?;
        let start = session.lock().project().pages().count();
        let mut ops = Vec::with_capacity(pages.len() * 2);
        for (index, page) in pages.into_iter().enumerate() {
            let background = Some(Hash::of(&page.bytes));
            ops.push(Op::AddFile { bytes: page.bytes });
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
        })
    })
    .await
}

/// The pages of `paths` in name order, numbers by value, an archive's or a
/// PDF's own in theirs.
fn read(mut paths: Vec<PathBuf>) -> anyhow::Result<Vec<EncodedPage>> {
    alphanumeric_sort::sort_slice_by_os_str_key(&mut paths, |path| {
        path.file_name().unwrap_or(path.as_os_str())
    });
    let groups = paths
        .into_par_iter()
        .map(|path| -> anyhow::Result<Vec<EncodedPage>> {
            let extension = path
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase);
            match extension.as_deref() {
                Some("zip" | "cbz") => zip::extract(&path),
                Some("rar" | "cbr") => rar::extract(&path),
                Some("pdf") => pdf::render(&path),
                _ if raster(&path) => Ok(vec![EncodedPage {
                    name: path.to_string_lossy().into_owned(),
                    bytes: std::fs::read(&path)
                        .with_context(|| format!("failed to read {}", path.display()))?,
                }]),
                _ => bail!("{} isn't an image, an archive, or a PDF", path.display()),
            }
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(groups.into_iter().flatten().collect())
}
