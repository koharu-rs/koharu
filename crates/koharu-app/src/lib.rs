//! Koharu's Tauri-managed application state, commands, and lifecycle.

extern crate self as koharu_app;

mod app;
mod channel;
mod commands;
pub mod host;

pub use app::{http_router, run};
pub use commands::{protocol_functions, router};
pub use host::{Host, configure_packaged_store, prepare_runtime};
