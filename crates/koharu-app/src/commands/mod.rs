pub(crate) mod agent;
pub(crate) mod canvas;
pub(crate) mod editing;
pub(crate) mod fonts;
pub(crate) mod import;
pub(crate) mod lifecycle;
pub(crate) mod output;
pub(crate) mod preferences;
pub(crate) mod processing;
pub(crate) mod project;

use parking_lot::Mutex;
use serde::Serialize;
use specta::Type;

pub use crate::channel::Channel;

#[derive(Debug, Type)]
#[specta(transparent)]
pub(crate) struct Error(#[specta(type = String)] anyhow::Error);

impl<E> From<E> for Error
where
    E: Into<anyhow::Error>,
{
    fn from(error: E) -> Self {
        Self(error.into())
    }
}

impl Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("{:#}", self.0))
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:#}", self.0)
    }
}

pub(crate) trait ChannelExt<T> {
    fn publish(&self, value: T);
}

impl<T: Serialize> ChannelExt<T> for Mutex<Option<Channel<T>>> {
    fn publish(&self, value: T) {
        let mut channel = self.lock();
        if channel
            .as_ref()
            .is_some_and(|channel| channel.send(value).is_err())
        {
            channel.take();
        }
    }
}

pub fn protocol_functions(types: &mut specta::Types) -> Vec<specta::datatype::Function> {
    specta::function::collect_functions![
        agent::get_agent_status,
        agent::login_agent,
        agent::logout_agent,
        agent::save_agent_config,
        agent::run_agent,
        agent::cancel_agent,
        lifecycle::subscribe,
        lifecycle::get_project,
        lifecycle::get_pages,
        lifecycle::get_page,
        lifecycle::list_projects,
        lifecycle::create_project,
        lifecycle::open_project,
        lifecycle::delete_project,
        lifecycle::close_project,
        lifecycle::import,
        lifecycle::select_page,
        editing::rename_page,
        editing::delete_pages,
        editing::move_page,
        editing::set_source_text,
        editing::set_translation,
        editing::set_typography,
        editing::set_geometry,
        editing::set_visibility,
        editing::delete_layers,
        editing::move_layer,
        editing::undo,
        editing::redo,
        processing::process,
        processing::stop_job,
        output::export,
        output::get_thumbnail,
        fonts::get_fonts,
        fonts::get_font_preview,
        preferences::save_preferences,
        preferences::get_preferences,
        preferences::get_translation_models,
        canvas::get_canvas_manifest,
        canvas::get_canvas_resource,
        canvas::prepare_canvas_page,
        canvas::get_canvas_page_manifest,
        canvas::get_canvas_page_resource,
        canvas::add_point_text,
        canvas::add_text_box,
        canvas::commit_paint,
        canvas::commit_erase,
        canvas::commit_transform,
        canvas::commit_inpaint,
    ](types)
}

