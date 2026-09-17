//! Proc macros that expand one command function into a shared body, an Axum
//! HTTP/WebSocket handler, and route/protocol metadata.
//!
//! Annotate a single `async fn` with [`macro@command`]. The function body is
//! kept once in a private `__name` function. The macro emits:
//!
//! - `#[specta::specta] pub async fn name(...)` with payload and Channel
//!   arguments for protocol generation
//! - `pub async fn name_http(...)` for axum
//! - a hidden `__name_route` module with `PATH` (`/rpc/{name}`) and `router()`
//!
//! [`routes`] builds an `axum::Router` by merging those `router()` functions.
//!
//! HTTP handlers extract `axum::extract::State<::koharu_app::host::Host>`.
//! Known state types map onto Host fields (`host.project` for `CurrentProject`,
//! `host.pipeline` for `Pipeline`). A `Host` parameter is cloned from axum
//! `State`. Unknown state types are a compile error. Window parameters come
//! from `host.window()`.
//!
//! # Parameter classes
//!
//! Classification is by type name:
//!
//! - Known app state (`Pipeline`, `Desktop`, `CurrentProject`, `Processing`,
//!   `AgentState`, `ProjectLibrary`, `Initialization`, `Host`, `*Channel`
//!   holders): HTTP clones `Host` or the matching Host field
//! - `WebviewWindow<_>`: HTTP `host.window()`; the shared body takes
//!   `Option<WebviewWindow<_>>`
//! - `Channel<_>`: GET WebSocket via `ws_sink`; POST named sink via
//!   `ws_channel`; never a JSON payload field
//! - Everything else: specta command argument and `{Name}Payload` JSON field.
//!   Generated payload structs deserialize the same camelCase names as the
//!   Specta protocol (`family_name` → `familyName`).
//!
//! Default HTTP method is `POST`. `subscribe` is `GET` (WebSocket upgrade).
//!
//! GET WebSocket handlers pass the upgraded socket to `ws_sink(&host, socket)`,
//! which returns a session handle. Channel parameters on GET are
//! `__ws.channel("param_name")`.
//!
//! POST handlers do not upgrade a WebSocket. They resolve a Channel-compatible
//! sink from the existing subscribe session with
//! `ws_channel(&host, "command_name", "param_name")?`. Missing sessions return
//! an invocation error before the command body runs.

mod command;

use proc_macro::TokenStream;

/// Expand one `async fn` into a Specta protocol function, an axum handler, and
/// a hidden `__name_route` module with `PATH` and `router()`.
///
/// ```ignore
/// #[command]
/// async fn process(
///     pipeline: Pipeline,
///     processing: Processing,
///     project: CurrentProject,
///     scope: Scope,
///     operation: Operation,
/// ) -> Result<JobId, Error> {
///     /* body once */
/// }
/// ```
#[proc_macro_attribute]
pub fn command(attr: TokenStream, item: TokenStream) -> TokenStream {
    command::expand(attr.into(), item.into())
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}

/// Build an `axum::Router` from `#[command]` functions.
///
/// ```ignore
/// routes!(process, import, export, subscribe);
/// ```
#[proc_macro]
pub fn routes(input: TokenStream) -> TokenStream {
    command::expand_routes(input.into())
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}
