use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use axum::{
    body::Body,
    http::{
        Request, StatusCode,
        header::{CONTENT_TYPE, HeaderValue},
        uri::Uri,
    },
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use tauri_runtime_cef::CefRuntime;

#[derive(Clone)]
pub(crate) struct FrontendAssets {
    inner: Arc<FrontendInner>,
}

enum FrontendInner {
    Dir(PathBuf),
    Memory(HashMap<String, Bytes>),
}

impl FrontendAssets {
    pub(crate) fn from_dir(root: PathBuf) -> Self {
        Self {
            inner: Arc::new(FrontendInner::Dir(root)),
        }
    }

    pub(crate) fn from_memory(files: HashMap<String, Vec<u8>>) -> Self {
        Self {
            inner: Arc::new(FrontendInner::Memory(
                files
                    .into_iter()
                    .map(|(key, bytes)| (normalize_asset_key(&key), Bytes::from(bytes)))
                    .collect(),
            )),
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn debug_export_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_WORKSPACE_DIR")).join("packages/koharu/out")
    }

    #[cfg_attr(debug_assertions, allow(dead_code))]
    pub(crate) fn from_tauri_assets<R: tauri::Runtime>(assets: &dyn tauri::Assets<R>) -> Self {
        let mut files = HashMap::new();
        for (key, _) in assets.iter() {
            let asset_key = tauri::utils::assets::AssetKey::from(key.as_ref());
            if let Some(bytes) = assets.get(&asset_key) {
                files.insert(normalize_asset_key(&key), bytes.into_owned());
            }
        }
        Self::from_memory(files)
    }

    pub(crate) fn into_router(self) -> axum::Router {
        axum::Router::new()
            .fallback(serve_frontend)
            .with_state(self)
    }
}

async fn serve_frontend(
    axum::extract::State(assets): axum::extract::State<FrontendAssets>,
    request: Request<Body>,
) -> Response {
    serve_path(&assets, request.uri())
}

pub(crate) fn serve_path(assets: &FrontendAssets, uri: &Uri) -> Response {
    match classify_path(uri.path()) {
        Some(ClassifiedPath::Asset(relative)) => match assets.lookup(&relative) {
            Some((bytes, mime)) => {
                let mut response = Response::new(Body::from(bytes));
                if let Ok(value) = HeaderValue::from_str(mime) {
                    response.headers_mut().insert(CONTENT_TYPE, value);
                }
                response
            }
            None => StatusCode::NOT_FOUND.into_response(),
        },
        Some(ClassifiedPath::Rpc) | None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClassifiedPath {
    Rpc,
    Asset(String),
}

fn classify_path(path: &str) -> Option<ClassifiedPath> {
    let decoded = percent_decode(path)?;
    let mut parts = Vec::new();
    for segment in decoded.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            return None;
        }
        parts.push(segment);
    }
    if parts.first().copied() == Some("rpc") {
        Some(ClassifiedPath::Rpc)
    } else {
        Some(ClassifiedPath::Asset(parts.join("/")))
    }
}

impl FrontendAssets {
    fn lookup(&self, relative: &str) -> Option<(Bytes, &'static str)> {
        for candidate in file_candidates(relative) {
            if let Some(bytes) = self.get(&candidate) {
                return Some((bytes, mime_type(&candidate)));
            }
        }
        should_fallback(relative)
            .then(|| self.get("index.html"))
            .flatten()
            .map(|bytes| (bytes, mime_type("index.html")))
    }

    fn get(&self, candidate: &str) -> Option<Bytes> {
        match self.inner.as_ref() {
            FrontendInner::Dir(root) => {
                let path = root.join(candidate);
                path.is_file()
                    .then(|| std::fs::read(path).ok())
                    .flatten()
                    .map(Bytes::from)
            }
            FrontendInner::Memory(files) => files.get(&normalize_asset_key(candidate)).cloned(),
        }
    }
}

pub(crate) fn load_frontend(context: &tauri::Context<CefRuntime>) -> Result<FrontendAssets> {
    #[cfg(debug_assertions)]
    {
        let _ = context;
        let dir = FrontendAssets::debug_export_dir();
        anyhow::ensure!(
            dir.join("index.html").is_file(),
            "frontend export is missing at {}",
            dir.display()
        );
        Ok(FrontendAssets::from_dir(dir))
    }
    #[cfg(not(debug_assertions))]
    {
        Ok(FrontendAssets::from_tauri_assets(context.assets()))
    }
}

fn file_candidates(relative: &str) -> Vec<String> {
    if relative.is_empty() {
        return vec!["index.html".into()];
    }
    let mut candidates = vec![relative.to_owned()];
    if !relative.contains('.') {
        candidates.push(format!("{relative}.html"));
        candidates.push(format!("{relative}/index.html"));
    } else if relative.ends_with('/') {
        candidates.push(format!("{relative}index.html"));
    }
    candidates
}

fn should_fallback(relative: &str) -> bool {
    if relative.is_empty() {
        return true;
    }
    Path::new(relative)
        .file_name()
        .and_then(|name| name.to_str())
        .is_none_or(|name| !name.contains('.'))
}

fn normalize_asset_key(key: &str) -> String {
    let key = key.replace('\\', "/");
    if key.starts_with('/') {
        key
    } else {
        format!("/{key}")
    }
}

fn percent_decode(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            out.push(decode_hex_pair(bytes[index + 1], bytes[index + 2])?);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(out).ok()
}

fn decode_hex_pair(high: u8, low: u8) -> Option<u8> {
    Some(decode_hex(high)? << 4 | decode_hex(low)?)
}

fn decode_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn mime_type(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript",
        Some("css") => "text/css",
        Some("json") | Some("map") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("wasm") => "application/wasm",
        Some("txt") => "text/plain; charset=utf-8",
        Some("xml") => "application/xml",
        Some("webmanifest") => "application/manifest+json",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode, header::CONTENT_TYPE},
        routing::post,
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn fixture_dir() -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .prefix("koharu-assets-")
            .tempdir()
            .unwrap();
        std::fs::write(dir.path().join("index.html"), b"<html>home</html>").unwrap();
        std::fs::write(dir.path().join("app.js"), b"console.log(1)").unwrap();
        std::fs::create_dir_all(dir.path().join("nested")).unwrap();
        std::fs::write(dir.path().join("nested/style.css"), b"body{}").unwrap();
        dir
    }

    fn app(root: &Path) -> Router {
        Router::new()
            .route("/rpc/get_project", post(|| async { "null" }))
            .fallback_service(FrontendAssets::from_dir(root.to_path_buf()).into_router())
    }

    async fn body(response: Response) -> Vec<u8> {
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec()
    }

    async fn assert_not_found_html(response: Response, path: &str) {
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        let bytes = body(response).await;
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("<html>"),
            "{path} must not fall back to SPA HTML, got {text}"
        );
    }

    #[tokio::test]
    async fn root_serves_index_html() {
        let dir = fixture_dir();
        let response = app(dir.path())
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
        assert_eq!(body(response).await, b"<html>home</html>");
    }

    #[tokio::test]
    async fn static_files_use_correct_mime() {
        let dir = fixture_dir();
        let js = app(dir.path())
            .oneshot(Request::get("/app.js").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(js.status(), StatusCode::OK);
        assert_eq!(js.headers().get(CONTENT_TYPE).unwrap(), "text/javascript");
        assert_eq!(body(js).await, b"console.log(1)");

        let css = app(dir.path())
            .oneshot(
                Request::get("/nested/style.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(css.status(), StatusCode::OK);
        assert_eq!(css.headers().get(CONTENT_TYPE).unwrap(), "text/css");
    }

    #[tokio::test]
    async fn unknown_browser_path_falls_back_to_index() {
        let dir = fixture_dir();
        let response = app(dir.path())
            .oneshot(Request::get("/project/demo").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
        assert_eq!(body(response).await, b"<html>home</html>");
    }

    #[tokio::test]
    async fn rpc_routes_take_precedence_and_unknown_rpc_is_not_html() {
        let dir = fixture_dir();
        let known = app(dir.path())
            .oneshot(
                Request::post("/rpc/get_project")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(known.status(), StatusCode::OK);
        assert_eq!(body(known).await, b"null");

        let unknown = app(dir.path())
            .oneshot(
                Request::get("/rpc/does-not-exist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_not_found_html(unknown, "unknown /rpc").await;

        for path in [
            "/rpc%2fdoes-not-exist",
            "/rpc%2Fdoes-not-exist",
            "//rpc/does-not-exist",
            "///rpc/does-not-exist",
            "/%2frpc/does-not-exist",
        ] {
            let response = app(dir.path())
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_not_found_html(response, path).await;
        }
    }

    #[test]
    fn classify_path_decodes_once_and_treats_rpc_prefix() {
        assert_eq!(classify_path("/rpc"), Some(ClassifiedPath::Rpc));
        assert_eq!(
            classify_path("/rpc/does-not-exist"),
            Some(ClassifiedPath::Rpc)
        );
        assert_eq!(
            classify_path("/rpc%2fdoes-not-exist"),
            Some(ClassifiedPath::Rpc)
        );
        assert_eq!(
            classify_path("//rpc/does-not-exist"),
            Some(ClassifiedPath::Rpc)
        );
        assert_eq!(
            classify_path("/rpc%252fdoes-not-exist"),
            Some(ClassifiedPath::Asset("rpc%2fdoes-not-exist".into()))
        );
        assert_eq!(
            classify_path("/project/demo"),
            Some(ClassifiedPath::Asset("project/demo".into()))
        );
        assert!(classify_path("/../index.html").is_none());
        assert!(classify_path("/%2e%2e/index.html").is_none());
        for path in ["/foo%", "/foo%2", "/foo%zz", "/project%GG"] {
            assert!(classify_path(path).is_none(), "{path}");
        }
    }

    #[tokio::test]
    async fn invalid_paths_are_rejected() {
        let dir = fixture_dir();
        let response = app(dir.path())
            .oneshot(Request::get("/../index.html").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let bytes = body(response).await;
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("<html>home</html>"),
            "parent-dir paths must not escape the asset root, got {text}"
        );

        let escaped = app(dir.path())
            .oneshot(
                Request::get("/%2e%2e/index.html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(escaped.status(), StatusCode::NOT_FOUND);
        let bytes = body(escaped).await;
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("<html>home</html>"),
            "percent-encoded traversal must not escape the asset root, got {text}"
        );

        let malformed = app(dir.path())
            .oneshot(Request::get("/foo%zz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_not_found_html(malformed, "malformed percent encoding").await;
    }

    #[tokio::test]
    async fn memory_assets_serve_index_and_spa_fallback() {
        let mut files = HashMap::new();
        files.insert("/index.html".into(), b"<html>embedded</html>".to_vec());
        files.insert("/app.js".into(), b"console.log(2)".to_vec());
        let app = Router::new().fallback_service(FrontendAssets::from_memory(files).into_router());

        let root = app
            .clone()
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(root.status(), StatusCode::OK);
        assert_eq!(
            root.headers().get(CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
        assert_eq!(body(root).await, b"<html>embedded</html>");

        let spa = app
            .clone()
            .oneshot(Request::get("/project/demo").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(spa.status(), StatusCode::OK);
        assert_eq!(body(spa).await, b"<html>embedded</html>");

        let js = app
            .oneshot(Request::get("/app.js").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(js.status(), StatusCode::OK);
        assert_eq!(js.headers().get(CONTENT_TYPE).unwrap(), "text/javascript");
    }

    #[tokio::test]
    async fn missing_static_file_is_not_html() {
        let dir = fixture_dir();
        let response = app(dir.path())
            .oneshot(Request::get("/missing.js").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let bytes = body(response).await;
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("<html>"));
    }
}
