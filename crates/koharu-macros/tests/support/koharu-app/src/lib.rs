use std::marker::PhantomData;

use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct Pipeline;

#[derive(Clone)]
pub struct CefRuntime;

#[derive(Clone)]
pub struct WebviewWindow<R = CefRuntime>(PhantomData<R>);

#[derive(Clone)]
pub struct CurrentProject;

#[derive(Clone)]
pub struct ProjectLibrary;

#[derive(Clone)]
pub struct Processing;

#[derive(Clone)]
pub struct Desktop;

#[derive(Clone)]
pub struct AgentState;

#[derive(Clone)]
pub struct Initialization;

#[derive(Clone)]
pub struct CanvasChannel;

#[derive(Clone)]
pub struct JobChannel;

#[derive(Clone)]
pub struct DownloadChannel;

#[derive(Clone)]
pub struct ResourceChannel;

#[derive(Clone)]
pub struct ProjectChannel;

pub struct Channel<T>(PhantomData<T>);

impl<T> Channel<T> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

#[derive(Deserialize)]
pub struct Scope;

#[derive(Serialize)]
pub struct JobId;

pub struct CanvasState;

pub struct LoginEvent;

pub struct StartupState;

pub struct ExportBytes;

pub struct Error;

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
    }
}

pub mod host {
    use axum::extract::{FromRequest, Request, ws::WebSocket};
    use axum::response::Response;
    use serde::de::DeserializeOwned;

    use super::{
        AgentState, CanvasChannel, Channel, CurrentProject, Desktop, DownloadChannel, Error,
        ExportBytes, Initialization, JobChannel, Pipeline, Processing, ProjectChannel,
        ProjectLibrary, ResourceChannel,
    };

    #[derive(Clone)]
    pub struct Host {
        pub pipeline: Pipeline,
        pub project: CurrentProject,
        pub library: ProjectLibrary,
        pub processing: Processing,
        pub desktop: Desktop,
        pub state: AgentState,
        pub initialization: Initialization,
        pub canvas: CanvasChannel,
        pub jobs: JobChannel,
        pub downloads: DownloadChannel,
        pub resources: ResourceChannel,
        pub project_channel: ProjectChannel,
    }

    pub type ApiResult<T> = Result<T, Error>;

    pub struct JsonOrMultipart<T> {
        pub payload: T,
        pub files: Option<()>,
    }

    pub struct HttpFile;

    impl From<ExportBytes> for HttpFile {
        fn from(_: ExportBytes) -> Self {
            HttpFile
        }
    }

    impl<S, T> FromRequest<S> for JsonOrMultipart<T>
    where
        T: DeserializeOwned,
        S: Send + Sync,
    {
        type Rejection = Error;

        async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
            let axum::Json(payload) = axum::Json::<T>::from_request(req, state)
                .await
                .map_err(|_| Error)?;
            Ok(Self {
                payload,
                files: None,
            })
        }
    }

    pub struct WsSink;

    impl WsSink {
        pub fn channel<T>(&self, _name: &str) -> Channel<T> {
            Channel::new()
        }

        pub fn complete<T, E>(self, _result: Result<T, E>) {}
    }

    pub fn ws_sink(_host: &Host, socket: WebSocket) -> WsSink {
        let _ = socket;
        WsSink
    }

    pub fn ws_channel<T>(
        _host: &Host,
        _command_name: &str,
        _param_name: &str,
    ) -> Result<Channel<T>, Error> {
        Ok(Channel::new())
    }

    pub fn binary_response(_value: impl Into<HttpFile>) -> Response {
        Response::new(axum::body::Body::empty())
    }

    impl Host {
        pub fn window(&self) -> Option<super::WebviewWindow> {
            None
        }
    }
}
