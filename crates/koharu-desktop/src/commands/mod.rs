//! The commands the React interface calls.

mod agent;
mod canvas;
#[cfg(debug_assertions)]
mod debug;
mod export;
mod fonts;
mod gallery;
mod import;
mod pages;
mod preferences;
mod processing;
mod project;
mod scene;

use serde::Serialize;
use specta::{
    Type, Types,
    datatype::{DataType, Function},
};
use tauri::ipc::{Invoke, InvokeResponseBody, IpcResponse};
use tauri_runtime_cef::CefRuntime;

/// A command's failure, as its message with its causes.
#[derive(Debug, Serialize, Type)]
#[serde(transparent)]
pub struct Error(String);

impl<E: Into<anyhow::Error>> From<E> for Error {
    fn from(error: E) -> Self {
        Self(format!("{:#}", error.into()))
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Bytes a command answers as an `ArrayBuffer`, not a JSON array of numbers.
pub struct Bytes(pub Vec<u8>);

impl Type for Bytes {
    fn definition(_: &mut Types) -> DataType {
        DataType::Reference(specta_typescript::define("ArrayBuffer"))
    }
}

impl IpcResponse for Bytes {
    fn body(self) -> tauri::Result<InvokeResponseBody> {
        Ok(InvokeResponseBody::Raw(self.0))
    }
}

/// Every command, each run inside a span continuing the trace its
/// `sentry-trace` header carries: the interface's span of that call. Tauri
/// runs an async command's future inside a span of its own, made under
/// this one.
pub fn handler() -> impl Fn(Invoke<CefRuntime>) -> bool + Send + Sync + 'static {
    let commands: fn(Invoke<CefRuntime>) -> bool = tauri::generate_handler![
        project::list_projects,
        project::create,
        project::open,
        project::delete_project,
        project::close,
        project::get_open_project,
        pages::get_shown_page,
        pages::show,
        scene::query,
        scene::edit,
        scene::undo,
        scene::redo,
        scene::go_to,
        pages::get_page_thumbnail,
        processing::run_pipeline,
        processing::stop_pipeline,
        processing::unload,
        processing::get_pipeline_events,
        agent::run_agent,
        agent::stop_agent,
        agent::reset_agent,
        preferences::get_preferences,
        preferences::save_preferences,
        preferences::list_models,
        preferences::list_vendors,
        preferences::set_api_key,
        preferences::sign_in,
        preferences::stop_sign_in,
        preferences::sign_out,
        gallery::list_sites,
        gallery::search_site,
        gallery::browse_site,
        gallery::open_series,
        gallery::get_cover,
        gallery::sign_in_site,
        gallery::sign_out_site,
        gallery::get_account,
        gallery::read_chapter,
        fonts::list_fonts,
        fonts::get_font_preview,
        import::import,
        export::export,
        canvas::set_viewport,
        canvas::set_tool,
        canvas::set_brush,
        canvas::set_colors,
        canvas::set_story,
        canvas::style_typing,
        canvas::fit,
        canvas::set_zoom,
        canvas::select,
        canvas::nudge,
        processing::fill,
        canvas::deselect,
        #[cfg(debug_assertions)]
        debug::capture,
        #[cfg(debug_assertions)]
        debug::debug_pointer,
    ];
    move |invoke: Invoke<CefRuntime>| {
        let message = &invoke.message;
        let trace = message
            .headers()
            .get("sentry-trace")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let _span = tracing::info_span!(
            "command",
            sentry.name = message.command(),
            sentry.op = "command",
            sentry.trace = trace,
        )
        .entered();
        commands(invoke)
    }
}

/// Every command's signature, for the bindings; the same commands as
/// `handler`'s, but the debug build's own.
pub fn bindings(types: &mut Types) -> Vec<Function> {
    specta::function::collect_functions![
        project::list_projects,
        project::create,
        project::open,
        project::delete_project,
        project::close,
        project::get_open_project,
        pages::get_shown_page,
        pages::show,
        scene::query,
        scene::edit,
        scene::undo,
        scene::redo,
        scene::go_to,
        pages::get_page_thumbnail,
        processing::run_pipeline,
        processing::stop_pipeline,
        processing::unload,
        processing::get_pipeline_events,
        agent::run_agent,
        agent::stop_agent,
        agent::reset_agent,
        preferences::get_preferences,
        preferences::save_preferences,
        preferences::list_models,
        preferences::list_vendors,
        preferences::set_api_key,
        preferences::sign_in,
        preferences::stop_sign_in,
        preferences::sign_out,
        gallery::list_sites,
        gallery::search_site,
        gallery::browse_site,
        gallery::open_series,
        gallery::get_cover,
        gallery::sign_in_site,
        gallery::sign_out_site,
        gallery::get_account,
        gallery::read_chapter,
        fonts::list_fonts,
        fonts::get_font_preview,
        import::import,
        export::export,
        canvas::set_viewport,
        canvas::set_tool,
        canvas::set_brush,
        canvas::set_colors,
        canvas::set_story,
        canvas::style_typing,
        canvas::fit,
        canvas::set_zoom,
        canvas::select,
        canvas::nudge,
        processing::fill,
        canvas::deselect,
    ](types)
}
