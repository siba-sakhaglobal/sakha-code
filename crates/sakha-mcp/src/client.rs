//! `McpClient`/`McpClientManager`: connection lifecycle and tool/resource
//! discovery against a single MCP server.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use sakha_core::{SakhaError, SakhaResult};

use crate::jsonrpc::{JsonRpcNotification, JsonRpcRequest};
use crate::server_registry::McpServerConfig;
use crate::transport::McpTransport;

/// A discovered MCP tool definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// A discovered MCP resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResourceDescriptor {
    pub uri: String,
    pub name: String,
    pub mime_type: Option<String>,
}

/// Protocol version Sakha speaks when initializing an MCP session.
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// A single MCP client connection: handshake, discovery, and tool
/// invocation against one server.
pub struct McpConnection {
    pub config: McpServerConfig,
    pub transport: Box<dyn McpTransport>,
    next_id: AtomicI64,
    initialized: std::sync::atomic::AtomicBool,
}

impl McpConnection {
    pub fn new(config: McpServerConfig, transport: Box<dyn McpTransport>) -> Self {
        Self { config, transport, next_id: AtomicI64::new(1), initialized: std::sync::atomic::AtomicBool::new(false) }
    }

    fn next_request_id(&self) -> i64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Sends a JSON-RPC request and awaits the correlated response. The fake
    /// and stdio transports here are strictly request/response (one message
    /// out, one message in), matching how MCP stdio servers behave for a
    /// single in-flight call; concurrent pipelining is out of scope.
    async fn call(&self, method: &str, params: Option<serde_json::Value>) -> SakhaResult<serde_json::Value> {
        let id = self.next_request_id();
        let request = JsonRpcRequest::new(serde_json::json!(id), method, params);
        let payload = serde_json::to_value(&request)
            .map_err(|e| SakhaError::integrity("sakha-mcp", format!("failed to encode MCP request: {e}")))?;
        self.transport.send(crate::transport::RpcMessage(payload)).await?;
        let response = self.transport.receive().await?;
        let parsed = crate::jsonrpc::parse_response(response.0)?;
        if parsed.id != serde_json::json!(id) {
            return Err(SakhaError::integrity(
                "sakha-mcp",
                format!("MCP response id {:?} did not match request id {id}", parsed.id),
            ));
        }
        parsed.into_result("sakha-mcp")
    }

    async fn notify(&self, method: &str, params: Option<serde_json::Value>) -> SakhaResult<()> {
        let notification = JsonRpcNotification::new(method, params);
        let payload = serde_json::to_value(&notification)
            .map_err(|e| SakhaError::integrity("sakha-mcp", format!("failed to encode MCP notification: {e}")))?;
        self.transport.send(crate::transport::RpcMessage(payload)).await
    }

