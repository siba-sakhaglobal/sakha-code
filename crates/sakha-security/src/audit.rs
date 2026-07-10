//! Security audit trail: records every permission decision so "every tool
//! call writes an audit event before execution" (spec `05-system-architecture.md`
//! "Failure Rules") can be verified end to end.

use serde::{Deserialize, Serialize};

use crate::permissions::{PermissionDecision, PermissionRequest};

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
}
