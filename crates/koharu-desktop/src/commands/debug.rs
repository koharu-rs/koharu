//! Commands for checking the app from outside, in debug builds only.

use std::{io::Cursor, sync::Arc};

use image::ImageFormat;
use tauri::{AppHandle, Manager as _, State, ipc::Response};
use tauri_runtime_cef::CefRuntime;
use tokio::sync::oneshot;

use crate::{
    app::App,
    screen::{self, Pointer},
};

/// The window as it's composited, as PNG bytes: the canvas and CEF's paint
/// together, which a CDP screenshot of the page alone can't show.
#[tauri::command]
pub async fn capture_window(app: State<'_, Arc<App>>) -> Result<Response, String> {
    let screen = &app.screen;
    let (sender, receiver) = oneshot::channel();
    *screen.capture.lock() = Some(sender);
    screen.request_redraw();
    let image = receiver
        .await
        .map_err(|_| "the window closed before it drew".to_owned())?;
    let mut png = Cursor::new(Vec::new());
    image
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    Ok(Response::new(png.into_inner()))
}

/// Routes `pointer` as the main window would have, on the main thread where
/// its events arrive, so a check can click, drag, and zoom on the native
/// canvas.
#[tauri::command]
pub fn debug_pointer(
    handle: AppHandle<CefRuntime>,
    app: State<'_, Arc<App>>,
    pointer: Pointer,
    ctrl: bool,
) -> Result<(), String> {
    let app = Arc::clone(&app);
    handle
        .run_on_main_thread(move || screen::feed(&app, pointer, ctrl))
        .map_err(|error| error.to_string())
}
