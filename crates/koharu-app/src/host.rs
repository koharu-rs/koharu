use std::{
    ops::Deref,
    path::{Component, Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Context as _, Result, anyhow};
use axum::{
    body::Body,
    extract::{
        FromRequest, Multipart, Request,
        ws::{Message, WebSocket},
    },
    http::{
        StatusCode,
        header::{CONTENT_DISPOSITION, CONTENT_TYPE, HeaderValue},
    },
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tauri::{Manager, WebviewWindow};
use tauri_runtime_cef::CefRuntime;
use tokio::sync::mpsc;

use crate::channel::Channel;
use crate::commands::{
    ChannelExt as _, Error,
    agent::AgentState,
    canvas::CanvasChannel,
    lifecycle::{
        Download, DownloadChannel, DownloadState, Initialization, ModelResources, ProjectChannel,
        ResourceChannel,
    },
    processing::{JobChannel, Processing},
    project::{CurrentProject, ProjectLibrary},
};

/// Shared application objects for the desktop runtime and HTTP/WebSocket state.
#[derive(Clone)]
pub struct Host {
    pub(crate) project: CurrentProject,
    pub(crate) library: ProjectLibrary,
    pub(crate) processing: Processing,
    pub(crate) canvas: CanvasChannel,
    pub(crate) jobs: JobChannel,
    pub(crate) downloads: DownloadChannel,
    pub(crate) resources: ResourceChannel,
    pub(crate) project_channel: ProjectChannel,
    pub(crate) initialization: Initialization,
    pub(crate) desktop: koharu_desktop::Desktop,
    pub(crate) state: AgentState,
    pub(crate) pipeline: Pipeline,
    ws: Arc<parking_lot::Mutex<Option<WsSession>>>,
    ws_generation: Arc<AtomicU64>,
    window: Arc<parking_lot::Mutex<Option<WebviewWindow<CefRuntime>>>>,
    server_shutdown: Arc<tokio::sync::Notify>,
}

/// Shared pipeline handle. `host.pipeline.clone()` is injected into commands named `pipeline`.
#[derive(Clone)]
pub(crate) struct Pipeline(Arc<OnceLock<koharu_pipeline::Pipeline>>);

impl Deref for Pipeline {
    type Target = koharu_pipeline::Pipeline;

    fn deref(&self) -> &Self::Target {
        self.0.get().expect("pipeline is not initialized")
    }
}

struct WsSession {
    generation: u64,
    abort: tokio::task::AbortHandle,
    tx: mpsc::UnboundedSender<Value>,
}

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug)]
pub struct ApiError(String);

impl From<Error> for ApiError {
    fn from(error: Error) -> Self {
        Self(error.to_string())
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, self.0).into_response()
    }
}

/// JSON body or multipart form, used by import so `/rpc/import` can take files.
pub struct JsonOrMultipart<T> {
    pub payload: T,
    pub files: Option<tempfile::TempDir>,
}

/// Typed download returned by optional-binary HTTP commands such as export.
pub struct HttpFile {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
    pub filename: Option<String>,
}

pub fn binary_response(value: impl Into<HttpFile>) -> Response {
    let file = value.into();
    let mut builder = Response::builder().header(CONTENT_TYPE, file.content_type);
    if let Some(filename) = file.filename.as_deref().and_then(disposition_filename) {
        builder = builder.header(CONTENT_DISPOSITION, filename);
    }
    builder
        .body(Body::from(file.bytes))
        .expect("binary response")
}

fn disposition_filename(name: &str) -> Option<HeaderValue> {
    let sanitized = name.replace(['"', '\\', '\r', '\n'], "_");
    HeaderValue::from_str(&format!("attachment; filename=\"{sanitized}\"")).ok()
}

pub struct WsSink {
    tx: mpsc::UnboundedSender<Value>,
    session: Arc<parking_lot::Mutex<Option<WsSession>>>,
    generation: u64,
}

