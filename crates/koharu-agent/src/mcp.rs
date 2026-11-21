//! The agent's tools for outside agents, over MCP's Streamable HTTP on this
//! machine only: rig's tool set, listed and called as MCP tools.

use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
};

use parking_lot::RwLock;
use rig_agent::tool::ToolSet;
use rig_core::{
    completion::{
        ToolDefinition,
        message::{DocumentSourceKind, MimeType as _, ToolResultContent},
    },
    tool::{ToolContext, ToolOutput},
};
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        JsonObject, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
        Tool, ToolAnnotations,
    },
    service::RequestContext,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::{Tools, tools::refusal};

/// The MCP server, serving `/mcp` until dropped.
pub struct McpServer {
    address: SocketAddr,
    project: Arc<RwLock<Option<Tools>>>,
    cancel: CancellationToken,
}

impl McpServer {
    /// Serves on 127.0.0.1 at `port`, or a free port for 0, with no project
    /// open.
    pub async fn start(port: u16) -> anyhow::Result<Self> {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
        let address = listener.local_addr()?;
        let handler = Handler::new();
        let project = Arc::clone(&handler.project);
        let cancel = CancellationToken::new();
        // A browser sends an `Origin` and an MCP client doesn't, so refusing
        // every one keeps web pages out, as the MCP spec asks of a local
        // server; rmcp's `Host` check already keeps to loopback.
        let config = StreamableHttpServerConfig::default()
            .enforce_origin_validation()
            .with_cancellation_token(cancel.child_token());
        let service = StreamableHttpService::new(
            move || Ok(handler.clone()),
            Arc::new(LocalSessionManager::default()),
            config,
        );
        let router = axum::Router::new().nest_service("/mcp", service);
        let stop = cancel.clone();
        tokio::spawn(async move {
            let served = axum::serve(listener, router)
                .with_graceful_shutdown(async move { stop.cancelled().await })
                .await;
            if let Err(error) = served {
                tracing::warn!("the MCP server stopped: {error}");
            }
        });
        Ok(Self {
            address,
            project,
            cancel,
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// The project calls go to from now on, or none.
    pub fn open(&self, tools: Option<Tools>) {
        *self.project.write() = tools;
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Answers one MCP session from rig's tool set, on whichever project is
/// open.
#[derive(Clone)]
struct Handler {
    project: Arc<RwLock<Option<Tools>>>,
    tools: ToolSet,
}

impl Handler {
    fn new() -> Self {
        Self {
            project: Arc::default(),
            tools: Tools::toolset(),
        }
    }

    /// Runs the tool `name` on the open project, its arguments held to its
    /// schema first, as an op the schema leaves out would still parse.
    async fn answer(&self, name: &str, arguments: Value) -> CallToolResult {
        let definitions = self.tools.tool_definitions();
        let tool = definitions.iter().find(|tool| tool.name.as_str() == name);
        if let Some(refusal) = tool.and_then(|tool| refusal(&tool.parameters, &arguments)) {
            return CallToolResult::error(vec![ContentBlock::text(refusal)]);
        }
        let mut context = self
            .project
            .read()
            .as_ref()
            .map_or_else(ToolContext::default, Tools::tool_context);
        let result = self
            .tools
            .execute(name, arguments.to_string(), &mut context)
            .await;
        match result.into_result() {
            Ok(output) => CallToolResult::success(contents(output)),
            Err(error) => CallToolResult::error(contents(error.model_output().clone())),
        }
    }
}

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("koharu", env!("CARGO_PKG_VERSION")))
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = self.tools.tool_definitions();
        Ok(ListToolsResult::with_all_items(
            tools.iter().map(|tool| listed(tool)).collect(),
        ))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        Ok(self.answer(&request.name, arguments).await.into())
    }
}

/// `tool` as MCP lists it; rig's annotations are MCP's.
fn listed(tool: &ToolDefinition) -> Tool {
    let schema = match &tool.parameters {
        Value::Object(schema) => schema.clone(),
        _ => JsonObject::new(),
    };
    let hints = &tool.annotations;
    Tool::new(
        tool.name.as_str().to_owned(),
        tool.description.clone(),
        schema,
    )
    .annotate(ToolAnnotations::from_raw(
        hints.title.clone(),
        hints.read_only_hint,
        hints.destructive_hint,
        hints.idempotent_hint,
        hints.open_world_hint,
    ))
}

/// A tool's output as MCP content: JSON goes as its text.
fn contents(output: ToolOutput) -> Vec<ContentBlock> {
    let content = |content| match content {
        ToolResultContent::Text(text) => ContentBlock::text(text.text),
        ToolResultContent::Json { value } => ContentBlock::text(value.to_string()),
        ToolResultContent::Image(image) => match image.data {
            DocumentSourceKind::Base64(data) => ContentBlock::image(
                data,
                image
                    .media_type
                    .map_or("image/jpeg", |media| media.to_mime_type()),
            ),
            other => ContentBlock::text(format!("{other:?}")),
        },
    };
    output.into_content().into_iter().map(content).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn a_request_with_an_origin_is_refused() {
        let server = McpServer::start(0).await.expect("a server");
        let response = reqwest::Client::new()
            .post(format!("http://{}/mcp", server.address()))
            .header("Origin", "http://example.com")
            .header("Accept", "application/json, text/event-stream")
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "1"},
                },
            }))
            .send()
            .await
            .expect("an answer");
        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    }

    fn text(result: &CallToolResult) -> &str {
        let text = result.content.first().and_then(|content| content.as_text());
        text.map_or("", |text| text.text.as_str())
    }

    #[tokio::test]
    async fn a_call_with_no_project_open_fails() {
        let result = Handler::new().answer("get_project", json!({})).await;
        assert_eq!(result.is_error, Some(true));
        assert_eq!(text(&result), "no project is open");
    }

    /// An outside agent's call is held to the tool's schema, as the agent's
    /// own is.
    #[tokio::test]
    async fn a_call_that_does_not_fit_its_tool_is_refused() {
        let arguments = json!({"label": "x", "ops": [{"op": "add_file", "bytes": []}]});
        let result = Handler::new().answer("edit", arguments).await;
        assert_eq!(result.is_error, Some(true));
        assert!(text(&result).contains("schema"), "{}", text(&result));
    }
}
