//! The user's keyboard layout, as the interface's shortcuts need it.

use std::collections::HashMap;

use tauri::{AppHandle, Manager as _};
use tauri_runtime_cef::CefRuntime;
use tokio::sync::oneshot;

use super::Result;
use crate::screen::{self, Place};

/// Where the user's layout puts each US punctuation key, by
/// `KeyboardEvent.code`: the key a shortcut on the US key takes instead, and
/// what it types. Read on the main thread, whose layout its window's keys
/// arrive in.
#[tauri::command]
#[specta::specta]
pub async fn get_keyboard(handle: AppHandle<CefRuntime>) -> Result<HashMap<String, Place>> {
    let (send, places) = oneshot::channel();
    handle.run_on_main_thread(move || {
        let _ = send.send(screen::places());
    })?;
    Ok(places.await?)
}
