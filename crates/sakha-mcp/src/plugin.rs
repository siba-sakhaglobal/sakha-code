//! Plugin manifest schema, validation, and registry. See spec
//! `modules/10-mcp-plugin-connector-system.md` "Plugin Manifest Fields",
//! "Trust Model", and "Implementation Tasks": "Define plugin manifest
//! schema", "Implement plugin registry".

use serde::{Deserialize, Serialize};

use sakha_core::{SakhaError, SakhaResult};

use crate::server_registry::McpServerConfig;
use crate::trust::TrustLevel;

/// A plugin's declared manifest. Field set matches spec "Plugin Manifest
/// Fields" exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub publisher: String,
    pub entrypoint: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub config_schema: Option<serde_json::Value>,
    #[serde(default)]
    pub signature: Option<String>,
}

impl PluginManifest {
    /// Validates required fields are non-empty and the manifest is otherwise
    /// well-formed, per spec "Implementation Tasks": "Define plugin manifest
    /// schema" and "Tests": "Reject plugin with invalid manifest". This is
    /// deliberately conservative (non-empty `id`/`name`/`version`/`entrypoint`)
    /// rather than a full semver/URI validator, since the spec does not
    /// mandate a specific format for those fields beyond "present".
    pub fn validate(&self) -> SakhaResult<()> {
        let mut problems = Vec::new();
        if self.id.trim().is_empty() {
            problems.push("id must not be empty");
        }
        if self.name.trim().is_empty() {
            problems.push("name must not be empty");
        }
        if self.version.trim().is_empty() {
            problems.push("version must not be empty");
        }
        if self.entrypoint.trim().is_empty() {
            problems.push("entrypoint must not be empty");
        }
        if self.publisher.trim().is_empty() {
            problems.push("publisher must not be empty");
        }
        if !problems.is_empty() {
            return Err(SakhaError::invalid_input(
                "sakha-mcp",
                format!("invalid plugin manifest '{}': {}", self.id, problems.join(", ")),
            ));
        }
        Ok(())
    }

    pub fn from_json(value: &str) -> SakhaResult<Self> {
        let manifest: PluginManifest =
            serde_json::from_str(value).map_err(|e| SakhaError::invalid_input("sakha-mcp", format!("invalid plugin manifest JSON: {e}")))?;
        manifest.validate()?;
        Ok(manifest)
    }
}

/// A registered plugin: its manifest plus the trust level it was installed
/// at (spec "Trust Model").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredPlugin {
    pub manifest: PluginManifest,
    pub trust_level: TrustLevel,
    pub enabled: bool,
}

/// One entry in a plugin's audit trail (spec "Implementation Tasks":
/// "Implement plugin audit logs").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginAuditEntry {
    pub plugin_id: String,
    pub action: String,
    pub timestamp_unix: u64,
}

/// In-memory registry of installed plugins: validates manifests on install,
/// enforces trust-level gating, and keeps an audit trail of install/remove/
/// update/enable/disable actions.
#[derive(Debug, Default)]
pub struct PluginRegistry {
    plugins: std::collections::HashMap<String, RegisteredPlugin>,
    audit_log: Vec<PluginAuditEntry>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn audit(&mut self, plugin_id: &str, action: &str) {
        self.audit_log.push(PluginAuditEntry {
            plugin_id: plugin_id.to_string(),
            action: action.to_string(),
            timestamp_unix: sakha_core::time::now_utc().timestamp().clamp(0, i64::MAX) as u64,
        });
    }

    /// Installs `manifest` at `trust_level`. Rejects an invalid manifest
    /// (spec "Tests": "Reject plugin with invalid manifest") and a
    /// `TrustLevel::Blocked` install outright.
    pub fn install(&mut self, manifest: PluginManifest, trust_level: TrustLevel) -> SakhaResult<()> {
        manifest.validate()?;
        if trust_level == TrustLevel::Blocked {
            return Err(SakhaError::permission("sakha-mcp", format!("plugin '{}' is blocked and cannot be installed", manifest.id)));
        }
        let id = manifest.id.clone();
        self.plugins.insert(id.clone(), RegisteredPlugin { manifest, trust_level, enabled: true });
        self.audit(&id, "install");
        Ok(())
    }

