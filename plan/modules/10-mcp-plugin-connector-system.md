# Module 10: MCP, Plugins, and Connectors

## Responsibility

Connect Sakha to external tools and expose Sakha capabilities to other MCP clients.

## Rust Crate

`crates/sakha-mcp`

## Main Structs

- `McpClientManager`
- `McpServerManager`
- `McpConnection`
- `McpToolProxy`
- `McpResourceProxy`
- `PluginManifest`
- `PluginRegistry`
- `Connector`
- `ConnectorCredentialRef`
- `ConnectorPolicy`

## Features

- MCP stdio client.
- MCP HTTP/SSE client.
- MCP server exposing Sakha tools/resources/prompts.
- Tool discovery.
- Resource discovery.
- Prompt discovery.
- Connector auth policy.
- Plugin manifest validation.
- Plugin install/remove/update.
- Plugin trust levels.
- Connector output compression.

## Plugin Manifest Fields

- `id`
- `name`
- `version`
- `description`
- `publisher`
- `entrypoint`
- `permissions`
- `mcp_servers`
- `skills`
- `tools`
- `config_schema`
- `signature`

## Trust Model

- Local dev plugin.
- Workspace plugin.
- Organization-approved plugin.
- Marketplace plugin.
- Blocked plugin.

## Implementation Tasks

1. Implement MCP transport abstraction.
2. Implement MCP client connection lifecycle.
3. Implement tool proxy.
4. Implement resource proxy.
5. Implement Sakha MCP server.
6. Define plugin manifest schema.
7. Implement plugin registry.
8. Implement permission policy integration.
9. Implement connector credential references.
10. Implement plugin audit logs.

## Tests

- Connect to stdio MCP server.
- Proxy MCP tool call.
- Expose Sakha resource over MCP.
- Reject plugin with invalid manifest.
- Permission denied blocks connector call.
- Connector output is compressed.

