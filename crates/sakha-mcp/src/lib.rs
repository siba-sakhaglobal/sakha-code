//! sakha-mcp: MCP client, transport, server registry, tool bridge, trust policy.
//!
//! Public API skeleton — see spec `modules/10-mcp-plugin-connector-system.md`
//! and `crates/crate-work-breakdown.md`.

pub mod client;
pub mod jsonrpc;
pub mod server_registry;
pub mod tool_bridge;
pub mod transport;
pub mod trust;

pub use client::{
    DefaultMcpClientManager, McpClientManager, McpConnection, McpResourceDescriptor, McpToolDescriptor,
    MCP_PROTOCOL_VERSION,
};
pub use server_registry::{McpServerConfig, McpServerRegistry};
pub use tool_bridge::{namespaced_tool_name, McpProxyTool, McpToolBridge};
pub use transport::{DisconnectedTransport, FakeTransport, McpTransport, RpcMessage, StdioTransport, TransportKind};
pub use trust::{TrustLevel, TrustPolicy};

pub fn crate_name() -> &'static str {
    "sakha-mcp"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use sakha_tools::{ToolContext, ToolRegistry};

    /// Spawns a minimal fake MCP server loop on the server-side end of a
    /// `FakeTransport` pair, responding to `initialize`, `tools/list`, and
    /// `tools/call` like a real (very small) MCP server would.
    fn spawn_fake_server(server: FakeTransport, tools: Vec<serde_json::Value>) {
        tokio::spawn(async move {
            loop {
                let msg = match server.receive().await {
                    Ok(m) => m,
                    Err(_) => break,
                };
                let req: serde_json::Value = msg.0;
                let method = req.get("method").and_then(|m| m.as_str()).unwrap_or_default();
                // Notifications (no "id") get no response.
                let id = match req.get("id") {
                    Some(id) => id.clone(),
                    None => continue,
                };
                let result = match method {
                    "initialize" => serde_json::json!({
                        "protocolVersion": MCP_PROTOCOL_VERSION,
                        "serverInfo": { "name": "fake", "version": "0.0.0" },
                        "capabilities": {},
                    }),
                    "tools/list" => serde_json::json!({ "tools": tools }),
                    "tools/call" => {
                        let name = req
                            .get("params")
                            .and_then(|p| p.get("name"))
                            .and_then(|n| n.as_str())
                            .unwrap_or_default();
                        serde_json::json!({
                            "content": [{"type": "text", "text": format!("called {name}")}],
                            "isError": false,
                        })
                    }
                    other => serde_json::json!({ "error": format!("unknown method {other}") }),
                };
                let response = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": result,
                });
                if server.send(RpcMessage(response)).await.is_err() {
                    break;
                }
            }
        });
    }

    #[test]
    fn server_registry_registers_and_looks_up() {
        let mut registry = McpServerRegistry::new();
        registry.register(McpServerConfig {
            name: "headroom".into(),
            command: Some("headroom".into()),
            args: vec!["mcp".into()],
            url: None,
            trust_level: TrustLevel::LocalDev,
        });
        assert!(registry.get("headroom").is_some());
        assert!(registry.get_or_err("missing").is_err());
    }

    #[test]
    fn server_registry_builds_from_configs() {
        let registry = McpServerRegistry::from_configs(vec![McpServerConfig {
            name: "search".into(),
            command: Some("search-mcp".into()),
            args: vec![],
            url: None,
            trust_level: TrustLevel::Workspace,
        }]);
        assert_eq!(registry.list().len(), 1);
    }

    #[test]
    fn untrusted_server_blocked_by_trust_policy() {
        let policy = TrustPolicy::new(TrustLevel::OrganizationApproved);
        assert!(!policy.allows(TrustLevel::MarketplaceUnverified));
        assert!(policy.allows(TrustLevel::Workspace));
    }

    #[test]
    fn tool_bridge_namespaces_tool_names_by_server() {
        let bridge = McpToolBridge::new();
        let tools = vec![McpToolDescriptor {
            name: "compress".into(),
            description: "compress content".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        let bridged = bridge.bridge("headroom", tools).unwrap();
        assert_eq!(bridged[0].name.0, "mcp.headroom.compress");
    }

    #[tokio::test]
    async fn disconnected_transport_errors_on_send() {
        let transport = DisconnectedTransport;
        let result = transport.send(RpcMessage(serde_json::Value::Null)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn handshake_initialize_over_fake_transport() {
        let (client_transport, server_transport) = FakeTransport::pair();
        spawn_fake_server(server_transport, vec![]);

        let config = McpServerConfig {
            name: "fake".into(),
            command: None,
            args: vec![],
            url: None,
            trust_level: TrustLevel::LocalDev,
        };
        let connection = McpConnection::new(config, Box::new(client_transport));
        assert!(!connection.is_initialized());
        connection.initialize().await.unwrap();
        assert!(connection.is_initialized());
    }

    #[tokio::test]
    async fn tool_list_bridges_into_tool_registry() {
        let (client_transport, server_transport) = FakeTransport::pair();
        let fake_tools = vec![serde_json::json!({
            "name": "echo",
            "description": "echoes input",
            "inputSchema": {"type": "object"},
        })];
        spawn_fake_server(server_transport, fake_tools);

        let config = McpServerConfig {
            name: "fake".into(),
            command: None,
            args: vec![],
            url: None,
            trust_level: TrustLevel::LocalDev,
        };
        let connection = McpConnection::new(config, Box::new(client_transport));
        connection.initialize().await.unwrap();
        let tools = connection.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");

        let bridge = McpToolBridge::new();
        let trust_policy = TrustPolicy::new(TrustLevel::Workspace);
        let executable = bridge
            .bridge_executable("fake", TrustLevel::LocalDev, &trust_policy, Arc::new(connection), tools)
            .unwrap();
        assert_eq!(executable.len(), 1);

        let mut registry = ToolRegistry::new();
        for tool in executable {
            registry.register(tool);
        }
        assert_eq!(registry.len(), 1);
        let found = registry.get(&sakha_tools::ToolName::new("mcp.fake.echo")).unwrap();
        let context = ToolContext::new(".");
        let validated = found.validate(serde_json::json!({"text": "hi"})).unwrap();
        let result = found.execute(&validated, &context).await.unwrap();
        assert_eq!(result.status, sakha_tools::ToolCallStatus::Succeeded);
    }

    #[tokio::test]
    async fn untrusted_server_tools_are_not_bridged() {
        let (client_transport, server_transport) = FakeTransport::pair();
        spawn_fake_server(server_transport, vec![serde_json::json!({"name": "danger"})]);

        let config = McpServerConfig {
            name: "sketchy".into(),
            command: None,
            args: vec![],
            url: None,
            trust_level: TrustLevel::MarketplaceUnverified,
        };
        let connection = McpConnection::new(config, Box::new(client_transport));
        connection.initialize().await.unwrap();
        let tools = connection.list_tools().await.unwrap();

        let bridge = McpToolBridge::new();
        let trust_policy = TrustPolicy::new(TrustLevel::OrganizationApproved);
        let result = bridge.bridge_executable(
            "sketchy",
            TrustLevel::MarketplaceUnverified,
            &trust_policy,
            Arc::new(connection),
            tools,
        );
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fake_transport_reconnect_is_a_noop_ok() {
        let (client_transport, _server_transport) = FakeTransport::pair();
        assert!(client_transport.reconnect().await.is_ok());
    }

    #[tokio::test]
    async fn client_manager_connects_and_lists_tools() {
        let (client_transport, server_transport) = FakeTransport::pair();
        spawn_fake_server(server_transport, vec![serde_json::json!({"name": "ping"})]);

        // The manager's factory is `Fn`, so it must be able to hand out a
        // transport without owning it uniquely; a `Mutex<Option<_>>` lets a
        // single pre-built pair be moved out exactly once, panicking (inside
        // the test, not unsound) if a second connection were attempted.
        let slot = std::sync::Mutex::new(Some(client_transport));
        let mut manager = DefaultMcpClientManager::new(move |_cfg| {
            let transport = slot.lock().unwrap().take().expect("transport factory invoked more than once in this test");
            Box::new(transport) as Box<dyn McpTransport>
        });

        let config = McpServerConfig {
            name: "fake".into(),
            command: None,
            args: vec![],
            url: None,
            trust_level: TrustLevel::LocalDev,
        };
        manager.connect(config).await.unwrap();
        let tools = manager.list_tools("fake").await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "ping");
    }
}
