//! Permission request/decision types. See spec `04-core-domain-model.md`
//! (`ToolCall.permission_decision`) and `modules/14-security-sandbox-permissions.md`.

use serde::{Deserialize, Serialize};

/// Categories of permission-gated actions. See spec "Permission Types" /
/// "Permission Categories".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    FileRead,
    FileWrite,
    FileDelete,
    ShellRunSafe,
    ShellRunArbitrary,
    NetworkAccess,
    GitRemotePush,
    BrowserAutomation,
    SecretRead,
    McpConnectorCall,
    ExternalSideEffect,
    PackageInstall,
    DestructiveFilesystem,
}

impl PermissionKind {
    /// Dotted, glob-matchable rule namespace for this kind, e.g. `file.read`,
    /// `shell.run_arbitrary`. Used by `PermissionPolicy` rule matching.
    pub fn rule_namespace(&self) -> &'static str {
        match self {
            PermissionKind::FileRead => "file.read",
            PermissionKind::FileWrite => "file.write",
            PermissionKind::FileDelete => "file.delete",
            PermissionKind::ShellRunSafe => "shell.run_safe",
            PermissionKind::ShellRunArbitrary => "shell.run_arbitrary",
            PermissionKind::NetworkAccess => "network.access",
            PermissionKind::GitRemotePush => "git.remote_push",
            PermissionKind::BrowserAutomation => "browser.automation",
            PermissionKind::SecretRead => "secret.read",
            PermissionKind::McpConnectorCall => "mcp.connector_call",
            PermissionKind::ExternalSideEffect => "external.side_effect",
            PermissionKind::PackageInstall => "package.install",
            PermissionKind::DestructiveFilesystem => "filesystem.destructive",
        }
    }
}

/// A request to perform a permission-gated action, raised by the tool
/// runtime before execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub kind: PermissionKind,
    pub subject: String,
    pub reason: String,
    pub tool_name: Option<String>,
}

impl PermissionRequest {
    pub fn new(kind: PermissionKind, subject: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            kind,
            subject: subject.into(),
            reason: reason.into(),
            tool_name: None,
        }
    }

    pub fn with_tool_name(mut self, tool_name: impl Into<String>) -> Self {
        self.tool_name = Some(tool_name.into());
        self
    }

    /// Rule string this request is matched against, e.g. `shell.run_arbitrary:rm -rf /`.
    /// The rule namespace alone (without `:subject`) also matches, so policies
    /// can allow/deny an entire kind or scope to specific subjects.
    pub fn rule_key(&self) -> String {
        format!("{}:{}", self.kind.rule_namespace(), self.subject)
    }
}

/// The outcome of evaluating a `PermissionRequest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum PermissionDecision {
    Allowed,
    Denied { reason: String },
    NeedsApproval { reason: String },
}

impl PermissionDecision {
    pub fn allowed() -> Self {
        PermissionDecision::Allowed
    }

    pub fn denied(reason: impl Into<String>) -> Self {
        PermissionDecision::Denied { reason: reason.into() }
    }

    pub fn needs_approval(reason: impl Into<String>) -> Self {
        PermissionDecision::NeedsApproval { reason: reason.into() }
    }

    pub fn is_allowed(&self) -> bool {
        matches!(self, PermissionDecision::Allowed)
    }

    pub fn is_denied(&self) -> bool {
        matches!(self, PermissionDecision::Denied { .. })
    }

    pub fn needs_human_approval(&self) -> bool {
        matches!(self, PermissionDecision::NeedsApproval { .. })
    }
}
