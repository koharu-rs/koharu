use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use koharu_desktop::{CanvasState, Desktop};
use koharu_scene::{AssetInput, AssetMetadata, AssetRole, At, PageDraft};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use specta::Type;
use strum::{EnumMessage as _, IntoEnumIterator as _};
use tauri::WebviewWindow;
use tauri_runtime_cef::CefRuntime;
use walkdir::WalkDir;

use super::{
    Channel, ChannelExt as _, Error,
    agent::AgentState,
    canvas::CanvasChannel,
    import,
    preferences::Preferences,
    processing::{Job, Processing},
    project::{
        CurrentProject, Page, PageSummary, Project, ProjectInfo, ProjectLibrary, ProjectSummary,
    },
};
use crate::host::Host;

#[derive(Clone, Debug, Serialize, Type)]
pub struct StartupState {
    pub preferences: Preferences,
    pub jobs: Vec<Job>,
    pub canvas: CanvasState,
    pub native_dialogs: bool,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct PageSelection {
    pub project: ProjectInfo,
    pub page: Page,
}

#[derive(Clone, Copy, Debug, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PageImportSource {
    Files,
    Folder,
}

#[derive(Clone)]
pub(crate) struct Initialization {
    ready: tokio::sync::watch::Sender<bool>,
}

impl Default for Initialization {
    fn default() -> Self {
        let (ready, _) = tokio::sync::watch::channel(false);
        Self { ready }
    }
}

impl Initialization {
    pub(crate) fn ready(&self) {
        self.ready.send_replace(true);
    }

