//! The gallery's commands: searching the sites the built-in extensions read
//! and browsing their catalogs, a series through the extension that reads
//! its site, signing in to a site and who is, and reading a chapter.

use std::sync::Arc;

use anyhow::Context as _;
use koharu_extensions::{Catalog, EXTENSIONS, Extension, Listing, Series};
use parking_lot::Mutex;
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager as _, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_runtime_cef::CefRuntime;
use tokio::sync::oneshot;
use url::Url;

use super::{Bytes, Result};
use crate::{
    app::App,
    gallery::{self, Reading},
};

/// A site the gallery searches and browses.
#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Site {
    pub id: String,
    pub name: String,
    /// The catalogs it has besides its search.
    pub catalogs: Vec<Catalog>,
    /// Whether it has a sign-in.
    pub sign_in: bool,
}

/// Every site the built-in extensions read.
#[tauri::command]
#[specta::specta]
pub fn list_sites() -> Vec<Site> {
    EXTENSIONS
        .iter()
        .map(|extension| Site {
            id: extension.id().to_owned(),
            name: extension.name().to_owned(),
            catalogs: extension.catalogs().to_vec(),
            sign_in: extension.sign_in().is_some(),
        })
        .collect()
}

/// The page of the site `site`'s search for `query` that `next` starts.
#[tauri::command]
#[specta::specta]
pub async fn search_site(
    http: State<'_, reqwest::Client>,
    site: String,
    query: String,
    next: Option<String>,
) -> Result<Listing> {
    tracing::info!(site, "search site");
    Ok(extension(&site)?
        .search(&http, &query, next.as_deref())
        .await?)
}

/// The page of the site `site`'s `catalog` that `next` starts.
#[tauri::command]
#[specta::specta]
pub async fn browse_site(
    http: State<'_, reqwest::Client>,
    site: String,
    catalog: Catalog,
    next: Option<String>,
) -> Result<Listing> {
    tracing::info!(site, ?catalog, "browse site");
    Ok(extension(&site)?
        .catalog(&http, catalog, next.as_deref())
        .await?)
}

/// A series the gallery opened.
#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    pub url: Url,
    pub series: Series,
}

/// The series at `url`.
#[tauri::command]
#[specta::specta]
pub async fn open_series(http: State<'_, reqwest::Client>, url: Url) -> Result<Opened> {
    let extension = reader(&url)?;
    tracing::info!(extension = extension.id(), "open series");
    let series = extension.series(&http, &url).await?;
    Ok(Opened { url, series })
}

/// The cover at `url` of the series at `series`, through its extension, as
/// sites refuse it to the main window's page.
#[tauri::command]
#[specta::specta]
pub async fn get_cover(http: State<'_, reqwest::Client>, series: Url, url: Url) -> Result<Bytes> {
    Ok(Bytes(reader(&series)?.cover(&http, &url).await?))
}

/// Opens the sign-in page of the site `site` in a window of its own, and
/// answers once the user closes it, signed in or not; while it's open,
/// another call brings it forward and answers at once.
#[tauri::command]
#[specta::specta]
pub async fn sign_in_site(handle: AppHandle<CefRuntime>, site: String) -> Result<()> {
    let extension = extension(&site)?;
    tracing::info!(extension = extension.id(), "sign in to a site");
    let page = extension
        .sign_in()
        .with_context(|| format!("{} has no sign-in", extension.name()))?;
    let label = format!("sign-in-{}", extension.id());
    if let Some(window) = handle.get_webview_window(&label) {
        return Ok(window.set_focus()?);
    }
    let window = WebviewWindowBuilder::new(&handle, label, WebviewUrl::External(Url::parse(page)?))
        .title(extension.name())
        .inner_size(480.0, 720.0)
        .center()
        .build()?;
    let (closed, closing) = oneshot::channel();
    let closed = Mutex::new(Some(closed));
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed)
            && let Some(closed) = closed.lock().take()
        {
            // A caller that stopped waiting has no one to tell.
            let _ = closed.send(());
        }
    });
    // The window dropping its handler unsent went as well.
    let _ = closing.await;
    Ok(())
}

/// The name of the account signed in to the site `site`, `None` when no one
/// is.
#[tauri::command]
#[specta::specta]
pub async fn get_account(http: State<'_, reqwest::Client>, site: String) -> Result<Option<String>> {
    Ok(extension(&site)?.account(&http).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn sign_out_site(handle: AppHandle<CefRuntime>, site: String) -> Result<()> {
    let extension = extension(&site)?;
    let page = extension
        .sign_in()
        .with_context(|| format!("{} has no sign-in", extension.name()))?;
    tracing::info!(extension = extension.id(), "sign out of a site");
    if let Some(window) = handle.get_webview_window(&format!("sign-in-{}", extension.id())) {
        window.close()?;
    }
    Ok(gallery::sign_out(&handle, Url::parse(page)?).await?)
}

/// Opens the chapter at `url` as the open project, and loads its pages into
/// it, which `chapter` tells how far it went with; says how it reads.
#[tauri::command]
#[specta::specta]
pub async fn read_chapter(
    app: State<'_, Arc<App>>,
    http: State<'_, reqwest::Client>,
    url: Url,
) -> Result<Reading> {
    Ok(gallery::read(&app, reader(&url)?, http.inner().clone(), &url).await?)
}

/// The built-in extension `id`.
fn extension(id: &str) -> anyhow::Result<&'static dyn Extension> {
    EXTENSIONS
        .iter()
        .copied()
        .find(|extension| extension.id() == id)
        .with_context(|| format!("no extension is {id}"))
}

/// The built-in extension that reads `url`.
fn reader(url: &Url) -> anyhow::Result<&'static dyn Extension> {
    koharu_extensions::find(url)
        .with_context(|| format!("no extension reads {}", url.host_str().unwrap_or_default()))
}
