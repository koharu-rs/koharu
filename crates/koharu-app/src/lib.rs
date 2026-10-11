//! Koharu's Tauri-managed application state, commands, and lifecycle.

mod app;
mod commands;
#[cfg(target_os = "linux")]
mod linux_focus;

pub use app::run;
pub use commands::bindings;
