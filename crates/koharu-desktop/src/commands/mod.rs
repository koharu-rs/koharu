//! The commands the React interface calls.

mod agent;
mod canvas;
#[cfg(debug_assertions)]
mod debug;
mod export;
mod fonts;
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
        project::create_project,
        project::open_project,
        project::delete_project,
        project::close_project,
        project::get_open_project,
        pages::get_shown_page,
        pages::show_page,
        scene::query_scene,
        scene::edit_scene,
        scene::undo_edit,
        scene::redo_edit,
        scene::go_to_step,
        pages::get_page_thumbnail,
        processing::run_pipeline,
        processing::stop_pipeline,
        processing::get_pipeline_events,
        agent::run_agent,
        agent::stop_agent,
        preferences::get_preferences,
        preferences::save_preferences,
        preferences::list_models,
        preferences::list_vendors,
        preferences::set_api_key,
        fonts::list_fonts,
        fonts::get_font_preview,
        import::import_pages,
        export::export_pages,
        canvas::set_canvas_viewport,
        canvas::set_canvas_tool,
        canvas::set_canvas_brush,
        canvas::set_canvas_colors,
        canvas::fit_canvas,
        canvas::set_canvas_zoom,
        canvas::select_layers,
        #[cfg(debug_assertions)]
        debug::capture_window,
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
        project::create_project,
        project::open_project,
        project::delete_project,
        project::close_project,
        project::get_open_project,
        pages::get_shown_page,
        pages::show_page,
        scene::query_scene,
        scene::edit_scene,
        scene::undo_edit,
        scene::redo_edit,
        scene::go_to_step,
        pages::get_page_thumbnail,
        processing::run_pipeline,
        processing::stop_pipeline,
        processing::get_pipeline_events,
        agent::run_agent,
        agent::stop_agent,
        preferences::get_preferences,
        preferences::save_preferences,
        preferences::list_models,
        preferences::list_vendors,
        preferences::set_api_key,
        fonts::list_fonts,
        fonts::get_font_preview,
        import::import_pages,
        export::export_pages,
        canvas::set_canvas_viewport,
        canvas::set_canvas_tool,
        canvas::set_canvas_brush,
        canvas::set_canvas_colors,
        canvas::fit_canvas,
        canvas::set_canvas_zoom,
        canvas::select_layers,
    ](types)
}
