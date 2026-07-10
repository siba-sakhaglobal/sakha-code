//! Registry of configured MCP servers.

use serde::{Deserialize, Serialize};

use sakha_core::{SakhaError, SakhaResult};

use crate::trust::TrustLevel;

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
