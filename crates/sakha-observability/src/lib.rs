//! sakha-observability: logging, spans, metrics, cost ledger, audit export.
//!
//! Public API skeleton — see spec `modules/15-observability-audit.md` and
//! `crates/crate-work-breakdown.md`.

pub mod audit_export;
pub mod cost_ledger;
pub mod logging;
pub mod metrics;
pub mod spans;

pub use audit_export::{build_session_report, export_without_redaction, AuditExporter, AuditLog, SessionReport};
pub use cost_ledger::{CostLedger, LedgerEntry, TokenAccounting};
pub use logging::{
    init_logging, redact_with, LoggingHandle, NoopRedactionHook, ObservabilityConfig, PatternRedactionHook,
    RedactionHook,
};
pub use metrics::{MetricRecorder, MetricSample};
pub use spans::TraceSpan;

/// Placeholder for a per-loop diagnostic snapshot, expanded once
/// `sakha-loop` is implemented.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LoopDiagnostic {
    pub loop_id: Option<sakha_core::LoopId>,
    pub iterations: u64,
    pub stalled: bool,
}

/// Placeholder for a per-compression-item ledger entry.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CompressionLedger {
    pub items_compressed: u64,
    pub tokens_saved: u64,
}

/// Mirrors `04-core-domain-model.md` `ToolCall` audit fields at the
/// observability layer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolAuditRecord {
    pub tool_call_id: sakha_core::ToolCallId,
    pub tool_name: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub succeeded: Option<bool>,
}

pub fn crate_name() -> &'static str {
    "sakha-observability"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_ledger_totals_across_session() {
        let ledger = CostLedger::new();
        let session = sakha_core::SessionId::new();
        ledger.record(session, 100, 10, 5);
        ledger.record(session, 50, 3, 2);
        assert_eq!(ledger.total_cost_micros(session), 150);
        let tokens = ledger.token_accounting(session);
        assert_eq!(tokens.input_tokens, 13);
        assert_eq!(tokens.output_tokens, 7);
    }

    #[test]
    fn audit_export_round_trips_session_report() {
        let exporter = AuditExporter::new();
        let report = SessionReport {
            session_id: sakha_core::SessionId::new(),
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
    fn metric_recorder_sums_by_name() {
        let recorder = MetricRecorder::new();
        recorder.record("tool.latency_ms", 10.0, vec![]);
        recorder.record("tool.latency_ms", 20.0, vec![]);
        assert_eq!(recorder.total("tool.latency_ms"), 30.0);
    }

    #[test]
    fn trace_span_records_duration_after_end() {
        let mut span = TraceSpan::start("tool.execute", None);
        span.end();
        assert!(span.duration().is_some());
    }
}
