//! Registry of configured MCP servers, and `McpServerManager`: the
//! top-level facade tying server config, connection lifecycle, trust
//! enforcement, and tool/resource bridging together (spec "Main Structs":
//! `McpServerManager`; "Implementation Tasks": "Implement MCP client
//! connection lifecycle", "Implement tool proxy", "Implement resource
//! proxy").

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use sakha_core::{SakhaError, SakhaResult};
use sakha_tools::{Tool, ToolRegistry};

use crate::client::McpConnection;
use crate::tool_bridge::{McpResourceProxy, McpToolBridge};
use crate::transport::McpTransport;
use crate::trust::{TrustLevel, TrustPolicy};

/// Configuration for a single MCP server connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
    pub trust_level: TrustLevel,
}

/// Holds configured MCP servers by name.
#[derive(Debug, Default)]
pub struct McpServerRegistry {
    servers: std::collections::HashMap<String, McpServerConfig>,
}

impl McpServerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a registry from a list of server configs, e.g. loaded from a
    /// workspace/user config file. Later entries with the same name win.
    pub fn from_configs(configs: impl IntoIterator<Item = McpServerConfig>) -> Self {
        let mut registry = Self::new();
        for config in configs {
            registry.register(config);
        }
        registry
    }

    pub fn register(&mut self, config: McpServerConfig) {
        self.servers.insert(config.name.clone(), config);
    }

    pub fn unregister(&mut self, name: &str) -> Option<McpServerConfig> {
        self.servers.remove(name)
    }

    pub fn get(&self, name: &str) -> Option<&McpServerConfig> {
        self.servers.get(name)
    }

    pub fn get_or_err(&self, name: &str) -> SakhaResult<&McpServerConfig> {
        self.get(name).ok_or_else(|| SakhaError::invalid_input("sakha-mcp", format!("unknown MCP server: {name}")))
    }

    pub fn list(&self) -> Vec<&McpServerConfig> {
        self.servers.values().collect()
    }
}

/// Top-level MCP facade: holds configured servers, connects to them (via a
/// pluggable transport factory), enforces `TrustPolicy` before bridging, and
/// exposes every trusted server's tools/resources as `sakha-tools::Tool`s in
/// a shared `ToolRegistry`. This is what CLI/agent wiring drives day to day,
/// rather than talking to `McpConnection`/`McpToolBridge` directly.
pub struct McpServerManager {
    registry: McpServerRegistry,
    connections: HashMap<String, Arc<McpConnection>>,
    trust_policy: TrustPolicy,
    transport_factory: Box<dyn Fn(&McpServerConfig) -> Box<dyn McpTransport> + Send + Sync>,
}

impl McpServerManager {
    pub fn new<F>(trust_policy: TrustPolicy, transport_factory: F) -> Self
    where
        F: Fn(&McpServerConfig) -> Box<dyn McpTransport> + Send + Sync + 'static,
    {
        Self {
            registry: McpServerRegistry::new(),
            connections: HashMap::new(),
            trust_policy,
            transport_factory: Box::new(transport_factory),
        }
    }

    /// A manager whose transport factory spawns real stdio child processes
    /// for servers with a configured `command`, matching
    /// `DefaultMcpClientManager::stdio`'s behavior.
    pub fn stdio(trust_policy: TrustPolicy) -> Self {
        Self::new(trust_policy, |config: &McpServerConfig| -> Box<dyn McpTransport> {
            match &config.command {
                Some(command) => Box::new(crate::transport::StdioTransport::new(command.clone(), config.args.clone())),
                None => Box::new(crate::transport::DisconnectedTransport),
            }
        })
    }

    pub fn register_server(&mut self, config: McpServerConfig) {
        self.registry.register(config);
    }

    /// Connects to `server_name` (must already be registered) and performs
    /// the `initialize` handshake. Idempotent: reconnecting an
    /// already-connected server replaces the old connection.
    pub async fn connect(&mut self, server_name: &str) -> SakhaResult<()> {
        let config = self.registry.get_or_err(server_name)?.clone();
        let transport = (self.transport_factory)(&config);
        let connection = McpConnection::new(config, transport);
        connection.initialize().await?;
        self.connections.insert(server_name.to_string(), Arc::new(connection));
        Ok(())
    }

    pub fn disconnect(&mut self, server_name: &str) {
        self.connections.remove(server_name);
    }

    pub fn connection(&self, server_name: &str) -> SakhaResult<Arc<McpConnection>> {
        self.connections
            .get(server_name)
            .cloned()
            .ok_or_else(|| SakhaError::invalid_input("sakha-mcp", format!("not connected to MCP server: {server_name}")))
    }

