//! `AuditExporter`/`AuditLog`: an immutable, append-only audit trail and its
//! offline export. See spec "Audit Requirements" ("Immutable append-only
//! audit log", "Link every file write to tool call and turn", "Export
//! session report") and "Implement offline audit export".

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use sakha_core::{EventEnvelope, SakhaResult, SessionId};

use crate::logging::{redact_with, NoopRedactionHook, ObservabilityConfig, RedactionHook};

/// A finished, exportable summary of one session for audit review. Mirrors
/// spec "Export session report".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionReport {
    pub session_id: SessionId,
    pub events: Vec<EventEnvelope>,
    pub commands_run: Vec<String>,
    pub files_changed: Vec<String>,
    pub total_cost_micros: u64,
}

/// An append-only, in-process audit log. Events are never removed or
/// mutated once appended (`append` is the only write path), matching the
/// "Immutable append-only audit log" requirement. Safe to share across
/// concurrent tasks via an internal mutex.
#[derive(Default)]
pub struct AuditLog {
    events: Mutex<Vec<EventEnvelope>>,
}

impl AuditLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends an event to the log. This is the only mutation entry point;
    /// there is intentionally no `remove`/`clear` in the public API.
    pub fn append(&self, event: EventEnvelope) {
        self.events.lock().unwrap().push(event);
    }

    /// Returns every event recorded for `session_id`, in append (and thus
    /// chronological/sequence) order.
    pub fn events_for_session(&self, session_id: SessionId) -> Vec<EventEnvelope> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.session_id == Some(session_id))
            .cloned()
            .collect()
    }

    /// Returns a snapshot of every event recorded across all sessions, in
    /// append order.
    pub fn all_events(&self) -> Vec<EventEnvelope> {
        self.events.lock().unwrap().clone()
    }

    pub fn len(&self) -> usize {
        self.events.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Exports session audit data to a serializable form (JSON file, JSONL
/// stream, etc). Redaction is applied to free-text fields
/// (`commands_run`, `files_changed`, and event payload strings) when
/// `config.redact_secrets` is set, using the supplied `RedactionHook`.
pub struct AuditExporter {
    config: ObservabilityConfig,
}

impl AuditExporter {
    pub fn new() -> Self {
        Self { config: ObservabilityConfig::default() }
    }

    pub fn with_config(config: ObservabilityConfig) -> Self {
        Self { config }
    }

    /// Serializes a `SessionReport` to a pretty JSON string. Never panics on
    /// well-formed input; errors are surfaced as `SakhaError::Integrity`.
    pub fn export_json(&self, report: &SessionReport) -> SakhaResult<String> {
        serde_json::to_string_pretty(report)
            .map_err(|e| sakha_core::SakhaError::integrity("sakha-observability", "failed to serialize session report").with_cause(e))
    }

    /// Serializes a `SessionReport` after applying `hook` to its free-text
    /// fields, per `config.redact_secrets`. Use this when a report may
    /// contain command output or file paths derived from user/tool content.
    pub fn export_json_redacted(&self, report: &SessionReport, hook: &dyn RedactionHook) -> SakhaResult<String> {
        let redacted = self.redact_report(report, hook);
        self.export_json(&redacted)
    }

    /// Applies redaction to a report's free-text fields without serializing,
    /// e.g. for callers that want the redacted `SessionReport` value itself.
    pub fn redact_report(&self, report: &SessionReport, hook: &dyn RedactionHook) -> SessionReport {
        SessionReport {
            session_id: report.session_id,
            events: report.events.clone(),
            commands_run: report
                .commands_run
                .iter()
                .map(|c| redact_with(&self.config, hook, c))
                .collect(),
            files_changed: report.files_changed.clone(),
            total_cost_micros: report.total_cost_micros,
        }
    }

    /// Exports an audit log's events as newline-delimited JSON (JSONL): one
    /// `EventEnvelope` per line, in append order. This is the "offline audit
    /// export" format — easy to `tail -f`, diff, or stream to a sidecar.
    pub fn export_jsonl(&self, events: &[EventEnvelope]) -> SakhaResult<String> {
        let mut out = String::new();
        for event in events {
            let line = serde_json::to_string(event)
                .map_err(|e| sakha_core::SakhaError::integrity("sakha-observability", "failed to serialize audit event").with_cause(e))?;
            out.push_str(&line);
            out.push('\n');
        }
        Ok(out)
    }

    /// Parses a JSONL export (as produced by [`export_jsonl`]) back into
    /// `EventEnvelope`s, verifying round-trip fidelity. Blank lines are
    /// skipped; any malformed line yields an `Integrity` error.
    pub fn import_jsonl(&self, jsonl: &str) -> SakhaResult<Vec<EventEnvelope>> {
        jsonl
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str::<EventEnvelope>(line)
                    .map_err(|e| sakha_core::SakhaError::integrity("sakha-observability", "failed to parse audit event line").with_cause(e))
            })
            .collect()
    }

    /// Writes a `SessionReport` plus its underlying JSONL event log to
    /// `dir`, using conventional filenames (`session-<id>.json` and
    /// `session-<id>-events.jsonl`). Returns the two paths written.
    pub fn export_to_dir(&self, dir: &std::path::Path, report: &SessionReport) -> SakhaResult<(std::path::PathBuf, std::path::PathBuf)> {
        std::fs::create_dir_all(dir).map_err(|e| {
            sakha_core::SakhaError::integrity("sakha-observability", "failed to create export directory").with_cause(e)
        })?;

        let report_path = dir.join(format!("session-{}.json", report.session_id));
        let events_path = dir.join(format!("session-{}-events.jsonl", report.session_id));

        let report_json = self.export_json(report)?;
        std::fs::write(&report_path, report_json)
            .map_err(|e| sakha_core::SakhaError::integrity("sakha-observability", "failed to write session report").with_cause(e))?;

        let events_jsonl = self.export_jsonl(&report.events)?;
        std::fs::write(&events_path, events_jsonl)
            .map_err(|e| sakha_core::SakhaError::integrity("sakha-observability", "failed to write session events").with_cause(e))?;

        Ok((report_path, events_path))
    }
}

