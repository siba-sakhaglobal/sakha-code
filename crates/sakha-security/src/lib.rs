//! sakha-security: permission policy, sandbox adapters, secret references, redaction, audit checks.
//!
//! Public API skeleton — see spec `modules/14-security-sandbox-permissions.md`
//! and `crates/crate-work-breakdown.md`.

pub mod audit;
pub mod glob;
pub mod permissions;
pub mod policy;
pub mod redaction;
pub mod risk;
pub mod sandbox;
pub mod secrets;

pub use audit::{SecurityAudit, SecurityAuditEntry};
pub use permissions::{PermissionDecision, PermissionKind, PermissionRequest};
pub use policy::{PermissionPolicy, PolicyLayer, PolicySource};
pub use redaction::Redactor;
pub use risk::{RiskClassifier, RiskLevel};
pub use sandbox::{
    path_is_within_root, NoopSandbox, PassthroughSandboxAdapter, SandboxAdapter, SandboxOutcome, SandboxProfile,
    SandboxRequest,
};
pub use secrets::{EnvSecretStore, InMemorySecretStore, SecretAccessRecord, SecretAction, SecretRef, SecretStore};

pub fn crate_name() -> &'static str {
    "sakha-security"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec test: "Higher-priority policy overrides lower."
    #[test]
    fn higher_priority_policy_overrides_lower() {
        let policy = PermissionPolicy::new()
            .with_layer(PolicyLayer {
                source: Some(PolicySource::BuiltinDefault),
                allow_rules: vec!["file.read".into()],
                ..Default::default()
            })
            .with_layer(PolicyLayer {
                source: Some(PolicySource::CliFlag),
                deny_rules: vec!["file.read".into()],
                ..Default::default()
            });
        let merged = policy.merged();
        assert!(merged.allow_rules.contains(&"file.read".to_string()));
        assert!(merged.deny_rules.contains(&"file.read".to_string()));

        let req = PermissionRequest::new(PermissionKind::FileRead, "a.txt", "read");
        assert!(policy.check(&req).unwrap().is_denied());
    }

    /// Spec test: "Secret redaction catches known secret."
    #[test]
    fn secret_redaction_catches_known_secret() {
        let redactor = Redactor::new();
        let text = "export OPENAI_API_KEY=sk-abcdefghijklmnopqrstuvwxyz0123456789";
        assert!(redactor.contains_secret(text));
        assert!(!redactor.redact(text).contains("sk-abcdefghijklmnopqrstuvwxyz0123456789"));
    }

    /// Spec test: "Denied permission blocks tool."
    #[test]
    fn denied_permission_blocks_tool() {
        let decision = PermissionDecision::denied("workspace policy");
        assert!(!decision.is_allowed());
    }

    /// Spec test: "Sandbox prevents outside-workspace write."
    #[test]
    fn sandbox_prevents_outside_workspace_write() {
        let sandbox = NoopSandbox::new();
        let req = SandboxRequest::new(SandboxProfile::WorkspaceWrite, "/workspace")
            .with_write_paths(vec!["/workspace/../../etc/passwd".into()]);
        assert!(sandbox.validate(&req).is_err());
    }

    /// Spec test: "Network disabled blocks fetch tool."
    #[test]
    fn network_disabled_blocks_fetch_tool() {
        let sandbox = NoopSandbox::new();
        let req = SandboxRequest::new(SandboxProfile::NetworkDisabled, "/workspace").with_network(true);
        assert!(sandbox.validate(&req).is_err());
    }

    /// Spec test: "Audit log records every decision."
    #[test]
    fn audit_log_records_every_decision() {
        let policy = PermissionPolicy::new();
        let mut audit = SecurityAudit::new();

        let requests = vec![
            PermissionRequest::new(PermissionKind::FileRead, "a.txt", "read"),
            PermissionRequest::new(PermissionKind::ShellRunArbitrary, "rm -rf /", "run"),
        ];
        for req in &requests {
            let decision = policy.check(req).unwrap();
            audit.record(req, &decision);
        }

        assert_eq!(audit.len(), requests.len());
    }

    #[test]
    fn risk_classifier_ranks_shell_arbitrary_above_file_read() {
        let classifier = RiskClassifier::new();
        let read = PermissionRequest::new(PermissionKind::FileRead, "a.txt", "read");
        let shell = PermissionRequest::new(PermissionKind::ShellRunArbitrary, "rm -rf", "run");
        assert!(classifier.classify(&shell) > classifier.classify(&read));
    }
}