    async fn wait(&self) -> Result<()> {
        let mut ready = self.ready.subscribe();
        while !*ready.borrow_and_update() {
            ready
                .changed()
                .await
                .context("startup state closed before initialization completed")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct Download {
    #[specta(type = f64)]
    pub id: u64,
    pub state: DownloadState,
    pub name: Option<String>,
    #[specta(type = f64)]
    pub completed: u64,
    #[specta(type = f64)]
    pub total: u64,
    pub error: Option<String>,
}

#[derive(Clone, Default)]
pub(crate) struct DownloadChannel {
    pub(crate) channel: Arc<Mutex<Option<Channel<Download>>>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum DownloadState {
    Running,
    Finished,
    Failed,
}

#[derive(Clone, Debug, Default, Serialize, Type)]
pub struct ModelResources {
    #[specta(type = f64)]
    pub process_memory: u64,
    #[specta(type = f64)]
    pub system_memory: u64,
    pub process_cpu: f32,
    pub devices: Vec<DeviceResources>,
}

#[derive(Clone, Default)]
pub(crate) struct ResourceChannel {
    pub(crate) channel: Arc<Mutex<Option<Channel<ModelResources>>>>,
}

#[derive(Clone, Default)]
pub(crate) struct ProjectChannel {
    pub(crate) channel: Arc<Mutex<Option<Channel<Option<ProjectInfo>>>>>,
}

#[derive(Clone, Debug, Default, Serialize, Type)]
pub struct DeviceResources {
    pub name: String,
    pub selected: bool,
    #[specta(type = Option<f64>)]
    pub memory_budget: Option<u64>,
    #[specta(type = Option<f64>)]
    pub memory_used: Option<u64>,
    pub utilization: Option<f32>,
}

impl From<koharu_pipeline::ResourceSnapshot> for ModelResources {
    fn from(value: koharu_pipeline::ResourceSnapshot) -> Self {
        Self {
            process_memory: value.process_memory_bytes,
            system_memory: value.system_memory_bytes,
            process_cpu: value.process_cpu_percent,
            devices: value
                .devices
                .into_iter()
                .map(|device| DeviceResources {
                    name: device.name,
                    selected: device.selected,
                    memory_budget: device.memory_budget_bytes,
                    memory_used: device.memory_used_bytes,
                    utilization: device.utilization_percent,
                })
                .collect(),
        }
    }
}

#[koharu_macros::command]
pub(crate) async fn subscribe(
    host: Host,
    on_canvas: Channel<CanvasState>,
    on_job: Channel<Job>,
    on_download: Channel<Download>,
    on_resources: Channel<ModelResources>,
    on_project: Channel<Option<ProjectInfo>>,
) -> std::result::Result<StartupState, Error> {
    host.initialization.wait().await?;

    *host.canvas.channel.lock() = Some(on_canvas);
    *host.jobs.channel.lock() = Some(on_job);
    *host.downloads.channel.lock() = Some(on_download);
    *host.resources.channel.lock() = Some(on_resources);
    *host.project_channel.channel.lock() = Some(on_project);

    let canvas_state = host.desktop.canvas_state();
    let preferences = Preferences::load()?;
    Ok(StartupState {
        preferences,
        jobs: host.processing.jobs.lock().values().cloned().collect(),
        canvas: canvas_state,
        native_dialogs: host.window().is_some(),
    })
}

async fn replace_project(
    state: &AgentState,
    processing: &Processing,
    project: &CurrentProject,
    desktop: &Desktop,
    canvas: &CanvasChannel,
    project_channel: &ProjectChannel,
    opened: Project,
) -> Result<()> {
    let snapshot = opened.snapshot();
    let page = opened.active_page();
    let info = opened.info();

    state.reset().await;
    for stop in processing.stops.lock().values() {
        stop.stop();
    }
    processing.stops.lock().clear();
    processing.jobs.lock().clear();

    let previous = {
        let mut current = project.project.lock().await;
        current.replace(opened)
    };

    desktop.show_page(&snapshot, page).await?;
    let canvas_state = desktop.canvas_state();
    drop(previous);
    canvas.channel.publish(canvas_state);
    project_channel.channel.publish(Some(info));
    Ok(())
}

#[koharu_macros::command]
pub(crate) async fn get_project(
    project: CurrentProject,
) -> std::result::Result<Option<ProjectInfo>, Error> {
    Ok(project.project.lock().await.as_ref().map(Project::info))
}

#[koharu_macros::command]
pub(crate) async fn get_pages(
    project: CurrentProject,
) -> std::result::Result<Vec<PageSummary>, Error> {
    let snapshot = project
        .project
        .lock()
        .await
        .as_ref()
        .context("no project is open")?
        .snapshot();
    Ok(Project::pages(&snapshot)?)
}

#[koharu_macros::command]
pub(crate) async fn get_page(project: CurrentProject) -> std::result::Result<Option<Page>, Error> {
    let current = {
        let project = project.project.lock().await;
        project
            .as_ref()
            .map(|project| (project.snapshot(), project.active_page()))
    };
    Ok(current
        .and_then(|(snapshot, page)| page.map(|page| (snapshot, page)))
        .map(|(snapshot, page)| Project::page(&snapshot, page))
        .transpose()?)
}

#[koharu_macros::command]
pub(crate) async fn list_projects(
    library: ProjectLibrary,
) -> std::result::Result<Vec<ProjectSummary>, Error> {
    Ok(library.list()?)
}

#[tracing::instrument(
    target = "koharu_metrics",
    name = "project_created",
    skip_all,
    fields(origin = "user")
)]
#[koharu_macros::command]
pub(crate) async fn create_project(
    name: String,
    library: ProjectLibrary,
    state: AgentState,
    processing: Processing,
    project: CurrentProject,
    desktop: Desktop,
    canvas: CanvasChannel,
    project_channel: ProjectChannel,
) -> std::result::Result<(), Error> {
    let opened = library.create(&name).await?;
    replace_project(
        &state,
        &processing,
        &project,
        &desktop,
        &canvas,
        &project_channel,
        opened,
    )
    .await?;
    Ok(())
}

#[tracing::instrument(
    target = "koharu_metrics",
    name = "project_opened",
    skip_all,
    fields(origin = "user")
)]
#[koharu_macros::command]
pub(crate) async fn open_project(
    name: String,
    library: ProjectLibrary,
    state: AgentState,
    processing: Processing,
    project: CurrentProject,
    desktop: Desktop,
    canvas: CanvasChannel,
    project_channel: ProjectChannel,
) -> std::result::Result<(), Error> {
    let opened = library.open(&name).await?;
    replace_project(
        &state,
        &processing,
        &project,
        &desktop,
        &canvas,
        &project_channel,
        opened,
    )
    .await?;
    Ok(())
}

