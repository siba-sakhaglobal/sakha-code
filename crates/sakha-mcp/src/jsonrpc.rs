//! Minimal JSON-RPC 2.0 message envelopes used to frame MCP protocol
//! messages over any `McpTransport`. See spec
//! `modules/10-mcp-plugin-connector-system.md` ("MCP stdio client").

use serde::{Deserialize, Serialize};
use serde_json::Value;

use sakha_core::{SakhaError, SakhaResult};

/// A JSON-RPC 2.0 request envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(id: impl Into<Value>, method: impl Into<String>, params: Option<Value>) -> Self {
        Self { jsonrpc: "2.0".to_string(), id: id.into(), method: method.into(), params }
    }
}

/// A JSON-RPC 2.0 notification envelope (no `id`, no response expected).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcNotification {
    pub fn new(method: impl Into<String>, params: Option<Value>) -> Self {
        Self { jsonrpc: "2.0".to_string(), method: method.into(), params }
    }
}

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcErrorObject {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// A JSON-RPC 2.0 response envelope. Exactly one of `result`/`error` is set,
/// per spec; both are optional here so we can deserialize either shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcErrorObject>,
}

impl JsonRpcResponse {
    /// Converts this response into a `SakhaResult`, mapping a JSON-RPC error
    /// object into a `SakhaError` and an absent `result` into `Value::Null`.
    pub fn into_result(self, source_module: &str) -> SakhaResult<Value> {
        if let Some(err) = self.error {
            return Err(SakhaError::transient(
                source_module,
                format!("mcp rpc error {}: {}", err.code, err.message),
            ));
        }
        Ok(self.result.unwrap_or(Value::Null))
    }
}

/// Parses a raw transport payload as either a JSON-RPC response or an error,
/// tolerating servers that omit `jsonrpc` on responses.
pub fn parse_response(value: Value) -> SakhaResult<JsonRpcResponse> {
    serde_json::from_value(value)
        .map_err(|e| SakhaError::integrity("sakha-mcp", format!("invalid JSON-RPC response: {e}")))
}
