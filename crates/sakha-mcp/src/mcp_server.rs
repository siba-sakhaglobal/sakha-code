//! `SakhaMcpServer`: exposes a `sakha-tools::ToolRegistry` as an MCP server
//! (spec "Features": "MCP server exposing Sakha tools/resources/prompts";
//! "Implementation Tasks": "Implement Sakha MCP server"). Speaks JSON-RPC 2.0
//! newline-delimited messages, so it can run over stdio (an external MCP
//! client spawns Sakha as a child process) or be driven directly in-process
//! via `handle_message` (used by the daemon/tests without a subprocess).

use std::sync::Arc;

use serde_json::Value;

use sakha_core::SakhaResult;
use sakha_tools::{ToolContext, ToolName, ToolRegistry};

use crate::client::MCP_PROTOCOL_VERSION;

/// Serves `registry`'s tools over MCP JSON-RPC. Resource/prompt discovery is
/// intentionally minimal (empty lists) since `sakha-tools` has no resource/
/// prompt concept yet to expose; `tools/list` and `tools/call` are fully
/// functional, which is the primary "expose Sakha tools ... over MCP"
/// requirement.
pub struct SakhaMcpServer {
    pub registry: Arc<ToolRegistry>,
    pub workspace_root: std::path::PathBuf,
}

impl SakhaMcpServer {
    pub fn new(registry: Arc<ToolRegistry>, workspace_root: impl Into<std::path::PathBuf>) -> Self {
        Self { registry, workspace_root: workspace_root.into() }
    }

    /// Handles one incoming JSON-RPC request/notification and returns the
    /// response to send back (`None` for notifications, which get no
    /// response). Never panics: malformed requests get a JSON-RPC error
    /// response rather than propagating a parse failure.
    pub async fn handle_message(&self, request: Value) -> Option<Value> {
        let id = request.get("id").cloned();
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or_default().to_string();
        let params = request.get("params").cloned().unwrap_or(Value::Null);

        // A notification (no "id") never gets a response, per JSON-RPC 2.0.
        let id = id?;

        let result = self.dispatch(&method, params).await;
        Some(match result {
            Ok(value) => serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": value }),
            Err(err) => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32000, "message": err.to_string() },
            }),
        })
    }

    async fn dispatch(&self, method: &str, params: Value) -> SakhaResult<Value> {
        match method {
            "initialize" => Ok(serde_json::json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "serverInfo": { "name": "sakha", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "tools": {}, "resources": {}, "prompts": {} },
            })),
            "tools/list" => {
                let tools: Vec<Value> = self
                    .registry
                    .specs()
                    .into_iter()
                    .map(|spec| {
                        serde_json::json!({
                            "name": spec.name.0,
                            "description": spec.description,
                            "inputSchema": spec.input_schema.0,
                        })
                    })
                    .collect();
                Ok(serde_json::json!({ "tools": tools }))
            }
            "tools/call" => self.call_tool(params).await,
            "resources/list" => Ok(serde_json::json!({ "resources": [] })),
            "prompts/list" => Ok(serde_json::json!({ "prompts": [] })),
            other => Err(sakha_core::SakhaError::invalid_input("sakha-mcp", format!("unknown MCP method: {other}"))),
        }
    }

    async fn call_tool(&self, params: Value) -> SakhaResult<Value> {
        let name = params
            .get("name")
            .and_then(|n| n.as_str())
            .ok_or_else(|| sakha_core::SakhaError::invalid_input("sakha-mcp", "tools/call missing 'name'"))?;
        let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);

        let tool = self.registry.get_or_err(&ToolName::new(name))?;
        let validated = tool.validate(arguments)?;
        let context = ToolContext::new(self.workspace_root.clone());
        match tool.execute(&validated, &context).await {
            Ok(result) => Ok(serde_json::json!({
                "content": [{"type": "text", "text": serde_json::to_string(&result.output_json).unwrap_or_default()}],
                "isError": false,
            })),
            Err(err) => Ok(serde_json::json!({
                "content": [{"type": "text", "text": err.to_string()}],
                "isError": true,
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Arc<ToolRegistry> {
        Arc::new(sakha_tools::default_registry())
    }

    #[tokio::test]
    async fn initialize_reports_protocol_version() {
        let server = SakhaMcpServer::new(registry(), ".");
        let response = server
            .handle_message(serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}))
            .await
            .unwrap();
        assert_eq!(response["result"]["protocolVersion"], MCP_PROTOCOL_VERSION);
    }

    #[tokio::test]
    async fn tools_list_exposes_builtin_tools() {
        let server = SakhaMcpServer::new(registry(), ".");
        let response = server
            .handle_message(serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
            .await
            .unwrap();
        let tools = response["result"]["tools"].as_array().unwrap();
        assert!(!tools.is_empty());
        assert!(tools.iter().any(|t| t["name"] == "file.read"));
    }

    #[tokio::test]
    async fn tools_call_unknown_tool_returns_error_response_not_panic() {
        let server = SakhaMcpServer::new(registry(), ".");
        let response = server
            .handle_message(serde_json::json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": { "name": "nonexistent.tool", "arguments": {} },
            }))
            .await
            .unwrap();
        assert!(response.get("error").is_some());
    }

    #[tokio::test]
    async fn notification_without_id_gets_no_response() {
        let server = SakhaMcpServer::new(registry(), ".");
        let response = server.handle_message(serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;
        assert!(response.is_none());
    }

    #[tokio::test]
    async fn resources_and_prompts_list_return_empty_not_error() {
        let server = SakhaMcpServer::new(registry(), ".");
        let resources = server.handle_message(serde_json::json!({"jsonrpc": "2.0", "id": 4, "method": "resources/list"})).await.unwrap();
        assert_eq!(resources["result"]["resources"].as_array().unwrap().len(), 0);
        let prompts = server.handle_message(serde_json::json!({"jsonrpc": "2.0", "id": 5, "method": "prompts/list"})).await.unwrap();
        assert_eq!(prompts["result"]["prompts"].as_array().unwrap().len(), 0);
    }
}
