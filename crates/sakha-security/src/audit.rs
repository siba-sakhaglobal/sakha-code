//! Security audit trail: records every permission decision so "every tool
//! call writes an audit event before execution" (spec `05-system-architecture.md`
//! "Failure Rules") can be verified end to end.

use serde::{Deserialize, Serialize};

use crate::permissions::{PermissionDecision, PermissionRequest};
use sakha_core::SakhaError;

/// One recorded permission evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityAuditEntry {
    pub subject: String,
    pub kind: crate::permissions::PermissionKind,
    pub decision: PermissionDecision,
    pub tool_name: Option<String>,
}

/// An in-memory, append-only audit log of permission decisions. Not a
/// persistence layer (that's `sakha-memory`/`sakha-observability`'s job) —
/// this is the in-process record a `PermissionPolicy` caller can consult or
/// hand off to an exporter.
#[derive(Debug, Default)]
pub struct SecurityAudit {
    entries: Vec<SecurityAuditEntry>,
}

impl SecurityAudit {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a decision for a request. Call this for every permission
    /// check, regardless of outcome.
    pub fn record(&mut self, request: &PermissionRequest, decision: &PermissionDecision) {
        self.entries.push(SecurityAuditEntry {
            subject: request.subject.clone(),
            kind: request.kind,
            decision: decision.clone(),
            tool_name: request.tool_name.clone(),
        });
    }

    pub fn entries(&self) -> &[SecurityAuditEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn denied_count(&self) -> usize {
        self.entries.iter().filter(|e| e.decision.is_denied()).count()
    }

    /// Serializes the full audit trail to a JSON array, per spec task #10
    /// "Implement security audit exports". Suitable for writing to a file or
    /// shipping to an external audit system (`sakha-observability`'s
    /// exporter, a SIEM, etc.).
    pub fn export_json(&self) -> Result<String, SakhaError> {
        serde_json::to_string_pretty(&self.entries)
            .map_err(|e| SakhaError::integrity("sakha-security", "failed to serialize audit export").with_cause(e))
    }

    /// Serializes the audit trail as JSON Lines (one `SecurityAuditEntry` per
    /// line), the common shape for append-only audit log files and streaming
    /// ingestion into external log/audit systems.
    pub fn export_jsonl(&self) -> Result<String, SakhaError> {
        let mut out = String::new();
        for entry in &self.entries {
            let line = serde_json::to_string(entry)
                .map_err(|e| SakhaError::integrity("sakha-security", "failed to serialize audit entry").with_cause(e))?;
            out.push_str(&line);
            out.push('\n');
        }
        Ok(out)
    }

    /// Writes the audit trail to `path` as JSON (pretty-printed array), per
    /// spec "Implement security audit exports" -> "export audit to files".
    pub fn export_to_file(&self, path: impl AsRef<std::path::Path>) -> Result<(), SakhaError> {
        let json = self.export_json()?;
        std::fs::write(path, json)
            .map_err(|e| SakhaError::integrity("sakha-security", "failed to write audit export file").with_cause(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::PermissionKind;

    #[test]
    fn records_every_decision() {
        let mut audit = SecurityAudit::new();
        let req1 = PermissionRequest::new(PermissionKind::FileRead, "a.txt", "read");
        let req2 = PermissionRequest::new(PermissionKind::ShellRunArbitrary, "rm -rf /", "run");

        audit.record(&req1, &PermissionDecision::allowed());
        audit.record(&req2, &PermissionDecision::denied("blocked"));

        assert_eq!(audit.len(), 2);
        assert_eq!(audit.denied_count(), 1);
    }

    #[test]
    fn export_json_round_trips_all_entries() {
        let mut audit = SecurityAudit::new();
        let req = PermissionRequest::new(PermissionKind::FileRead, "a.txt", "read");
        audit.record(&req, &PermissionDecision::allowed());

        let json = audit.export_json().unwrap();
        let parsed: Vec<SecurityAuditEntry> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].subject, "a.txt");
    }

    #[test]
    fn export_jsonl_writes_one_entry_per_line() {
        let mut audit = SecurityAudit::new();
        audit.record(&PermissionRequest::new(PermissionKind::FileRead, "a.txt", "read"), &PermissionDecision::allowed());
        audit.record(
            &PermissionRequest::new(PermissionKind::ShellRunArbitrary, "rm -rf /", "run"),
            &PermissionDecision::denied("blocked"),
        );

        let jsonl = audit.export_jsonl().unwrap();
        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in &lines {
            let _: SecurityAuditEntry = serde_json::from_str(line).unwrap();
        }
    }

    #[test]
    fn export_to_file_writes_readable_json() {
        let mut audit = SecurityAudit::new();
        audit.record(&PermissionRequest::new(PermissionKind::FileRead, "a.txt", "read"), &PermissionDecision::allowed());

        let dir = std::env::temp_dir();
        let path = dir.join(format!("sakha-audit-export-test-{}.json", std::process::id()));
        audit.export_to_file(&path).unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        let parsed: Vec<SecurityAuditEntry> = serde_json::from_str(&contents).unwrap();
        assert_eq!(parsed.len(), 1);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn export_json_on_empty_audit_is_empty_array() {
        let audit = SecurityAudit::new();
        assert_eq!(audit.export_json().unwrap(), "[]");
    }
}
