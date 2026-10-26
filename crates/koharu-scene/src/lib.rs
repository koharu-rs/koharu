//! Koharu's scene graph on Loro. The schema and its evidence are in the
//! crate README.
//!
//! A [`Session`] alone owns an open project's document. Reads return owned
//! values; every change is an [`Edit`], checked in full before anything is
//! written and committed as one undo step. The UI, the pipeline, MCP clients,
//! and the in-app agent all use the same [`Op`] and [`Query`] types, and
//! [`tools()`] generates the agent and MCP tools from them.

mod color;
mod doc;
mod error;
mod model;
mod op;
mod overrides;
mod query;
mod session;
mod state;
mod tiles;
mod tools;

pub use error::{Error, Kind, Result};
pub use loro::TextDelta;
pub use model::*;
pub use op::{Author, Edit, Op};
pub use query::{Answer, Query};
pub use session::Session;
pub use tools::{ToolCall, ToolSpec, compact, tool_call, tools};
