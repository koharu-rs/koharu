//! The commands the React interface calls.

mod agent;
mod canvas;
#[cfg(debug_assertions)]
mod debug;
mod export;
mod fonts;
mod history;
mod import;
mod layers;
mod pages;
mod preferences;
mod processing;
mod project;

use std::sync::Arc;

use koharu_scene::{Edit, Op};
use serde::Serialize;
use specta::{
    Type, Types,
    datatype::{DataType, Function},
};
use tauri::{
    State,
    ipc::{Invoke, InvokeResponseBody, IpcResponse},
};
use tauri_runtime_cef::CefRuntime;

use crate::app::App;

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
        project::subscribe,
        project::list_projects,
        project::create_project,
        project::open_project,
        project::delete_project,
        project::close_project,
        project::get_project,
        project::get_page,
        project::select_page,
        pages::delete_pages,
        pages::move_page,
        pages::get_thumbnail,
        layers::set_visibility,
        layers::delete_layers,
        layers::move_layer,
        layers::set_locked,
        layers::set_text,
        layers::set_source_text,
        layers::set_style,
        layers::update_layer,
        history::undo,
        history::redo,
        history::go_to_step,
        processing::process,
        processing::stop,
        agent::get_agent_status,
        agent::save_agent_config,
        agent::run_agent,
        agent::cancel_agent,
        agent::set_mcp,
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
        project::subscribe,
        project::list_projects,
        project::create_project,
        project::open_project,
        project::delete_project,
        project::close_project,
        project::get_project,
        project::get_page,
        project::select_page,
        pages::delete_pages,
        pages::move_page,
        pages::get_thumbnail,
        layers::set_visibility,
        layers::delete_layers,
        layers::move_layer,
        layers::set_locked,
        layers::set_text,
        layers::set_source_text,
        layers::set_style,
        layers::update_layer,
        history::undo,
        history::redo,
        history::go_to_step,
        processing::process,
        processing::stop,
        agent::get_agent_status,
        agent::save_agent_config,
        agent::run_agent,
        agent::cancel_agent,
        agent::set_mcp,
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

/// Runs `ops` on the open project as one undo step, under `label`.
async fn edit(app: &State<'_, Arc<App>>, label: &str, ops: Vec<Op>) -> Result<()> {
    let app = Arc::clone(app);
    let edit = Edit {
        label: label.to_owned(),
        ops,
    };
    blocking(move || app.edit(edit)).await
}
