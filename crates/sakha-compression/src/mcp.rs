//! Headroom-over-MCP client adapter: implements `HeadroomClient` by calling
//! `headroom_compress` / `headroom_retrieve` / `headroom_stats` MCP tools.
//!
//! `sakha-compression` does not depend on the concrete `sakha-mcp` crate (to
//! avoid a dependency cycle, since MCP tool bridging can itself route through
//! compression). Instead `HeadroomMcpClient` is generic over an
//! `McpToolInvoker`: anything that can call a named MCP tool with JSON
//! arguments and return a JSON result. `sakha-mcp`'s `McpConnection::call_tool`
//! satisfies this shape directly.

use async_trait::async_trait;

use sakha_core::{SakhaError, SakhaResult};

use crate::headroom::{HeadroomClient, HeadroomCompressRequest, HeadroomCompressResponse, HeadroomMode};

/// Minimal capability an MCP client connection must provide for
/// `HeadroomMcpClient` to drive the `headroom_*` tools: call a named tool
/// with JSON arguments, get back a JSON result. See spec "MCP mode": expose/
/// use Headroom MCP tools (`headroom_compress`, `headroom_retrieve`,
/// `headroom_stats`).
#[async_trait]
pub trait McpToolInvoker: Send + Sync {
    async fn call_tool(&self, name: &str, arguments: serde_json::Value) -> SakhaResult<serde_json::Value>;
}

/// Talks to Headroom via an MCP server exposing the `headroom_*` tools,
/// through any `McpToolInvoker` (typically a live `McpConnection` from
/// `sakha-mcp`, wired in by the caller).
pub struct HeadroomMcpClient {
    pub server_name: String,
    invoker: std::sync::Arc<dyn McpToolInvoker>,
}

impl HeadroomMcpClient {
    /// Builds a client with no live connection; `compress`/`retrieve`/`stats`
    /// will report Headroom as unavailable until `with_invoker` is used
    /// instead. Kept so callers that only need `server_name`/`mode` (e.g. to
    /// populate config UI) don't need a live MCP connection up front.
    pub fn new(server_name: impl Into<String>) -> Self {
        Self { server_name: server_name.into(), invoker: std::sync::Arc::new(NoInvoker) }
    }

    /// Builds a client that drives the `headroom_*` tools through `invoker`
    /// (e.g. an `Arc<McpConnection>` from `sakha-mcp` after `initialize()`).
    pub fn with_invoker(server_name: impl Into<String>, invoker: std::sync::Arc<dyn McpToolInvoker>) -> Self {
        Self { server_name: server_name.into(), invoker }
    }
}

/// Default invoker for a `HeadroomMcpClient` built via `new`: reports
/// Headroom unavailable rather than panicking, per the "Headroom unavailable"
/// failure mode.
struct NoInvoker;

#[async_trait]
impl McpToolInvoker for NoInvoker {
    async fn call_tool(&self, _name: &str, _arguments: serde_json::Value) -> SakhaResult<serde_json::Value> {
        Err(SakhaError::not_implemented("sakha-compression", "HeadroomMcpClient: no MCP connection configured"))
    }
}

#[async_trait]
impl HeadroomClient for HeadroomMcpClient {
    async fn compress(&self, request: HeadroomCompressRequest) -> SakhaResult<HeadroomCompressResponse> {
        let args = serde_json::json!({
            "content": request.content,
            "content_kind": request.content_kind,
        });
        let result = self.invoker.call_tool("headroom_compress", args).await?;
        let compressed = result
            .get("compressed")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SakhaError::integrity("sakha-compression", "headroom_compress MCP result missing 'compressed'"))?
            .to_string();
        let marker_id = result
            .get("marker_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SakhaError::integrity("sakha-compression", "headroom_compress MCP result missing 'marker_id'"))?
            .to_string();
        Ok(HeadroomCompressResponse { compressed, marker_id })
    }

    async fn retrieve(&self, marker_id: &str) -> SakhaResult<String> {
        let args = serde_json::json!({ "marker_id": marker_id });
        let result = self.invoker.call_tool("headroom_retrieve", args).await?;
        result
            .get("content")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| SakhaError::integrity("sakha-compression", "headroom_retrieve MCP result missing 'content'"))
    }

    async fn stats(&self) -> SakhaResult<serde_json::Value> {
        self.invoker.call_tool("headroom_stats", serde_json::json!({})).await
    }

    fn mode(&self) -> HeadroomMode {
        HeadroomMode::Mcp
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A fake `McpToolInvoker` that behaves like a real Headroom MCP server
    /// for `headroom_compress`/`headroom_retrieve`/`headroom_stats`.
    struct FakeHeadroomMcpServer;

    #[async_trait]
    impl McpToolInvoker for FakeHeadroomMcpServer {
        async fn call_tool(&self, name: &str, arguments: serde_json::Value) -> SakhaResult<serde_json::Value> {
            match name {
                "headroom_compress" => {
                    let content = arguments.get("content").and_then(|v| v.as_str()).unwrap_or_default();
                    Ok(serde_json::json!({
                        "compressed": format!("compressed:{content}"),
                        "marker_id": "ccr:mcp-marker-1",
                    }))
                }
                "headroom_retrieve" => Ok(serde_json::json!({ "content": "original content" })),
                "headroom_stats" => Ok(serde_json::json!({ "items_compressed": 1 })),
                other => Err(SakhaError::invalid_input("sakha-compression", format!("unknown tool {other}"))),
            }
        }
    }

    #[tokio::test]
    async fn compress_calls_headroom_compress_tool_and_parses_response() {
        let client = HeadroomMcpClient::with_invoker("headroom", Arc::new(FakeHeadroomMcpServer));
        let response = client
            .compress(HeadroomCompressRequest { content: "hello".into(), content_kind: "tool_output".into() })
            .await
            .unwrap();
        assert_eq!(response.compressed, "compressed:hello");
        assert_eq!(response.marker_id, "ccr:mcp-marker-1");
    }

    #[tokio::test]
    async fn retrieve_calls_headroom_retrieve_tool() {
        let client = HeadroomMcpClient::with_invoker("headroom", Arc::new(FakeHeadroomMcpServer));
        let content = client.retrieve("ccr:mcp-marker-1").await.unwrap();
        assert_eq!(content, "original content");
    }

    #[tokio::test]
    async fn stats_calls_headroom_stats_tool() {
        let client = HeadroomMcpClient::with_invoker("headroom", Arc::new(FakeHeadroomMcpServer));
        let stats = client.stats().await.unwrap();
        assert_eq!(stats["items_compressed"], 1);
    }

    #[tokio::test]
    async fn no_invoker_configured_returns_error_not_panic() {
        let client = HeadroomMcpClient::new("headroom");
        let result = client
            .compress(HeadroomCompressRequest { content: "x".into(), content_kind: "text".into() })
            .await;
        assert!(result.is_err());
        assert_eq!(client.mode(), HeadroomMode::Mcp);
    }
}
