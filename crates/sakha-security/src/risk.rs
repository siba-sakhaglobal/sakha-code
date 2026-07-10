//! Risk classification for permission requests, used to decide default
//! sandbox profile and whether human approval is required.

use serde::{Deserialize, Serialize};

use crate::permissions::{PermissionKind, PermissionRequest};

/// Coarse risk tier assigned to an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    #[default]
    Low,
    Medium,
    High,
    Critical,
}

/// Classifies permission requests (and, for shell requests, the command text
/// itself) into a `RiskLevel`.
pub struct RiskClassifier {
    /// Command-name prefixes considered destructive/high-risk regardless of
    /// the declared `PermissionKind`.
    high_risk_commands: Vec<&'static str>,
    /// Command substrings considered critical (e.g. remote code execution,
    /// privilege escalation, filesystem wipes).
    critical_command_patterns: Vec<&'static str>,
}

impl RiskClassifier {
    pub fn new() -> Self {
        Self {
            high_risk_commands: vec![
                "rm", "rmdir", "del", "format", "mkfs", "dd", "shred", "chmod", "chown", "kill", "curl", "wget",
            ],
            critical_command_patterns: vec![
                "rm -rf /",
                "rm -rf ~",
                ":(){ :|:& };:",
                "sudo",
                "> /dev/sda",
                "mkfs.",
                "chmod -r 777 /",
                "chmod 777 /",
            ],
        }
    }

    /// Base classification driven by the permission kind alone.
    fn classify_kind(&self, kind: PermissionKind) -> RiskLevel {
        use PermissionKind::*;
        match kind {
            FileRead => RiskLevel::Low,
            FileWrite | ShellRunSafe | McpConnectorCall => RiskLevel::Medium,
            FileDelete | ShellRunArbitrary | NetworkAccess | PackageInstall | DestructiveFilesystem => {
                RiskLevel::High
            }
            GitRemotePush | BrowserAutomation | SecretRead | ExternalSideEffect => RiskLevel::Critical,
        }
    }

    /// Inspects a shell command string and returns an escalation, if any,
    /// beyond the kind-based baseline.
    pub fn classify_command(&self, command: &str) -> RiskLevel {
        let lower = command.to_lowercase();
        for pattern in &self.critical_command_patterns {
            if lower.contains(pattern) {
                return RiskLevel::Critical;
            }
        }
        let first_word = lower.split_whitespace().next().unwrap_or("");
        let base_name = first_word.rsplit(['/', '\\']).next().unwrap_or(first_word);
        if self.high_risk_commands.contains(&base_name) {
            return RiskLevel::High;
        }
        RiskLevel::Low
    }

    /// Classifies a full permission request, combining the kind baseline with
    /// command-text escalation for shell-shaped requests.
    pub fn classify(&self, request: &PermissionRequest) -> RiskLevel {
        let base = self.classify_kind(request.kind);
        let escalation = match request.kind {
            PermissionKind::ShellRunSafe | PermissionKind::ShellRunArbitrary => {
                Some(self.classify_command(&request.subject))
            }
            _ => None,
        };
        match escalation {
            Some(level) => base.max(level),
            None => base,
        }
    }
}

impl Default for RiskClassifier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_arbitrary_outranks_file_read() {
        let classifier = RiskClassifier::new();
        let read = PermissionRequest::new(PermissionKind::FileRead, "a.txt", "read");
        let shell = PermissionRequest::new(PermissionKind::ShellRunArbitrary, "rm -rf /tmp/x", "run");
        assert!(classifier.classify(&shell) > classifier.classify(&read));
    }

    #[test]
    fn destructive_command_text_escalates_to_critical() {
        let classifier = RiskClassifier::new();
        let req = PermissionRequest::new(PermissionKind::ShellRunSafe, "sudo rm -rf /", "run");
        assert_eq!(classifier.classify(&req), RiskLevel::Critical);
    }

    #[test]
    fn benign_command_stays_low_for_safe_kind() {
        let classifier = RiskClassifier::new();
        let req = PermissionRequest::new(PermissionKind::ShellRunSafe, "echo hello", "run");
        assert_eq!(classifier.classify(&req), RiskLevel::Medium);
    }
}