impl WsSink {
    pub fn channel<T: serde::Serialize>(&self, name: &str) -> Channel<T> {
        let tx = self.tx.clone();
        let channel = envelope_channel("subscribe", name)
            .unwrap_or_else(|error| panic!("{error}"))
            .to_owned();
        Channel::from_sink(move |payload| tx.send(envelope(&channel, payload)).is_ok())
    }

    pub fn complete<T, E>(self, result: Result<T, E>)
    where
        T: serde::Serialize,
        E: std::fmt::Display,
    {
        match result {
            Ok(value) => {
                if let Ok(payload) = serde_json::to_value(&value) {
                    let _ = self.tx.send(envelope("startup", payload));
                } else {
                    let _ = self
                        .tx
                        .send(error_envelope("failed to serialize startup state"));
                    self.finish_session();
                }
            }
            Err(error) => {
                let _ = self.tx.send(error_envelope(&error));
                self.finish_session();
            }
        }
    }

    fn finish_session(self) {
        take_ws_session(&self.session, self.generation);
    }
}

fn take_ws_session(slot: &parking_lot::Mutex<Option<WsSession>>, generation: u64) {
    let mut slot = slot.lock();
    if slot
        .as_ref()
        .is_some_and(|session| session.generation == generation)
    {
        slot.take();
    }
}

pub fn ws_channel<T: serde::Serialize>(
    host: &Host,
    command_name: &str,
    param_name: &str,
) -> Result<Channel<T>, ApiError> {
    let channel = envelope_channel(command_name, param_name)?.to_owned();
    let tx = host
        .ws
        .lock()
        .as_ref()
        .map(|session| session.tx.clone())
        .ok_or_else(|| ApiError("no active subscribe session".into()))?;
    Ok(Channel::from_sink(move |payload| {
        tx.send(envelope(&channel, payload)).is_ok()
    }))
}

pub(crate) fn envelope_channel(command: &str, param: &str) -> Result<&'static str, ApiError> {
    match (command, param) {
        ("subscribe", "on_canvas") => Ok("canvas"),
        ("subscribe", "on_job") => Ok("job"),
        ("subscribe", "on_download") => Ok("download"),
        ("subscribe", "on_resources") => Ok("resources"),
        ("subscribe", "on_project") => Ok("project"),
        ("login_agent", "on_event") => Ok("agent_login"),
        ("run_agent", "on_event") => Ok("agent_event"),
        _ => Err(ApiError(format!("unknown channel {command}/{param}"))),
    }
}

fn envelope(channel: &str, payload: Value) -> Value {
    serde_json::json!({ "channel": channel, "payload": payload })
}

fn error_envelope(error: impl std::fmt::Display) -> Value {
    serde_json::json!({ "channel": "startup", "error": error.to_string() })
}