#[tracing::instrument(
    target = "koharu_metrics",
    name = "project_closed",
    skip_all,
    fields(origin = "user")
)]
#[koharu_macros::command]
pub(crate) async fn close_project(
    state: AgentState,
    processing: Processing,
    project: CurrentProject,
    desktop: Desktop,
    canvas: CanvasChannel,
    project_channel: ProjectChannel,
) -> std::result::Result<(), Error> {
    close_current_project(
        &state,
        &processing,
        &project,
        &desktop,
        &canvas,
        &project_channel,
    )
    .await?;
    Ok(())
}

#[tracing::instrument(
    target = "koharu_metrics",
    name = "project_deleted",
    skip_all,
    fields(origin = "user")
)]
#[koharu_macros::command]
pub(crate) async fn delete_project(
    name: String,
    library: ProjectLibrary,
    state: AgentState,
    processing: Processing,
    project: CurrentProject,
    desktop: Desktop,
    canvas: CanvasChannel,
    project_channel: ProjectChannel,
) -> std::result::Result<(), Error> {
    let active = project
        .project
        .lock()
        .await
        .as_ref()
        .is_some_and(|project| project.name == name);
    if active {
        close_current_project(
            &state,
            &processing,
            &project,
            &desktop,
            &canvas,
            &project_channel,
        )
        .await?;
    }
    tokio::task::spawn_blocking(move || library.delete(&name))
        .await
        .context("project deletion task failed")??;
    Ok(())
}

async fn close_current_project(
    state: &AgentState,
    processing: &Processing,
    project: &CurrentProject,
    desktop: &Desktop,
    canvas: &CanvasChannel,
    project_channel: &ProjectChannel,
) -> Result<()> {
    state.reset().await;
    for stop in processing.stops.lock().values() {
        stop.stop();
    }
    processing.stops.lock().clear();
    processing.jobs.lock().clear();
    let previous = {
        let mut current = project.project.lock().await;
        current.take()
    };
    desktop.clear().await;
    let canvas_state = desktop.canvas_state();
    drop(previous);
    canvas.channel.publish(canvas_state);
    project_channel.channel.publish(None);
    Ok(())
}