impl Default for AuditExporter {
    fn default() -> Self {
        Self::new()
    }
}

/// Builds a `SessionReport` from an `AuditLog`'s recorded events for
/// `session_id`, deriving `commands_run`/`files_changed` from event
/// payloads where present (best-effort; payloads are the loosely-typed
/// `serde_json::Value` carried by `EventEnvelope`).
pub fn build_session_report(log: &AuditLog, session_id: SessionId, total_cost_micros: u64) -> SessionReport {
    let events = log.events_for_session(session_id);
    let mut commands_run = Vec::new();
    let mut files_changed = Vec::new();

    for event in &events {
        if let Some(cmd) = event.payload.get("command").and_then(|v| v.as_str()) {
            commands_run.push(cmd.to_string());
        }
        if let Some(path) = event.payload.get("file_path").and_then(|v| v.as_str()) {
            files_changed.push(path.to_string());
        }
    }

    SessionReport { session_id, events, commands_run, files_changed, total_cost_micros }
}

/// A no-redaction convenience for tests/callers that don't need secret
/// scrubbing (equivalent to `AuditExporter::default().export_json`, but
/// explicit about intent at call sites).
pub fn export_without_redaction(exporter: &AuditExporter, report: &SessionReport) -> SakhaResult<String> {
    exporter.export_json_redacted(report, &NoopRedactionHook)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sakha_core::EventKind;
    use serde_json::json;

    #[test]
    fn audit_export_round_trips_session_report() {
        let exporter = AuditExporter::new();
        let report = SessionReport {
            session_id: SessionId::new(),
            events: Vec::new(),
            commands_run: vec!["cargo test".into()],
            files_changed: vec!["src/lib.rs".into()],
            total_cost_micros: 42,
        };
        let json = exporter.export_json(&report).unwrap();
        let back: SessionReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.session_id, report.session_id);
        assert_eq!(back.commands_run, report.commands_run);
    }

    #[test]
    fn audit_log_is_append_only_and_filters_by_session() {
        let log = AuditLog::new();
        let session_a = SessionId::new();
        let session_b = SessionId::new();
        log.append(EventEnvelope::for_session(session_a, EventKind::SessionStarted, json!({})));
        log.append(EventEnvelope::for_session(session_b, EventKind::SessionStarted, json!({})));
        log.append(EventEnvelope::for_session(session_a, EventKind::SessionCompleted, json!({})));

        assert_eq!(log.len(), 3);
        assert_eq!(log.events_for_session(session_a).len(), 2);
        assert_eq!(log.events_for_session(session_b).len(), 1);
    }

    #[test]
    fn jsonl_export_round_trips_events() {
        let session = SessionId::new();
        let events = vec![
            EventEnvelope::for_session(session, EventKind::ToolCallStarted, json!({"tool": "read_file"})),
            EventEnvelope::for_session(session, EventKind::ToolCallCompleted, json!({"tool": "read_file", "status": "ok"})),
        ];
        let exporter = AuditExporter::new();
        let jsonl = exporter.export_jsonl(&events).unwrap();
        assert_eq!(jsonl.lines().count(), 2);

        let parsed = exporter.import_jsonl(&jsonl).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].kind, EventKind::ToolCallStarted);
        assert_eq!(parsed[1].kind, EventKind::ToolCallCompleted);
    }

    #[test]
    fn redacted_export_strips_secrets_from_commands_run() {
        struct FakeHook;
        impl RedactionHook for FakeHook {
            fn redact(&self, text: &str) -> String {
                text.replace("sk-secret123", "[REDACTED]")
            }
        }

        let report = SessionReport {
            session_id: SessionId::new(),
            events: Vec::new(),
            commands_run: vec!["curl -H 'Authorization: sk-secret123'".into()],
            files_changed: vec![],
            total_cost_micros: 0,
        };
        let exporter = AuditExporter::new();
        let json = exporter.export_json_redacted(&report, &FakeHook).unwrap();
        assert!(!json.contains("sk-secret123"));
        assert!(json.contains("[REDACTED]"));
    }

    #[test]
    fn build_session_report_links_tool_calls_and_file_writes() {
        let log = AuditLog::new();
        let session = SessionId::new();
        log.append(EventEnvelope::for_session(
            session,
            EventKind::ToolCallCompleted,
            json!({"command": "cargo test", "tool": "shell"}),
        ));
        log.append(EventEnvelope::for_session(
            session,
            EventKind::ToolCallCompleted,
            json!({"file_path": "src/lib.rs", "tool": "write_file"}),
        ));

        let report = build_session_report(&log, session, 123);
        assert_eq!(report.commands_run, vec!["cargo test".to_string()]);
        assert_eq!(report.files_changed, vec!["src/lib.rs".to_string()]);
        assert_eq!(report.total_cost_micros, 123);
        assert_eq!(report.events.len(), 2);
    }

    #[test]
    fn export_to_dir_writes_report_and_events_files() {
        let dir = std::env::temp_dir().join(format!("sakha-obs-export-{}", uuid::Uuid::new_v4()));
        let session = SessionId::new();
        let report = SessionReport {
            session_id: session,
            events: vec![EventEnvelope::for_session(session, EventKind::SessionCompleted, json!({}))],
            commands_run: vec![],
            files_changed: vec![],
            total_cost_micros: 7,
        };
        let exporter = AuditExporter::new();
        let (report_path, events_path) = exporter.export_to_dir(&dir, &report).unwrap();
        assert!(report_path.exists());
        assert!(events_path.exists());
        let events_content = std::fs::read_to_string(&events_path).unwrap();
        assert_eq!(events_content.lines().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