pub fn ws_sink(host: &Host, socket: WebSocket) -> WsSink {
    let generation = host.ws_generation.fetch_add(1, Ordering::Relaxed) + 1;
    if let Some(previous) = host.ws.lock().take() {
        previous.abort.abort();
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    let (mut sink, mut stream) = socket.split();
    let host_ws = host.ws.clone();
    let abort = tokio::spawn(async move {
        loop {
            tokio::select! {
                outbound = rx.recv() => {
                    let Some(value) = outbound else {
                        break;
                    };
                    let text = match serde_json::to_string(&value) {
                        Ok(text) => text,
                        Err(error) => {
                            tracing::warn!(%error, "failed to serialize WebSocket envelope");
                            continue;
                        }
                    };
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                incoming = stream.next() => {
                    match incoming {
                        Some(Ok(Message::Ping(payload))) => {
                            if sink.send(Message::Pong(payload)).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Ok(_)) => {}
                        Some(Err(_)) => break,
                    }
                }
            }
        }
        take_ws_session(&host_ws, generation);
    })
    .abort_handle();

    *host.ws.lock() = Some(WsSession {
        generation,
        abort,
        tx: tx.clone(),
    });
    WsSink {
        tx,
        session: host.ws.clone(),
        generation,
    }
}

impl Host {
    pub fn new() -> Result<Self> {
        let project = CurrentProject {
            project: Arc::new(tokio::sync::Mutex::new(None)),
        };
        let processing = Processing::default();
        let canvas = CanvasChannel::default();
        let desktop = koharu_desktop::Desktop::new()?;
        let pipeline = Pipeline(Arc::new(OnceLock::new()));
        let state = AgentState::new(
            project.clone(),
            desktop.clone(),
            canvas.clone(),
            processing.clone(),
            pipeline.clone(),
        )?;
        Ok(Self::assemble(
            project,
            ProjectLibrary::new()?,
            processing,
            canvas,
            JobChannel::default(),
            DownloadChannel::default(),
            ResourceChannel::default(),
            ProjectChannel::default(),
            Initialization::default(),
            desktop,
            state,
            pipeline,
        ))
    }

    // Field-for-field assembler keeps Host construction off AppHandle so
    // desktop and headless share state without manager cycles.
    #[allow(clippy::too_many_arguments)]
    fn assemble(
        project: CurrentProject,
        library: ProjectLibrary,
        processing: Processing,
        canvas: CanvasChannel,
        jobs: JobChannel,
        downloads: DownloadChannel,
        resources: ResourceChannel,
        project_channel: ProjectChannel,
        initialization: Initialization,
        desktop: koharu_desktop::Desktop,
        state: AgentState,
        pipeline: Pipeline,
    ) -> Self {
        Self {
            project,
            library,
            processing,
            canvas,
            jobs,
            downloads,
            resources,
            project_channel,
            initialization,
            desktop,
            state,
            pipeline,
            ws: Arc::new(parking_lot::Mutex::new(None)),
            ws_generation: Arc::new(AtomicU64::new(0)),
            window: Arc::new(parking_lot::Mutex::new(None)),
            server_shutdown: Arc::new(tokio::sync::Notify::new()),
        }
    }

    pub fn install(&self, manager: &impl Manager<CefRuntime>) {
        manager.manage(self.clone());
    }

    pub fn set_window(&self, window: WebviewWindow<CefRuntime>) {
        *self.window.lock() = Some(window);
    }

    pub fn window(&self) -> Option<WebviewWindow<CefRuntime>> {
        self.window.lock().clone()
    }

    pub fn server_shutdown(&self) -> Arc<tokio::sync::Notify> {
        self.server_shutdown.clone()
    }

    pub fn shutdown(&self) {
        self.processing.stop_all();
        self.state.cancel_all();
        self.server_shutdown.notify_waiters();
        if let Some(session) = self.ws.lock().take() {
            session.abort.abort();
        }
        self.window.lock().take();
    }

    pub fn spawn_download_events(&self) {
        let downloads = self.downloads.clone();
        drop(tauri::async_runtime::spawn(async move {
            let mut events = koharu_runtime::download::subscribe();
            loop {
                match events.recv().await {
                    Ok(event) => {
                        let download = match event {
                            koharu_runtime::download::Event::Started { id, name } => {
                                tracing::info!(
                                    target: "koharu_metrics",
                                    metric = "download_start",
                                    resource = "runtime",
                                );
                                Download {
                                    id,
                                    state: DownloadState::Running,
                                    name: Some(name),
                                    completed: 0,
                                    total: 0,
                                    error: None,
                                }
                            }
                            koharu_runtime::download::Event::Progress {
                                id,
                                name,
                                completed,
                                total,
                            } => {
                                tracing::info!(
                                    target: "koharu_metrics",
                                    metric = "download_progress",
                                    resource = "runtime",
                                    used_bytes = completed,
                                    total_bytes = total,
                                );
                                Download {
                                    id,
                                    state: DownloadState::Running,
                                    name: Some(name),
                                    completed,
                                    total,
                                    error: None,
                                }
                            }
                            koharu_runtime::download::Event::Finished { id } => {
                                tracing::info!(
                                    target: "koharu_metrics",
                                    metric = "download_result",
                                    resource = "runtime",
                                    outcome = "completed",
                                );
                                Download {
                                    id,
                                    state: DownloadState::Finished,
                                    name: None,
                                    completed: 0,
                                    total: 0,
                                    error: None,
                                }
                            }
                            koharu_runtime::download::Event::Failed { id, name, error } => {
                                tracing::info!(
                                    target: "koharu_metrics",
                                    metric = "download_result",
                                    resource = "runtime",
                                    outcome = "failed",
                                );
                                Download {
                                    id,
                                    state: DownloadState::Failed,
                                    name: Some(name),
                                    completed: 0,
                                    total: 0,
                                    error: Some(error),
                                }
                            }
                        };
                        downloads.channel.publish(download);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "download channel fell behind");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }));
    }

    #[cfg(test)]
    pub(crate) fn for_router_tests() -> Result<Self> {
        Ok(Self::assemble(
            CurrentProject {
                project: Arc::new(tokio::sync::Mutex::new(None)),
            },
            ProjectLibrary::with_root({
                tempfile::Builder::new()
                    .prefix("koharu-library-")
                    .tempdir()
                    .context("router test library")?
                    .keep()
            })?,
            Processing::default(),
            CanvasChannel::default(),
            JobChannel::default(),
            DownloadChannel::default(),
            ResourceChannel::default(),
            ProjectChannel::default(),
            Initialization::default(),
            koharu_desktop::Desktop::new()?,
            AgentState::empty(),
            Pipeline(Arc::new(OnceLock::new())),
        ))
    }

    #[cfg(test)]
    pub(crate) async fn open_memory_project(&self, name: &str) -> Result<()> {
        use koharu_scene::{At, PageDraft, Session};

        use crate::commands::project::Project;
        let mut session = Session::memory().await?;
        let mut setup = session.snapshot().edit();
        setup.add_page(PageDraft::new("page", 64.0, 64.0), At::End)?;
        session.commit(setup.finish()?).await?;
        *self.project.project.lock().await = Some(Project::new(session, name.to_owned()));
        Ok(())
    }

    #[tracing::instrument(
        target = "koharu_metrics",
        name = "app_started",
        skip_all,
        fields(phase = "initialization")
    )]
    pub async fn initialize(&self, cpu: bool) -> Result<()> {
        let device = prepare_runtime(cpu).await?;
        let pipeline = koharu_pipeline::Pipeline::load(device)?;
        let mut resources = pipeline.subscribe_resources();
        self.pipeline
            .0
            .set(pipeline)
            .map_err(|_| anyhow!("pipeline is already initialized"))?;

        let resource_channel = self.resources.clone();
        drop(tauri::async_runtime::spawn(async move {
            while resources.changed().await.is_ok() {
                let snapshot = resources.borrow_and_update().clone();
                resource_channel
                    .channel
                    .publish(ModelResources::from(snapshot));
            }
        }));

        let project = self
            .project
            .project
            .lock()
            .await
            .as_ref()
            .map(|project| (project.snapshot(), project.active_page()));
        if let Some((snapshot, page)) = project {
            self.desktop.show_page(&snapshot, page).await?;
        } else {
            self.desktop.clear().await;
        }
        self.initialization.ready();
        Ok(())
    }
}

/// Downloads and initializes process-wide native runtimes without starting HTTP or GUI.
pub async fn prepare_runtime(cpu: bool) -> Result<koharu_ml::Device> {
    koharu_ml::init()
        .await
        .context("failed to initialize the ML runtime")?;
    let device = koharu_ml::device(cpu);
    koharu_metrics::context(serde_json::json!({
        "compute_backend": device.backend.to_string().to_ascii_lowercase(),
        "device_type": format!("{:?}", device.device_type).to_ascii_lowercase(),
        "gpu_model": device.description.clone(),
        "vram_bytes": device.memory_total,
    }));
    Ok(device)
}

/// Configures the Windows packaged store from an install/resource directory.
///
/// Headless has no Tauri `Application`, so it passes `None` and uses the executable directory.
pub fn configure_packaged_store(resource_dir: Option<PathBuf>) -> Result<()> {
    #[cfg(all(target_os = "windows", not(debug_assertions)))]
    {
        let root = match resource_dir {
            Some(dir) => dir,
            None => std::env::current_exe()
                .context("failed to locate Koharu executable")?
                .parent()
                .context("failed to locate Koharu's installation directory")?
                .to_path_buf(),
        };
        koharu_runtime::Store::configure(root.join("store"))?;
    }
    #[cfg(not(all(target_os = "windows", not(debug_assertions))))]
    {
        let _ = resource_dir;
    }
    Ok(())
}

impl<S, T> FromRequest<S> for JsonOrMultipart<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let content_type = req
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if content_type.starts_with("multipart/form-data") {
            let multipart = Multipart::from_request(req, state)
                .await
                .map_err(|error| ApiError(error.to_string()))?;
            parse_multipart(multipart).await
        } else {
            let axum::Json(payload) = axum::Json::<T>::from_request(req, state)
                .await
                .map_err(|error| ApiError(error.to_string()))?;
            Ok(Self {
                payload,
                files: None,
            })
        }
    }
}

