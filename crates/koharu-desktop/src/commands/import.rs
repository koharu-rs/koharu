//! Pages from files: images, ZIP, CBZ, RAR, and CBR archives of them, and PDFs
//! rendered with hayro, in name order. Each becomes a file of the project and
//! a page made from it.

use std::{path::PathBuf, sync::Arc};

use koharu_io::{EXTENSIONS, ImportReport};
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
    /// Every page file inside a folder.
    Folder,
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
    let dialog = super::file_dialog(&window);
    let paths: Vec<PathBuf> = match source {
        ImportSource::Files => match dialog.add_filter("Pages", &EXTENSIONS).pick_files().await {
            Some(files) => files.iter().map(|file| file.path().to_owned()).collect(),
            None => return Ok(()),
        },
        ImportSource::Folder => match dialog.pick_folder().await {
            Some(folder) => vec![folder.path().to_owned()],
            None => return Ok(()),
        },
    };
    let session = app.session()?;
    tokio::task::spawn_blocking(move || koharu_io::import_paths(&session, &paths, None)).await??;
    Ok(())
}

/// Imports dropped or provided file/folder paths starting at `insert_at` (or at
/// the end when `None`), expanding directories recursively in alphanumeric order.
#[tauri::command]
#[specta::specta]
pub async fn import_paths(
    app: State<'_, Arc<App>>,
    paths: Vec<String>,
    insert_at: Option<usize>,
) -> Result<ImportReport> {
    let session = app.session()?;
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    Ok(
        tokio::task::spawn_blocking(move || koharu_io::import_paths(&session, &paths, insert_at))
            .await??,
    )
}