    /// Performs the MCP `initialize` handshake followed by the
    /// `notifications/initialized` notification.
    pub async fn initialize(&self) -> SakhaResult<()> {
        let params = serde_json::json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "sakha", "version": env!("CARGO_PKG_VERSION") },
        });
        self.call("initialize", Some(params)).await?;
        self.notify("notifications/initialized", None).await?;
        self.initialized.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::SeqCst)
    }

    fn require_initialized(&self) -> SakhaResult<()> {
        if !self.is_initialized() {
            return Err(SakhaError::invalid_input(
                "sakha-mcp",
                format!("MCP connection to '{}' used before initialize()", self.config.name),
            ));
        }
        Ok(())
    }

    pub async fn list_tools(&self) -> SakhaResult<Vec<McpToolDescriptor>> {
        self.require_initialized()?;
        let result = self.call("tools/list", None).await?;
        let tools = result
            .get("tools")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Array(vec![]));
        let raw: Vec<RawTool> = serde_json::from_value(tools)
            .map_err(|e| SakhaError::integrity("sakha-mcp", format!("invalid tools/list result: {e}")))?;
        Ok(raw
            .into_iter()
            .map(|t| McpToolDescriptor {
                name: t.name,
                description: t.description.unwrap_or_default(),
                input_schema: t.input_schema.unwrap_or_else(|| serde_json::json!({"type": "object"})),
            })
            .collect())
    }

    pub async fn list_resources(&self) -> SakhaResult<Vec<McpResourceDescriptor>> {
        self.require_initialized()?;
        let result = self.call("resources/list", None).await?;
        let resources = result
            .get("resources")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Array(vec![]));
        let raw: Vec<RawResource> = serde_json::from_value(resources)
            .map_err(|e| SakhaError::integrity("sakha-mcp", format!("invalid resources/list result: {e}")))?;
        Ok(raw
            .into_iter()
            .map(|r| McpResourceDescriptor { uri: r.uri, name: r.name.unwrap_or_default(), mime_type: r.mime_type })
            .collect())
    }

    pub async fn call_tool(&self, name: &str, arguments: serde_json::Value) -> SakhaResult<serde_json::Value> {
        self.require_initialized()?;
        let params = serde_json::json!({ "name": name, "arguments": arguments });
        let result = self.call("tools/call", Some(params)).await?;
        if let Some(true) = result.get("isError").and_then(|v| v.as_bool()) {
            let message = result
                .get("content")
                .map(|c| c.to_string())
                .unwrap_or_else(|| "MCP tool call reported an error".to_string());
            return Err(SakhaError::invalid_input("sakha-mcp", format!("MCP tool '{name}' failed: {message}")));
        }
        Ok(result)
    }
}

#[derive(Debug, Deserialize)]
struct RawTool {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(rename = "inputSchema", default)]
    input_schema: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct RawResource {
    uri: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(rename = "mimeType", default)]
    mime_type: Option<String>,
}

/// Manages the set of live `McpConnection`s.
#[async_trait]
pub trait McpClientManager: Send + Sync {
    async fn connect(&mut self, config: McpServerConfig) -> SakhaResult<()>;
    async fn disconnect(&mut self, server_name: &str) -> SakhaResult<()>;
    async fn list_tools(&self, server_name: &str) -> SakhaResult<Vec<McpToolDescriptor>>;
}

/// A `McpClientManager` backed by a transport factory, so callers can plug
/// in `StdioTransport::new` for real servers or a fake-transport factory in
/// tests without changing manager logic.
pub struct DefaultMcpClientManager {
    transport_factory: Box<dyn Fn(&McpServerConfig) -> Box<dyn McpTransport> + Send + Sync>,
    connections: HashMap<String, McpConnection>,
}

impl DefaultMcpClientManager {
    pub fn new<F>(transport_factory: F) -> Self
    where
        F: Fn(&McpServerConfig) -> Box<dyn McpTransport> + Send + Sync + 'static,
    {
        Self { transport_factory: Box::new(transport_factory), connections: HashMap::new() }
    }

    /// A manager whose transport factory spawns real stdio child processes,
    /// keyed off `McpServerConfig::command`.
    pub fn stdio() -> Self {
        Self::new(|config: &McpServerConfig| -> Box<dyn McpTransport> {
            match &config.command {
                Some(command) => Box::new(crate::transport::StdioTransport::new(command.clone(), config.args.clone())),
                None => Box::new(crate::transport::DisconnectedTransport),
            }
        })
    }

    pub fn connection(&self, server_name: &str) -> SakhaResult<&McpConnection> {
        self.connections
            .get(server_name)
            .ok_or_else(|| SakhaError::invalid_input("sakha-mcp", format!("not connected to MCP server: {server_name}")))
    }
}

#[async_trait]
impl McpClientManager for DefaultMcpClientManager {
    async fn connect(&mut self, config: McpServerConfig) -> SakhaResult<()> {
        let transport = (self.transport_factory)(&config);
        let connection = McpConnection::new(config.clone(), transport);
        connection.initialize().await?;
        self.connections.insert(config.name.clone(), connection);
        Ok(())
    }

    async fn disconnect(&mut self, server_name: &str) -> SakhaResult<()> {
        self.connections.remove(server_name);
        Ok(())
    }

    async fn list_tools(&self, server_name: &str) -> SakhaResult<Vec<McpToolDescriptor>> {
        self.connection(server_name)?.list_tools().await
    }
}