async fn parse_multipart<T: DeserializeOwned>(
    mut multipart: Multipart,
) -> Result<JsonOrMultipart<T>, ApiError> {
    let dir = tempfile::TempDir::new().map_err(|error| ApiError(error.to_string()))?;
    let mut extra = serde_json::Map::new();
    let mut uploaded = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| ApiError(error.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "files" {
            let raw_name = field.file_name().unwrap_or("upload").to_string();
            let safe = safe_filename(&raw_name);
            let data = field
                .bytes()
                .await
                .map_err(|error| ApiError(error.to_string()))?;
            let path = unique_path(dir.path(), &safe);
            std::fs::write(&path, &data).map_err(|error| ApiError(error.to_string()))?;
            uploaded.push(path);
        } else {
            let text = field
                .text()
                .await
                .map_err(|error| ApiError(error.to_string()))?;
            let value = serde_json::from_str(&text).unwrap_or(Value::String(text));
            extra.insert(name, value);
        }
    }

    if !uploaded.is_empty() {
        extra.insert(
            "paths".into(),
            serde_json::to_value(&uploaded).map_err(|error| ApiError(error.to_string()))?,
        );
    }

    let payload = serde_json::from_value(Value::Object(extra))
        .map_err(|error| ApiError(format!("invalid import fields: {error}")))?;
    Ok(JsonOrMultipart {
        payload,
        files: Some(dir),
    })
}

