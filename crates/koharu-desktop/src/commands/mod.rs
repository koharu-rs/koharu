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

pub fn handler() -> impl Fn(Invoke<CefRuntime>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        project::list_projects,
        project::create_project,
        project::open_project,
        project::delete_project,
        project::close_project,
        project::get_open_project,
        project::get_shown_page,
        project::show_page,
        scene::query,
        scene::edit,
        scene::undo,
        scene::redo,
        scene::go_to_step,
        pages::get_thumbnail,
        processing::process,
        processing::stop,
        processing::get_runs,
        agent::run_agent,
        agent::cancel_agent,
        preferences::get_preferences,
        preferences::save_preferences,
        preferences::get_models,
        preferences::get_vendors,
        preferences::set_api_key,
        fonts::get_fonts,
        fonts::get_font_preview,
        import::import,
        import::import_paths,
        export::export,
        canvas::set_canvas_viewport,
        canvas::set_tool,
        canvas::set_brush,
        canvas::set_colors,
        canvas::fit_canvas,
        canvas::set_zoom,
        canvas::select_layers,
        #[cfg(debug_assertions)]
        debug::capture_window,
        #[cfg(debug_assertions)]
        debug::debug_pointer,
    ]
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
        project::get_shown_page,
        project::show_page,
        scene::query,
        scene::edit,
        scene::undo,
        scene::redo,
        scene::go_to_step,
        pages::get_thumbnail,
        processing::process,
        processing::stop,
        processing::get_runs,
        agent::run_agent,
        agent::cancel_agent,
        preferences::get_preferences,
        preferences::save_preferences,
        preferences::get_models,
        preferences::get_vendors,
        preferences::set_api_key,
        fonts::get_fonts,
        fonts::get_font_preview,
        import::import,
        import::import_paths,
        export::export,
        canvas::set_canvas_viewport,
        canvas::set_tool,
        canvas::set_brush,
        canvas::set_colors,
        canvas::fit_canvas,
        canvas::set_zoom,
        canvas::select_layers,
    ](types)
}

/// Runs blocking work, a render or a file, off the async runtime's workers.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Result<T> {
    Ok(tokio::task::spawn_blocking(work).await??)
}
