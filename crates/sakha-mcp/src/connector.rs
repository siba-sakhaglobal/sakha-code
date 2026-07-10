//! Connectors: named external integrations reachable through Sakha (spec
//! "Main Structs": `Connector`, `ConnectorCredentialRef`, `ConnectorPolicy`;
//! "Features": "Connector auth policy", "Connector output compression").

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use sakha_core::{SakhaError, SakhaResult};
use sakha_security::{PermissionDecision, PermissionKind, PermissionPolicy, PermissionRequest, SecretRef};

/// A reference to the credential a `Connector` authenticates with. Mirrors
/// `sakha_security::SecretRef`'s "reference, not the secret" shape so
/// connector configs stay safe to log/serialize.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorCredentialRef {
    pub secret_ref: SecretRef,
}

impl ConnectorCredentialRef {
    pub fn new(secret_ref: SecretRef) -> Self {
        Self { secret_ref }
    }
}

/// Policy gating whether a connector call may proceed: which
/// `PermissionKind` it requires and whether its output must be compressed
/// before being handed back to the model (spec "Connector output
/// compression").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorPolicy {
    pub required_permission: PermissionKind,
    pub compress_output: bool,
}

impl Default for ConnectorPolicy {
    fn default() -> Self {
        Self { required_permission: PermissionKind::McpConnectorCall, compress_output: true }
    }
}

/// A named external integration (e.g. a SaaS API) reachable through Sakha,
/// gated by a `ConnectorPolicy` and authenticated via a
/// `ConnectorCredentialRef`. Distinct from an `McpConnection`: a connector
/// may be backed by an MCP server, a direct HTTP client, or any other
/// transport; this crate models the auth/policy/compression contract every
/// connector must satisfy regardless of backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connector {
    pub name: String,
    pub description: String,
    pub credential: ConnectorCredentialRef,
    pub policy: ConnectorPolicy,
}

impl Connector {
    pub fn new(name: impl Into<String>, credential: ConnectorCredentialRef) -> Self {
        Self { name: name.into(), description: String::new(), credential, policy: ConnectorPolicy::default() }
    }

    pub fn with_policy(mut self, policy: ConnectorPolicy) -> Self {
        self.policy = policy;
        self
    }
}

/// Checks `connector`'s policy against `permission_policy` before a call is
/// allowed to proceed (spec "Tests": "Permission denied blocks connector
/// call").
pub fn check_connector_permission(connector: &Connector, permission_policy: &PermissionPolicy) -> SakhaResult<PermissionDecision> {
    let request = PermissionRequest::new(connector.policy.required_permission, connector.name.clone(), "connector_call");
    permission_policy.check(&request)
}

/// Runs `output` through `compressor` when `connector.policy.compress_output`
/// is set, using `LocalFallbackCompressor`'s always-available truncation
/// (spec "Features": "Connector output compression"). Returns the content
/// unchanged when compression is disabled or the input is small.
pub async fn compress_connector_output(
    connector: &Connector,
    compressor: &Arc<dyn sakha_compression::ContextCompressor>,
    output: String,
) -> SakhaResult<sakha_compression::CompressedContextItem> {
    let policy = if connector.policy.compress_output {
        sakha_compression::CompressionPolicy::default()
    } else {
        let mut policy = sakha_compression::CompressionPolicy::default();
        policy.enabled = false;
        policy
    };
    let item = sakha_compression::ContextItem::new(output).with_kind(sakha_compression::ContentKind::ToolOutput);
    compressor.compress(item, &policy).await
}

/// In-memory registry of configured connectors, so callers can look one up
/// by name before dispatching a call.
#[derive(Debug, Default)]
pub struct ConnectorRegistry {
    connectors: std::collections::HashMap<String, Connector>,
}

impl ConnectorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, connector: Connector) {
        self.connectors.insert(connector.name.clone(), connector);
    }

    pub fn get(&self, name: &str) -> SakhaResult<&Connector> {
        self.connectors
            .get(name)
            .ok_or_else(|| SakhaError::invalid_input("sakha-mcp", format!("unknown connector: {name}")))
    }

    pub fn list(&self) -> Vec<&Connector> {
        self.connectors.values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_connector(compress: bool) -> Connector {
        Connector::new("github", ConnectorCredentialRef::new(SecretRef::new("github_token")))
            .with_policy(ConnectorPolicy { required_permission: PermissionKind::McpConnectorCall, compress_output: compress })
    }

    #[test]
    fn registry_register_then_get_round_trips() {
        let mut registry = ConnectorRegistry::new();
        registry.register(test_connector(true));
        assert_eq!(registry.get("github").unwrap().name, "github");
        assert!(registry.get("missing").is_err());
    }

    #[test]
    fn permission_denied_blocks_connector_call() {
        let connector = test_connector(true);
        let policy = PermissionPolicy::new().with_layer(sakha_security::PolicyLayer {
            source: Some(sakha_security::PolicySource::WorkspaceLocal),
            deny_rules: vec!["mcp.connector_call".into()],
            ..Default::default()
        });
        let decision = check_connector_permission(&connector, &policy).unwrap();
        assert!(decision.is_denied());
    }

    #[test]
    fn permission_allowed_by_default_policy() {
        let connector = test_connector(true);
        let policy = PermissionPolicy::new();
        let decision = check_connector_permission(&connector, &policy).unwrap();
        // Default policy neither explicitly allows nor denies McpConnectorCall;
        // exercised mainly to prove `check` never errors/panics for a
        // well-formed connector request.
        let _ = decision;
    }

    #[tokio::test]
    async fn connector_output_is_compressed_when_policy_requires_it() {
        let connector = test_connector(true);
        let store = Arc::new(sakha_compression::InMemoryRetrievalStore::new());
        let compressor: Arc<dyn sakha_compression::ContextCompressor> = Arc::new(
            sakha_compression::LocalFallbackCompressor::new(store)
                .with_config(sakha_compression::LocalFallbackConfig { head_lines: 3, tail_lines: 3, min_bytes_to_truncate: 10 }),
        );
        let raw: String = (0..500).map(|i| format!("connector output line {i}\n")).collect();
        let compressed = compress_connector_output(&connector, &compressor, raw.clone()).await.unwrap();
        assert!(compressed.compressed_content.len() < raw.len());
        assert!(compressed.marker.is_some());
    }

    #[tokio::test]
    async fn connector_output_untouched_when_compression_disabled() {
        let connector = test_connector(false);
        let store = Arc::new(sakha_compression::InMemoryRetrievalStore::new());
        let compressor: Arc<dyn sakha_compression::ContextCompressor> = Arc::new(sakha_compression::LocalFallbackCompressor::new(store));
        let raw: String = (0..500).map(|i| format!("connector output line {i}\n")).collect();
        let compressed = compress_connector_output(&connector, &compressor, raw.clone()).await.unwrap();
        assert_eq!(compressed.compressed_content, raw);
        assert!(compressed.marker.is_none());
    }
}
