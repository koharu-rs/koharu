//! Koharu's Tauri-managed application state, commands, and lifecycle.

mod app;
mod commands;
mod history;

pub use app::run;
pub use commands::bindings;
