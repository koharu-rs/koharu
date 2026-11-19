//! The projects folder and the open project.

use std::{collections::BTreeMap, path::Path, sync::Arc};

use anyhow::{Context as _, ensure};
use koharu_scene::{EXTENSION, Edit, Op, Session};
use tauri::{State, Window};
use tauri_runtime_cef::CefRuntime;

use super::{
    Result,
    import::{LinkedSource, linked_images, pick_originals},
};
use crate::app::App;

#[tauri::command]
#[specta::specta]
pub fn list_projects(app: State<'_, Arc<App>>) -> Result<Vec<String>> {
    Ok(app.library.list()?)
}

/// Creates a project and opens it.
#[tauri::command]
#[specta::specta]
pub async fn create(app: State<'_, Arc<App>>, name: String) -> Result<()> {
    let app = Arc::clone(&app);
    Ok(tokio::task::spawn_blocking(move || {
        let session = app.library.create(&name)?;
        app.open(name, session)
    })
    .await??)
}

#[tauri::command]
#[specta::specta]
pub async fn open(app: State<'_, Arc<App>>, name: String) -> Result<()> {
    let app = Arc::clone(&app);
    Ok(tokio::task::spawn_blocking(move || {
        let session = app.library.open(&name)?;
        app.open(name, session)
    })
    .await??)
}

#[tauri::command]
#[specta::specta]
pub async fn open_project_file(app: State<'_, Arc<App>>, window: Window<CefRuntime>) -> Result<()> {
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .add_filter("Koharu", &[EXTENSION])
        .pick_file()
        .await
    else {
        return Ok(());
    };
    let path = file.path().to_owned();
    let app = Arc::clone(&app);
    Ok(tokio::task::spawn_blocking(move || {
        let session = Session::open(&path)?;
        app.open(project_name(&path), session)
    })
    .await??)
}

#[tauri::command]
#[specta::specta]
pub async fn create_linked_project(
    app: State<'_, Arc<App>>,
    window: Window<CefRuntime>,
    kind: LinkedSource,
) -> Result<()> {
    let Some(source) = pick_originals(&window, kind).await else {
        return Ok(());
    };
    let suggested = source.file_stem().unwrap_or_default().to_string_lossy();
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .add_filter("Koharu", &[EXTENSION])
        .set_file_name(format!("{suggested}.{EXTENSION}"))
        .save_file()
        .await
    else {
        return Ok(());
    };
    let mut path = file.path().to_owned();
    if path.extension().is_none() {
        path.set_extension(EXTENSION);
    }
    let app = Arc::clone(&app);
    Ok(tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        ensure!(
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case(EXTENSION)),
            "a Koharu project needs the .{EXTENSION} extension"
        );
        let images = linked_images(&source, &path, kind)?;
        let mut session = Session::create(&path)?;
        let edit = Edit {
            label: "import".to_owned(),
            ops: images
                .into_iter()
                .enumerate()
                .map(|(index, (file, source))| Op::AddExternalPage {
                    index,
                    file,
                    source,
                })
                .collect(),
        };
        if let Err(error) = session.apply(edit) {
            drop(session);
            Session::remove(&path).context("could not remove incomplete project")?;
            return Err(error.into());
        }
        app.open(project_name(&path), session)
    })
    .await??)
}

#[tauri::command]
#[specta::specta]
pub async fn relink_originals(
    app: State<'_, Arc<App>>,
    window: Window<CefRuntime>,
    kind: LinkedSource,
) -> Result<()> {
    let session = app.session()?;
    let Some(source) = pick_originals(&window, kind).await else {
        return Ok(());
    };
    let (path, snapshot) = {
        let session = session.lock();
        (session.path().to_owned(), session.project())
    };
    let app = Arc::clone(&app);
    Ok(tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let images: BTreeMap<_, _> = linked_images(&source, &path, kind)?.into_iter().collect();
        let ops = snapshot
            .external_images()
            .filter(|(_, original)| original.member.is_some() == matches!(kind, LinkedSource::Zip))
            .map(|(file, previous)| {
                let source = images.get(&file).with_context(|| {
                    format!(
                        "the selection has no identical copy of {}",
                        previous.path.display()
                    )
                })?;
                Ok(Op::RelinkExternalImage {
                    file,
                    previous: previous.clone(),
                    source: source.clone(),
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        ensure!(
            !ops.is_empty(),
            "this project has no linked originals of the selected kind"
        );
        let opened = app.project.lock();
        let current = opened.as_ref().context("no project is open")?;
        ensure!(
            Arc::ptr_eq(&current.session, &session),
            "the open project changed"
        );
        session.lock().apply(Edit {
            label: "relink_originals".to_owned(),
            ops,
        })?;
        Ok(())
    })
    .await??)
}

fn project_name(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// Deletes a project, closing it first if it's the open one.
#[tauri::command]
#[specta::specta]
pub fn delete_project(app: State<'_, Arc<App>>, name: String) -> Result<()> {
    if app
        .project
        .lock()
        .as_ref()
        .is_some_and(|project| project.name == name)
    {
        app.close();
    }
    Ok(app.library.delete(&name)?)
}

#[tauri::command]
#[specta::specta]
pub fn close(app: State<'_, Arc<App>>) {
    app.close();
}

/// The open project's name, for an interface that loaded after it opened.
#[tauri::command]
#[specta::specta]
pub fn get_open_project(app: State<'_, Arc<App>>) -> Option<String> {
    app.project
        .lock()
        .as_ref()
        .map(|project| project.name.clone())
}