fn safe_filename(name: &str) -> String {
    Path::new(name)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .unwrap_or("upload")
        .to_owned()
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let mut path = dir.join(name);
    if !path.exists() {
        return path;
    }
    let stem = Path::new(name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("upload");
    let ext = Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");
    let mut index = 1;
    loop {
        path = if ext.is_empty() {
            dir.join(format!("{stem}-{index}"))
        } else {
            dir.join(format!("{stem}-{index}.{ext}"))
        };
        if !path.exists() {
            return path;
        }
        index += 1;
    }
}

pub(crate) fn sanitize_import_paths(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>, Error> {
    for path in &paths {
        if path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(anyhow!("import path must not contain '..'").into());
        }
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelopes_match_browser_channel_names() {
        assert_eq!(
            envelope_channel("subscribe", "on_canvas").unwrap(),
            "canvas"
        );
        assert_eq!(envelope_channel("subscribe", "on_job").unwrap(), "job");
        assert_eq!(
            envelope_channel("subscribe", "on_download").unwrap(),
            "download"
        );
        assert_eq!(
            envelope_channel("subscribe", "on_resources").unwrap(),
            "resources"
        );
        assert_eq!(
            envelope_channel("subscribe", "on_project").unwrap(),
            "project"
        );
        assert_eq!(
            envelope_channel("login_agent", "on_event").unwrap(),
            "agent_login"
        );
        assert_eq!(
            envelope_channel("run_agent", "on_event").unwrap(),
            "agent_event"
        );
        assert!(envelope_channel("login_agent", "on_canvas").is_err());
    }
}
