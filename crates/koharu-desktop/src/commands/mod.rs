//! The commands the React interface calls.

mod agent;
mod canvas;
#[cfg(debug_assertions)]
mod debug;
mod downloads;
mod export;
mod fonts;
mod gallery;
mod import;
mod keyboard;
mod metadata;
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

/// A file dialog parented to `window`, except for an X11 window inside a
/// Wayland session: `xdg-desktop-portal-gnome` / Nautilus crashes in
/// `gxdp_wayland_set_x11_parent` when an `x11:` parent is passed without
/// Mutter's service channel.
fn file_dialog(window: &tauri::Window<CefRuntime>) -> rfd::AsyncFileDialog {
    let dialog = rfd::AsyncFileDialog::new();
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        && matches!(
            wgpu::rwh::HasWindowHandle::window_handle(window).map(|handle| handle.as_raw()),
            Ok(wgpu::rwh::RawWindowHandle::Xlib(_) | wgpu::rwh::RawWindowHandle::Xcb(_))
        )
    {
        return dialog;
    }
    dialog.set_parent(window)
}

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
        processing::cancel_run,
        processing::unload,
        processing::get_pipeline_events,
        downloads::list_downloads,
        downloads::cancel_download,
        downloads::retry_download,
        downloads::dismiss_download,
        agent::run_agent,
        agent::stop_agent,
        agent::reset_agent,
        preferences::get_preferences,
        preferences::save_preferences,
        preferences::list_model_catalog,
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
        import::import_paths,
        export::export,
        keyboard::get_keyboard,
        metadata::get_comic_info,
        metadata::set_comic_info,
        canvas::set_viewport,
        canvas::set_tool,
        canvas::set_brush,
        canvas::set_colors,
        canvas::set_story,
        canvas::set_original,
        canvas::style_typing,
        canvas::annotate_typing,
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
        processing::cancel_run,
        processing::unload,
        processing::get_pipeline_events,
        downloads::list_downloads,
        downloads::cancel_download,
        downloads::retry_download,
        downloads::dismiss_download,
        agent::run_agent,
        agent::stop_agent,
        agent::reset_agent,
        preferences::get_preferences,
        preferences::save_preferences,
        preferences::list_model_catalog,
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
        import::import_paths,
        export::export,
        keyboard::get_keyboard,
        metadata::get_comic_info,
        metadata::set_comic_info,
        canvas::set_viewport,
        canvas::set_tool,
        canvas::set_brush,
        canvas::set_colors,
        canvas::set_story,
        canvas::set_original,
        canvas::style_typing,
        canvas::annotate_typing,
        canvas::fit,
        canvas::set_zoom,
        canvas::select,
        canvas::nudge,
        processing::fill,
        canvas::deselect,
    ](types)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_and_events_expose_queue_and_download_commands_without_stop_pipeline() {
        let mut types = Types::default();
        let fns = bindings(&mut types);
        let evs = crate::events::bindings(&mut types);

        let fn_names: Vec<&str> = fns.iter().map(|f| f.name().as_ref()).collect();
        assert!(!fn_names.contains(&"stop_pipeline"));
        for expected in [
            "run_pipeline",
            "cancel_run",
            "list_downloads",
            "cancel_download",
            "retry_download",
            "dismiss_download",
        ] {
            assert!(fn_names.contains(&expected), "missing command {expected}");
        }

        let run_fn = fns.iter().find(|f| f.name() == "run_pipeline").unwrap();
        let run_args: Vec<&str> = run_fn
            .args()
            .iter()
            .map(|(name, _)| name.as_ref())
            .collect();
        assert_eq!(run_args, vec!["target", "stages", "priority"]);

        let cancel_fn = fns.iter().find(|f| f.name() == "cancel_run").unwrap();
        let cancel_args: Vec<&str> = cancel_fn
            .args()
            .iter()
            .map(|(name, _)| name.as_ref())
            .collect();
        assert_eq!(cancel_args, vec!["run", "page", "stage"]);

        let ev_names: Vec<&str> = evs.iter().map(|(name, _)| *name).collect();
        assert!(ev_names.contains(&"pipeline"));
        assert!(ev_names.contains(&"download"));
    }
}
