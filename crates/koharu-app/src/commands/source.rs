mod cache;
mod document;

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use anyhow::{Context, Result, bail};
use futures::{StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, Rect, State, Webview, WebviewUrl,
    ipc::Channel,
};
use tauri_runtime_cef::CefRuntime;
use tokio::sync::Mutex;
use url::Url;

use super::{
    Error, import, lifecycle,
    processing::{JobId, Processing},
    project::{CurrentProject, Project, ProjectLibrary},
};

const BROWSER: &str = "rawkuma-source";
const HOME: &str = "https://rawkuma.net/";

#[revision::revisioned(revision = 1)]
#[derive(Clone)]
struct ChapterSource {
    url: String,
}

impl koharu_scene::Component for ChapterSource {
    const KIND: &'static str = "dev.koharu.rawkuma.chapter";

    fn validate(&self, _: &koharu_scene::ValidationContext<'_>) -> koharu_scene::Result<()> {
        if self.url.len() <= 8192 && Url::parse(&self.url).is_ok_and(|url| source_url(&url)) {
            Ok(())
        } else {
            Err(koharu_scene::Error::Invalid(
                "invalid Rawkuma chapter URL".into(),
            ))
        }
    }
}

pub(super) fn project_url(snapshot: &koharu_scene::Snapshot) -> Result<Option<String>> {
    Ok(snapshot
        .project_component::<ChapterSource>()?
        .map(|source| source.url))
}

async fn remember_source(project: &mut Project, url: &str) -> Result<()> {
    let snapshot = project.snapshot();
    if let Some(existing) = project_url(&snapshot)? {
        if existing != url {
            bail!("the cached project belongs to a different Rawkuma chapter");
        }
        return Ok(());
    }
    // The chapter link belongs to the document, not the disposable HTTP cache.
    let patch = snapshot.patch(|edit| edit.set_project(&ChapterSource { url: url.into() }))?;
    project.session.commit(patch).await?;
    Ok(())
}

#[derive(Default)]
pub(crate) struct SourceState {
    operation: Mutex<()>,
    browser: Mutex<()>,
    pending: parking_lot::Mutex<Option<PendingTranslation>>,
}

struct PendingTranslation {
    project: String,
    path: PathBuf,
    settings: String,
    job: Option<JobId>,
}

impl SourceState {
    fn prepared(&self, project: String, path: PathBuf, settings: String) {
        *self.pending.lock() = Some(PendingTranslation {
            project,
            path,
            settings,
            job: None,
        });
    }

    pub(super) fn begin_translation(&self, project: &str, job: JobId, settings: &str) {
        if let Some(pending) = self
            .pending
            .lock()
            .as_mut()
            .filter(|pending| pending.project == project && pending.settings == settings)
        {
            pending.job = Some(job);
        }
    }

