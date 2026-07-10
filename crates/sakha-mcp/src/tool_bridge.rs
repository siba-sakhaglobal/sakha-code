//! `McpToolBridge`: proxies discovered MCP tools into `sakha-tools`
//! `Tool`/`ToolSpec`s, namespaced by server name (e.g. `mcp.<server>.<tool>`).

use std::sync::Arc;

use async_trait::async_trait;

use sakha_core::{SakhaError, SakhaResult};
use sakha_security::PermissionKind;
use sakha_tools::{
    IdempotencyPolicy, Tool, ToolContext, ToolInputSchema, ToolName, ToolOutputSchema,
    ToolPermissionSpec, ToolPlan, ToolResult, ToolSpec, ToolSummary, ValidatedInput,
};

use crate::client::{McpConnection, McpToolDescriptor};
use crate::trust::{TrustLevel, TrustPolicy};

/// Builds the namespaced tool name Sakha uses for a given MCP server/tool
/// pair: `mcp.<server>.<tool>`.
pub fn namespaced_tool_name(server_name: &str, tool_name: &str) -> String {
    format!("mcp.{server_name}.{tool_name}")
}

/// Converts MCP tool descriptors from a given server into `sakha-tools`
/// `ToolSpec`s namespaced by server name.
pub struct McpToolBridge;

impl McpToolBridge {
    pub fn new() -> Self {
        Self
    }

    pub fn bridge(&self, server_name: &str, tools: Vec<McpToolDescriptor>) -> SakhaResult<Vec<ToolSpec>> {
        Ok(tools.into_iter().map(|t| spec_for(server_name, &t)).collect())
    }

    /// Bridges discovered MCP tools into live, executable `Tool` objects
    /// backed by `connection`, gated by `trust_policy` against
    /// `server_trust`. Returns an error (no tools) if the server's trust
    /// level does not satisfy the policy — callers should treat this as
    /// "server blocked", not a partial success.
    pub fn bridge_executable(
        &self,
        server_name: &str,
        server_trust: TrustLevel,
        trust_policy: &TrustPolicy,
        connection: Arc<McpConnection>,
        tools: Vec<McpToolDescriptor>,
    ) -> SakhaResult<Vec<Arc<dyn Tool>>> {
        if !trust_policy.allows(server_trust) {
            return Err(SakhaError::permission(
                "sakha-mcp",
                format!("MCP server '{server_name}' trust level {server_trust:?} is below policy minimum"),
            ));
        }
        Ok(tools
            .into_iter()
            .map(|t| {
                let spec = spec_for(server_name, &t);
                Arc::new(McpProxyTool { server_name: server_name.to_string(), mcp_tool_name: t.name, spec, connection: connection.clone() })
                    as Arc<dyn Tool>
            })
            .collect())
    }
}

impl Default for McpToolBridge {
    fn default() -> Self {
        Self::new()
    }
}

fn spec_for(server_name: &str, t: &McpToolDescriptor) -> ToolSpec {
    ToolSpec {
        name: ToolName::new(namespaced_tool_name(server_name, &t.name)),
        description: t.description.clone(),
        input_schema: ToolInputSchema(t.input_schema.clone()),
        output_schema: ToolOutputSchema(serde_json::json!({"type": "object"})),
        permission_spec: ToolPermissionSpec { required: vec![PermissionKind::McpConnectorCall] },
        idempotency_policy: IdempotencyPolicy::NotIdempotent,
    }
}

/// A `sakha-tools::Tool` implementation that proxies calls to a single MCP
/// tool over a live `McpConnection`.
pub struct McpProxyTool {
    pub server_name: String,
    pub mcp_tool_name: String,
    pub spec: ToolSpec,
    pub connection: Arc<McpConnection>,
}

#[async_trait]
impl Tool for McpProxyTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    fn validate(&self, input: serde_json::Value) -> SakhaResult<ValidatedInput> {
        // MCP input schemas are validated server-side on `tools/call`; Sakha
        // accepts any JSON object here and lets the server reject bad input.
        Ok(ValidatedInput(input))
    }

    async fn plan(&self, _input: &ValidatedInput, _context: &ToolContext) -> SakhaResult<ToolPlan> {
        Ok(ToolPlan {
            summary: format!("Call MCP tool '{}' on server '{}'", self.mcp_tool_name, self.server_name),
            affected_paths: Vec::new(),
            is_destructive: false,
        })
    }

    async fn execute(&self, input: &ValidatedInput, _context: &ToolContext) -> SakhaResult<ToolResult> {
        let output = self.connection.call_tool(&self.mcp_tool_name, input.0.clone()).await?;
        Ok(ToolResult::success(output))
    }

    fn summarize(&self, result: &ToolResult) -> ToolSummary {
        let text = match result.status {
            sakha_tools::ToolCallStatus::Succeeded => {
                format!("mcp.{}.{} succeeded", self.server_name, self.mcp_tool_name)
            }
            _ => format!(
                "mcp.{}.{} failed: {}",
                self.server_name,
                self.mcp_tool_name,
                result.error_message.clone().unwrap_or_default()
            ),
        };
        ToolSummary { text, truncated: false }
    }
}