    pub fn remove(&mut self, plugin_id: &str) -> SakhaResult<()> {
        self.plugins
            .remove(plugin_id)
            .ok_or_else(|| SakhaError::invalid_input("sakha-mcp", format!("unknown plugin: {plugin_id}")))?;
        self.audit(plugin_id, "remove");
        Ok(())
    }

    /// Replaces an installed plugin's manifest in place (same `id`), e.g. a
    /// version bump.
    pub fn update(&mut self, manifest: PluginManifest) -> SakhaResult<()> {
        manifest.validate()?;
        let id = manifest.id.clone();
        let existing = self
            .plugins
            .get_mut(&id)
            .ok_or_else(|| SakhaError::invalid_input("sakha-mcp", format!("unknown plugin: {id}")))?;
        existing.manifest = manifest;
        self.audit(&id, "update");
        Ok(())
    }

    pub fn set_enabled(&mut self, plugin_id: &str, enabled: bool) -> SakhaResult<()> {
        let plugin = self
            .plugins
            .get_mut(plugin_id)
            .ok_or_else(|| SakhaError::invalid_input("sakha-mcp", format!("unknown plugin: {plugin_id}")))?;
        plugin.enabled = enabled;
        self.audit(plugin_id, if enabled { "enable" } else { "disable" });
        Ok(())
    }

    pub fn get(&self, plugin_id: &str) -> Option<&RegisteredPlugin> {
        self.plugins.get(plugin_id)
    }

    pub fn list(&self) -> Vec<&RegisteredPlugin> {
        self.plugins.values().collect()
    }

    pub fn audit_log(&self) -> &[PluginAuditEntry] {
        &self.audit_log
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(id: &str) -> PluginManifest {
        PluginManifest {
            id: id.to_string(),
            name: "Test Plugin".into(),
            version: "0.1.0".into(),
            description: "a test plugin".into(),
            publisher: "sakha".into(),
            entrypoint: "index.js".into(),
            permissions: vec![],
            mcp_servers: vec![],
            skills: vec![],
            tools: vec![],
            config_schema: None,
            signature: None,
        }
    }

    #[test]
    fn valid_manifest_passes_validation() {
        assert!(manifest("plugin.a").validate().is_ok());
    }

    #[test]
    fn manifest_with_empty_id_is_rejected() {
        let mut m = manifest("plugin.b");
        m.id = String::new();
        assert!(m.validate().is_err());
    }

    #[test]
    fn registry_install_then_get_round_trips() {
        let mut registry = PluginRegistry::new();
        registry.install(manifest("plugin.c"), TrustLevel::Workspace).unwrap();
        let found = registry.get("plugin.c").unwrap();
        assert_eq!(found.trust_level, TrustLevel::Workspace);
        assert!(found.enabled);
    }

    #[test]
    fn registry_rejects_invalid_manifest_install() {
        let mut registry = PluginRegistry::new();
        let mut m = manifest("plugin.d");
        m.entrypoint = String::new();
        assert!(registry.install(m, TrustLevel::Workspace).is_err());
        assert!(registry.get("plugin.d").is_none());
    }

    #[test]
    fn registry_rejects_blocked_trust_level() {
        let mut registry = PluginRegistry::new();
        assert!(registry.install(manifest("plugin.e"), TrustLevel::Blocked).is_err());
    }

    #[test]
    fn registry_records_audit_log_entries() {
        let mut registry = PluginRegistry::new();
        registry.install(manifest("plugin.f"), TrustLevel::LocalDev).unwrap();
        registry.set_enabled("plugin.f", false).unwrap();
        registry.remove("plugin.f").unwrap();
        let actions: Vec<&str> = registry.audit_log().iter().map(|e| e.action.as_str()).collect();
        assert_eq!(actions, vec!["install", "disable", "remove"]);
    }
}