pub fn router() -> axum::Router<crate::host::Host> {
    koharu_macros::routes!(
        agent::get_agent_status,
        agent::login_agent,
        agent::logout_agent,
        agent::save_agent_config,
        agent::run_agent,
        agent::cancel_agent,
        lifecycle::subscribe,
        lifecycle::get_project,
        lifecycle::get_pages,
        lifecycle::get_page,
        lifecycle::list_projects,
        lifecycle::create_project,
        lifecycle::open_project,
        lifecycle::delete_project,
        lifecycle::close_project,
        lifecycle::import,
        lifecycle::select_page,
        editing::rename_page,
        editing::delete_pages,
        editing::move_page,
        editing::set_source_text,
        editing::set_translation,
        editing::set_typography,
        editing::set_geometry,
        editing::set_visibility,
        editing::delete_layers,
        editing::move_layer,
        editing::undo,
        editing::redo,
        processing::process,
        processing::stop_job,
        output::export,
        output::get_thumbnail,
        fonts::get_fonts,
        fonts::get_font_preview,
        preferences::save_preferences,
        preferences::get_preferences,
        preferences::get_translation_models,
        canvas::get_canvas_manifest,
        canvas::get_canvas_resource,
        canvas::prepare_canvas_page,
        canvas::get_canvas_page_manifest,
        canvas::get_canvas_page_resource,
        canvas::add_point_text,
        canvas::add_text_box,
        canvas::commit_paint,
        canvas::commit_erase,
        canvas::commit_transform,
        canvas::commit_inpaint,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::Host;
    use axum::{
        body::Body,
        http::{
            Request, StatusCode,
            header::{CONTENT_DISPOSITION, CONTENT_TYPE},
        },
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn test_router(host: Host) -> axum::Router {
        router().with_state(host)
    }

    async fn body_text(response: axum::http::Response<Body>) -> String {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[tokio::test]
    async fn get_project_accepts_empty_post_body() {
        let host = Host::for_router_tests().unwrap();
        let response = test_router(host)
            .oneshot(
                Request::post("/rpc/get_project")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_text(response).await, "null");
    }

    #[tokio::test]
    async fn login_agent_errors_without_subscribe_session() {
        let host = Host::for_router_tests().unwrap();
        let response = test_router(host)
            .oneshot(
                Request::post("/rpc/login_agent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = body_text(response).await;
        assert!(
            body.contains("no active subscribe session"),
            "expected session error before login work, got {body}"
        );
    }

    #[tokio::test]
    async fn import_accepts_repeated_multipart_files_through_router() {
        let host = Host::for_router_tests().unwrap();
        let boundary = "----KoharuBoundary";
        let mut body = Vec::new();
        body.extend_from_slice(
            format!(
                "--{boundary}\r\n\
                 Content-Disposition: form-data; name=\"source\"\r\n\r\n\
                 files\r\n\
                 --{boundary}\r\n\
                 Content-Disposition: form-data; name=\"files\"; filename=\"page.png\"\r\n\
                 Content-Type: image/png\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(TINY_PNG);
        body.extend_from_slice(
            format!(
                "\r\n--{boundary}\r\n\
                 Content-Disposition: form-data; name=\"files\"; filename=\"two.png\"\r\n\
                 Content-Type: image/png\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(TINY_PNG);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let response = test_router(host)
            .oneshot(
                Request::post("/rpc/import")
                    .header(
                        CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let text = body_text(response).await;
        assert!(
            text.contains("no project is open"),
            "multipart files must reach __import while TempDir is alive, got {text}"
        );
        assert!(
            !text.contains("invalid import fields") && !text.contains("Failed to deserialize"),
            "extractor must accept repeated files fields, got {text}"
        );
    }

    #[tokio::test]
    async fn commit_paint_accepts_camel_case_json_body() {
        let host = Host::for_router_tests().unwrap();
        let response = test_router(host)
            .oneshot(
                Request::post("/rpc/commit_paint")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"expectedRevision":0,"layer":null,"points":[],"brush":{"diameter":1.0,"color":[0,0,0,255]}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let text = body_text(response).await;
        assert!(
            !text.contains("Failed to deserialize")
                && !text.contains("missing field")
                && !text.contains("unknown field"),
            "camelCase commit_paint body must deserialize, got {status} {text}"
        );
        assert_ne!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "camelCase body must not be a serde rejection, got {text}"
        );
    }

    #[tokio::test]
    async fn export_without_destination_returns_typed_bytes() {
        let host = Host::for_router_tests().unwrap();
        host.open_memory_project("demo").await.unwrap();
        let response = test_router(host)
            .oneshot(
                Request::post("/rpc/export")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"format":"png","destination":null}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(CONTENT_TYPE).unwrap(),
            "application/zip"
        );
        assert_eq!(
            response.headers().get(CONTENT_DISPOSITION).unwrap(),
            "attachment; filename=\"demo.zip\""
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert!(
            bytes.starts_with(b"PK"),
            "expected zip bytes, got {bytes:?}"
        );
    }
}