    pub(super) async fn finish_translation(
        &self,
        job: JobId,
        success: bool,
        revision: Option<koharu_scene::Revision>,
    ) -> Result<()> {
        let pending = {
            let mut pending = self.pending.lock();
            if pending
                .as_ref()
                .is_some_and(|pending| pending.job == Some(job))
            {
                pending.take()
            } else {
                None
            }
        };
        if success
            && let Some(pending) = pending
            && let Some(revision) = revision
        {
            cache::write_json(
                &pending.path,
                &cache::CachedProject {
                    name: pending.project,
                    complete: true,
                    revision: Some(revision),
                },
            )
            .await?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Deserialize, Type)]
pub struct SourceBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl SourceBounds {
    fn rect(self) -> Result<Rect> {
        if [self.x, self.y, self.width, self.height]
            .iter()
            .any(|v| !v.is_finite())
            || self.x < 0.0
            || self.y < 0.0
            || self.width < 1.0
            || self.height < 1.0
        {
            bail!("invalid source browser bounds");
        }
        Ok(Rect {
            position: LogicalPosition::new(self.x, self.y).into(),
            size: LogicalSize::new(self.width, self.height).into(),
        })
    }
}

#[derive(Clone, Deserialize, Serialize, Type)]
pub struct SourcePage {
    pub url: String,
    pub title: String,
    pub images: Vec<String>,
}

#[derive(Clone, Serialize, Type)]
pub struct SourceProgress {
    pub completed: u32,
    pub total: u32,
}

#[derive(Serialize, Type)]
pub struct SourceTranslation {
    pub project: String,
    pub cached: bool,
    pub pages: u32,
}

fn trusted(webview: &Webview<CefRuntime>) -> Result<()> {
    if webview.label() != "main" {
        bail!("source commands are only available to the Koharu editor");
    }
    Ok(())
}

fn source_url(url: &Url) -> bool {
    url.scheme() == "https"
        && matches!(url.host_str(), Some("rawkuma.net" | "www.rawkuma.net"))
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
}

fn root() -> Result<PathBuf> {
    Ok(dirs::cache_dir()
        .context("the cache directory is unavailable")?
        .join("koharu")
        .join("rawkuma"))
}

pub(super) fn settings_fingerprint() -> Result<String> {
    let pipeline = koharu_config::load::<koharu_pipeline::PipelineConfig>("pipeline")?;
    let typesetting = koharu_renderer::TypesettingConfig::load()?;
    let providers = koharu_config::load::<koharu_translator::ProvidersConfig>("providers")?;
    Ok(cache::digest(&serde_json::to_vec(&(
        &*pipeline.read()?,
        &*typesetting.read()?,
        &*providers.read()?,
    ))?))
}

async fn inspect(handle: &AppHandle<CefRuntime>) -> Result<SourcePage> {
    let view = handle
        .get_webview(BROWSER)
        .context("open the Rawkuma source browser first")?;
    let url = view.url()?;
    if !source_url(&url) {
        bail!("the browser is not on Rawkuma");
    }
    let html = document::read(&view).await?;
    if view.url()? != url {
        bail!("the source page changed during inspection");
    }
    document::parse(url, &html)
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn open_source_browser(
    webview: Webview<CefRuntime>,
    handle: AppHandle<CefRuntime>,
    state: State<'_, SourceState>,
    bounds: SourceBounds,
    url: Option<String>,
) -> Result<(), Error> {
    trusted(&webview)?;
    let url = url.map(|url| Url::parse(&url)).transpose()?;
    if url.as_ref().is_some_and(|url| !source_url(url)) {
        return Err(anyhow::anyhow!("enter an HTTPS Rawkuma URL").into());
    }
    let _browser = state.browser.lock().await;
    let rect = bounds.rect()?;
    if let Some(view) = handle.get_webview(BROWSER) {
        view.set_bounds(rect)?;
        if let Some(url) = url
            && view.url()? != url
        {
            view.navigate(url)?;
        }
        view.show()?;
    } else {
        let window = handle
            .get_window("main")
            .context("the editor window is unavailable")?;
        window.add_child(
            tauri::webview::WebviewBuilder::new(
                BROWSER,
                WebviewUrl::External(url.unwrap_or(Url::parse(HOME)?)),
            )
            .bounds(rect)
            .initialization_script(include_str!(
                "../../../../packages/bridge/src/rawkuma-navigation.js"
            ))
            .data_directory(
                dirs::cache_dir()
                    .context("the cache directory is unavailable")?
                    .join("koharu")
                    .join("rawkuma-browser"),
            )
            .on_navigation(source_url)
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny),
        )?;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn resize_source_browser(
    webview: Webview<CefRuntime>,
    handle: AppHandle<CefRuntime>,
    state: State<'_, SourceState>,
    bounds: SourceBounds,
) -> Result<(), Error> {
    trusted(&webview)?;
    let _browser = state.browser.lock().await;
    if let Some(view) = handle.get_webview(BROWSER) {
        view.set_bounds(bounds.rect()?)?;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn close_source_browser(
    webview: Webview<CefRuntime>,
    handle: AppHandle<CefRuntime>,
    state: State<'_, SourceState>,
) -> Result<(), Error> {
    trusted(&webview)?;
    let _browser = state.browser.lock().await;
    if let Some(view) = handle.get_webview(BROWSER) {
        view.hide()?;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn navigate_source_browser(
    webview: Webview<CefRuntime>,
    handle: AppHandle<CefRuntime>,
    url: String,
) -> Result<(), Error> {
    trusted(&webview)?;
    let url = Url::parse(&url)?;
    if !source_url(&url) {
        return Err(anyhow::anyhow!("enter an HTTPS Rawkuma URL").into());
    }
    handle
        .get_webview(BROWSER)
        .context("the source browser is closed")?
        .navigate(url)?;
    Ok(())
}

#[derive(Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SourceBrowserAction {
    Back,
    Forward,
    Reload,
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn source_browser_action(
    webview: Webview<CefRuntime>,
    handle: AppHandle<CefRuntime>,
    action: SourceBrowserAction,
) -> Result<(), Error> {
    trusted(&webview)?;
    let script = match action {
        SourceBrowserAction::Back => "history.back()",
        SourceBrowserAction::Forward => "history.forward()",
        SourceBrowserAction::Reload => "location.reload()",
    };
    handle
        .get_webview(BROWSER)
        .context("the source browser is closed")?
        .eval(script)?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn inspect_source_page(
    webview: Webview<CefRuntime>,
    handle: AppHandle<CefRuntime>,
) -> Result<SourcePage, Error> {
    trusted(&webview)?;
    Ok(inspect(&handle).await?)
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn prepare_source_translation(
    webview: Webview<CefRuntime>,
    handle: AppHandle<CefRuntime>,
    state: State<'_, SourceState>,
    revalidate: bool,
    replace: bool,
    on_progress: Channel<SourceProgress>,
) -> Result<SourceTranslation, Error> {
    trusted(&webview)?;
    let _operation = state
        .operation
        .try_lock()
        .context("another source operation is running")?;
    ensure_idle(&handle)?;
    let page = inspect(&handle).await?;
    if page.images.is_empty() {
        return Err(
            anyhow::anyhow!("open a chapter containing manga pages before translating").into(),
        );
    }
    let root = root()?;
    let client = cache::client()?;
    let count = page.images.len() as u32;
    let completed = Arc::new(AtomicU32::new(0));
    let _ = on_progress.send(SourceProgress {
        completed: 0,
        total: count,
    });
    let images = futures::stream::iter(page.images.clone().into_iter().map(|url| {
        let completed = completed.clone();
        let root = root.clone();
        let client = client.clone();
        let referer = page.url.clone();
        let on_progress = on_progress.clone();
        async move {
            let image =
                cache::download(&client, &root, Url::parse(&url)?, &referer, revalidate).await?;
            let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
            let _ = on_progress.send(SourceProgress {
                completed: done,
                total: count,
            });
            Ok::<_, anyhow::Error>(image)
        }
    }))
    .buffered(3)
    .try_collect::<Vec<_>>()
    .await?;
    ensure_idle(&handle)?;
    let settings = settings_fingerprint()?;
    // Results are bound to ordered source bytes AND the model/language/instructions/layout settings.
    let fingerprint = cache::digest(&serde_json::to_vec(&(
        "rawkuma-v1",
        &page.url,
        images
            .iter()
            .map(|image| (&image.url, &image.digest))
            .collect::<Vec<_>>(),
        &settings,
    ))?);
    let entry_path = root.join("projects").join(format!("{fingerprint}.json"));
    let existing = cache::read_json::<cache::CachedProject>(&entry_path).await?;
    let library = handle.state::<ProjectLibrary>().inner().clone();
    if !replace && let Some(mut entry) = existing {
        let active = handle
            .state::<CurrentProject>()
            .project
            .lock()
            .await
            .as_ref()
            .filter(|project| project.name == entry.name)
            .map(|project| {
                let snapshot = project.snapshot();
                (snapshot.revision(), snapshot.pages().count())
            });
        if let Some((revision, page_count)) = active
            && page_count == images.len()
        {
            let current = handle.state::<CurrentProject>();
            let mut current = current.project.lock().await;
            let project = current
                .as_mut()
                .filter(|project| project.name == entry.name)
                .context("the cached project was closed")?;
            remember_source(project, &page.url).await?;
            let cached = entry.complete && entry.revision == Some(revision);
            if cached && project.revision() != revision {
                entry.revision = Some(project.revision());
                cache::write_json(&entry_path, &entry).await?;
            }
            state.prepared(entry.name.clone(), entry_path, settings);
            return Ok(SourceTranslation {
                project: entry.name,
                cached,
                pages: count,
            });
        }
        if active.is_none()
            && library
                .list()?
                .iter()
                .any(|project| project.name == entry.name)
        {
            let mut opened = library.open(&entry.name).await?;
            let snapshot = opened.snapshot();
            if snapshot.pages().count() == images.len() {
                let cached = entry.complete && entry.revision == Some(snapshot.revision());
                remember_source(&mut opened, &page.url).await?;
                if cached && opened.revision() != snapshot.revision() {
                    entry.revision = Some(opened.revision());
                    cache::write_json(&entry_path, &entry).await?;
                }
                lifecycle::replace_project(&handle, opened).await?;
                state.prepared(entry.name.clone(), entry_path, settings);
                return Ok(SourceTranslation {
                    project: entry.name,
                    cached,
                    pages: count,
                });
            }
        }
    }
    // Import copies content into the durable project. Clearing the HTTP cache cannot remove it.
    let directory = tempfile::tempdir()?;
    let mut paths = Vec::new();
    for (index, image) in images.iter().enumerate() {
        let path = directory
            .path()
            .join(format!("{:04}.{}", index + 1, image.extension));
        tokio::fs::copy(image.path(&root), &path).await?;
        paths.push(path);
    }
    let pages = tokio_rayon::spawn(move || import::import(paths)).await?;
    let title: String = page
        .title
        .chars()
        .filter(|c| !c.is_control() && !"<>:\"/\\|?*".contains(*c))
        .scan(0, |bytes, c| {
            *bytes += c.len_utf8();
            (*bytes <= 180).then_some(c)
        })
        .take(80)
        .collect();
    let title = title.trim().trim_matches('.');
    let language = koharu_pipeline::PipelineConfig::load()?
        .read()?
        .translation
        .target_language;
    let name = format!(
        "Rawkuma [{}] - {} - {}",
        language.tag(),
        if title.is_empty() { "Chapter" } else { title },
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let mut opened = library.create(&name).await?;
    remember_source(&mut opened, &page.url).await?;
    lifecycle::append_imported_pages(&mut opened, pages).await?;
    lifecycle::replace_project(&handle, opened).await?;
    cache::write_json(
        &entry_path,
        &cache::CachedProject {
            name: name.clone(),
            complete: false,
            revision: None,
        },
    )
    .await?;
    state.prepared(name.clone(), entry_path, settings);
    Ok(SourceTranslation {
        project: name,
        cached: false,
        pages: count,
    })
}

fn ensure_idle(handle: &AppHandle<CefRuntime>) -> Result<()> {
    if !handle.state::<Processing>().stops.lock().is_empty() {
        bail!("wait for the current processing job to finish");
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn clear_source_cache(
    webview: Webview<CefRuntime>,
    handle: AppHandle<CefRuntime>,
    state: State<'_, SourceState>,
) -> Result<(), Error> {
    trusted(&webview)?;
    let _operation = state
        .operation
        .try_lock()
        .context("wait for the source operation to finish")?;
    ensure_idle(&handle)?;
    let root = root()?;
    // Only disposable cache and its lookup index are removed. Durable .khrproj projects remain.
    for directory in ["images", "metadata", "projects"] {
        match tokio::fs::remove_dir_all(root.join(directory)).await {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    state.pending.lock().take();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn chapter_url_survives_reopening_and_cannot_be_reassigned_by_a_cache_hit() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("chapter.khrproj");
        let url = "https://rawkuma.net/manga/example/chapter-38-5/";
        let mut project = Project::create("chapter".into(), path.clone())
            .await
            .unwrap();
        assert_eq!(project.info().unwrap().source_url, None);
        remember_source(&mut project, url).await.unwrap();
        let revision = project.revision();
        remember_source(&mut project, url).await.unwrap();
        assert_eq!(project.revision(), revision);
        assert!(
            remember_source(&mut project, "https://rawkuma.net/manga/other/")
                .await
                .is_err()
        );
        drop(project);
        let reopened = Project::open("chapter".into(), path).await.unwrap();
        assert_eq!(reopened.info().unwrap().source_url.as_deref(), Some(url));
    }

    #[tokio::test]
    async fn chapter_metadata_rejects_urls_outside_the_source_browser_boundary() {
        let root = tempfile::tempdir().unwrap();
        let mut project = Project::create("test".into(), root.path().join("test.khrproj"))
            .await
            .unwrap();
        for url in [
            "http://rawkuma.net/",
            "https://evil.test/",
            "https://rawkuma.net.evil.test/",
            "https://user@rawkuma.net/",
            "javascript:alert(1)",
        ] {
            assert!(remember_source(&mut project, url).await.is_err());
        }
        assert_eq!(project.info().unwrap().source_url, None);
    }

    #[tokio::test]
    async fn only_the_matching_successful_pipeline_job_publishes_a_complete_cache() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("project.json");
        let state = SourceState::default();
        let first = JobId::new();
        state.prepared("chapter".into(), path.clone(), "settings".into());
        state.begin_translation("chapter", first, "another language or model");
        state
            .finish_translation(first, true, Some(koharu_scene::Revision::new(2)))
            .await
            .unwrap();
        assert!(!path.exists());
        state.begin_translation("chapter", first, "settings");
        state
            .finish_translation(JobId::new(), true, Some(koharu_scene::Revision::new(2)))
            .await
            .unwrap();
        assert!(!path.exists());
        state
            .finish_translation(first, false, Some(koharu_scene::Revision::new(2)))
            .await
            .unwrap();
        assert!(!path.exists());
        let retry = JobId::new();
        state.prepared("chapter".into(), path.clone(), "settings".into());
        state.begin_translation("another project", retry, "settings");
        state
            .finish_translation(retry, true, Some(koharu_scene::Revision::new(2)))
            .await
            .unwrap();
        assert!(!path.exists());
        state.begin_translation("chapter", retry, "settings");
        state
            .finish_translation(retry, true, Some(koharu_scene::Revision::new(2)))
            .await
            .unwrap();
        let entry: cache::CachedProject = cache::read_json(&path).await.unwrap().unwrap();
        assert_eq!(entry.name, "chapter");
        assert!(entry.complete);
    }
}
