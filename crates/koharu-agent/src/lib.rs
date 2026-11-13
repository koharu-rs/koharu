//! Koharu's agent and its tools on an open project.

// Proving a tool's future `Send` walks the renderer's wgpu types deeper than
// the default 128.
#![recursion_limit = "256"]

mod agent;
mod mcp;
mod tools;

pub use agent::{Agent, AgentConfig, Event};
pub use mcp::McpServer;
pub use tools::Tools;