    /// Discovers tools and resources on `server_name` and bridges every one
    /// permitted by `trust_policy` into `registry`. Returns the number of
    /// tools bridged (resources are bridged as additional tools too, so the
    /// total registry growth is `tools + resources`).
    pub async fn bridge_into(&self, server_name: &str, registry: &mut ToolRegistry) -> SakhaResult<usize> {
        let config = self.registry.get_or_err(server_name)?;
        let connection = self.connection(server_name)?;

        let tools = connection.list_tools().await?;
        let tool_bridge = McpToolBridge::new();
        let bridged_tools = tool_bridge.bridge_executable(server_name, config.trust_level, &self.trust_policy, connection.clone(), tools)?;
        let tool_count = bridged_tools.len();
        for tool in bridged_tools {
            register(registry, tool);
        }

        let resources = connection.list_resources().await.unwrap_or_default();
        if !resources.is_empty() {
            let resource_proxy = McpResourceProxy::new();
            let bridged_resources =
                resource_proxy.bridge_executable(server_name, config.trust_level, &self.trust_policy, connection, resources)?;
            for tool in bridged_resources {
                register(registry, tool);
            }
        }

        Ok(tool_count)
    }

    pub fn registry(&self) -> &McpServerRegistry {
        &self.registry
    }
}

fn register(registry: &mut ToolRegistry, tool: Arc<dyn Tool>) {
    registry.register(tool);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::FakeTransport;

    fn spawn_fake_server(server: FakeTransport, tools: Vec<serde_json::Value>, resources: Vec<serde_json::Value>) {
        tokio::spawn(async move {
            loop {
                let msg = match server.receive().await {
                    Ok(m) => m,
                    Err(_) => break,
                };
                let req: serde_json::Value = msg.0;
                let method = req.get("method").and_then(|m| m.as_str()).unwrap_or_default();
                let id = match req.get("id") {
                    Some(id) => id.clone(),
                    None => continue,
                };
                let result = match method {
                    "initialize" => serde_json::json!({
                        "protocolVersion": crate::client::MCP_PROTOCOL_VERSION,
                        "serverInfo": { "name": "fake", "version": "0.0.0" },
                        "capabilities": {},
                    }),
                    "tools/list" => serde_json::json!({ "tools": tools }),
                    "resources/list" => serde_json::json!({ "resources": resources }),
                    "resources/read" => serde_json::json!({ "contents": [{"text": "resource body"}] }),
                    "tools/call" => serde_json::json!({ "content": [{"type": "text", "text": "ok"}], "isError": false }),
                    other => serde_json::json!({ "error": format!("unknown method {other}") }),
                };
                let response = serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
                if server.send(crate::transport::RpcMessage(response)).await.is_err() {
                    break;
                }
            }
        });
    }

    #[tokio::test]
    async fn manager_connects_and_bridges_tools_and_resources_into_registry() {
        let (client_transport, server_transport) = FakeTransport::pair();
        spawn_fake_server(
            server_transport,
            vec![serde_json::json!({"name": "echo", "description": "echoes", "inputSchema": {"type": "object"}})],
            vec![serde_json::json!({"uri": "file:///a.txt", "name": "a"})],
        );

        let slot = std::sync::Mutex::new(Some(client_transport));
        let mut manager = McpServerManager::new(TrustPolicy::new(TrustLevel::Workspace), move |_cfg| {
            let transport = slot.lock().unwrap().take().expect("transport factory invoked more than once");
            Box::new(transport) as Box<dyn McpTransport>
        });
        manager.register_server(McpServerConfig {
            name: "fake".into(),
            command: None,
            args: vec![],
            url: None,
            trust_level: TrustLevel::LocalDev,
        });
        manager.connect("fake").await.unwrap();

        let mut registry = ToolRegistry::new();
        let bridged = manager.bridge_into("fake", &mut registry).await.unwrap();
        assert_eq!(bridged, 1);
        // 1 tool + 1 resource proxy tool.
        assert_eq!(registry.len(), 2);
        assert!(registry.get(&sakha_tools::ToolName::new("mcp.fake.echo")).is_some());
    }

    #[tokio::test]
    async fn manager_bridge_into_untrusted_server_is_rejected() {
        let (client_transport, server_transport) = FakeTransport::pair();
        spawn_fake_server(server_transport, vec![serde_json::json!({"name": "danger"})], vec![]);

        let slot = std::sync::Mutex::new(Some(client_transport));
        let mut manager = McpServerManager::new(TrustPolicy::new(TrustLevel::OrganizationApproved), move |_cfg| {
            let transport = slot.lock().unwrap().take().expect("transport factory invoked more than once");
            Box::new(transport) as Box<dyn McpTransport>
        });
        manager.register_server(McpServerConfig {
            name: "sketchy".into(),
            command: None,
            args: vec![],
            url: None,
            trust_level: TrustLevel::MarketplaceUnverified,
        });
        manager.connect("sketchy").await.unwrap();

        let mut registry = ToolRegistry::new();
        let result = manager.bridge_into("sketchy", &mut registry).await;
        assert!(result.is_err());
        assert!(registry.is_empty());
    }
}
