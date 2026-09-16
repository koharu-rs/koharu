//! Koharu's Tauri-managed application state, commands, and lifecycle.

mod app;
mod commands;

pub use app::run;
pub use commands::bindings;