#[tracing::instrument(
    target = "koharu_metrics",
    name = "import",
    skip_all,
    fields(origin = "user", method = ?source),
)]
#[koharu_macros::command]
pub(crate) async fn import(
    source: PageImportSource,
    window: WebviewWindow<CefRuntime>,
    paths: Option<Vec<String>>,
    desktop: Desktop,
    project: CurrentProject,
    processing: Processing,
    canvas: CanvasChannel,
) -> std::result::Result<(), Error> {
    if !processing.stops.lock().is_empty() {
        return Err(anyhow::anyhow!("pages cannot be imported while processing is running").into());
    }
    let files = resolve_import_paths(source, window, paths).await?;
    let Some(files) = files else {
        return Ok(());
    };
    if files.is_empty() {
        return Err(anyhow::anyhow!("no supported images were found in the selection").into());
    }
    let pages = tokio_rayon::spawn(move || import::import(files)).await?;
    let page_count = pages.len();

    let (commit, page) = {
        let mut project = project.project.lock().await;
        let project = project.as_mut().context("no project is open")?;
        let source = AssetRole::new("source")?;
        let patch = project.snapshot().patch(|edit| {
            for imported in pages {
                let page = edit.add_page(
                    PageDraft::new(
                        imported.name,
                        f64::from(imported.width),
                        f64::from(imported.height),
                    ),
                    At::End,
                )?;
                edit.set_asset(
                    page,
                    &source,
                    AssetInput::new(
                        imported.bytes,
                        imported.format.to_mime_type(),
                        AssetMetadata {
                            width: Some(imported.width),
                            height: Some(imported.height),
                            attributes: Default::default(),
                        },
                    ),
                )?;
            }
            Ok(())
        })?;
        let commit = project.session.commit(patch).await?;
        project.record(vec![commit.revision]);
        project.reconcile_page();
        let page = project.active_page();
        (commit, page)
    };
    desktop.synchronize(&commit.snapshot, page, &commit).await?;
    let canvas_state = desktop.canvas_state();
    canvas.channel.publish(canvas_state);
    tracing::info!(target: "koharu_metrics", metric = "page_imported", page_count);
    Ok(())
}

async fn resolve_import_paths(
    source: PageImportSource,
    window: Option<WebviewWindow<CefRuntime>>,
    paths: Option<Vec<String>>,
) -> Result<Option<Vec<PathBuf>>, Error> {
    if let Some(paths) = paths.filter(|paths| !paths.is_empty()) {
        let paths =
            crate::host::sanitize_import_paths(paths.into_iter().map(PathBuf::from).collect())?;
        if matches!(source, PageImportSource::Folder) && paths.len() == 1 && paths[0].is_dir() {
            return Ok(Some(collect_folder_images(&paths[0])));
        }
        return Ok(Some(paths));
    }
    let Some(window) = window else {
        return Err(anyhow::anyhow!("no files to import").into());
    };
    let extensions = import::Format::iter()
        .flat_map(|format| format.get_serializations())
        .collect::<Vec<_>>();
    let dialog = rfd::AsyncFileDialog::new()
        .add_filter("Images, archives, and PDF", &extensions)
        .set_parent(&window);
    Ok(match source {
        PageImportSource::Files => dialog.pick_files().await.map(|files| {
            files
                .into_iter()
                .map(|file| file.path().to_owned())
                .collect::<Vec<_>>()
        }),
        PageImportSource::Folder => dialog
            .pick_folder()
            .await
            .map(|folder| collect_folder_images(folder.path())),
    })
}

fn collect_folder_images(folder: &std::path::Path) -> Vec<PathBuf> {
    WalkDir::new(folder)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| match entry {
            Ok(entry) if entry.file_type().is_file() => Some(entry.into_path()),
            Ok(_) => None,
            Err(error) => {
                tracing::warn!(%error, "could not inspect an import directory entry");
                None
            }
        })
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.parse::<import::Format>().is_ok())
        })
        .collect()
}

#[tracing::instrument(
    target = "koharu_metrics",
    name = "page_selected",
    skip_all,
    fields(origin = "user")
)]
#[koharu_macros::command]
pub(crate) async fn select_page(
    desktop: Desktop,
    page: koharu_scene::EntityId,
    project: CurrentProject,
    canvas: CanvasChannel,
) -> std::result::Result<PageSelection, Error> {
    let (snapshot, project_info, selected_page) = {
        let mut project = project.project.lock().await;
        let project = project.as_mut().context("no project is open")?;
        project.select_page(page)?;
        let snapshot = project.snapshot();
        let project_info = project.info();
        let selected_page = Project::page(&snapshot, page)?;
        (snapshot, project_info, selected_page)
    };
    if desktop.show_page(&snapshot, Some(page)).await? {
        let canvas_state = desktop.canvas_state();
        canvas.channel.publish(canvas_state);
    }
    Ok(PageSelection {
        project: project_info,
        page: selected_page,
    })
}
