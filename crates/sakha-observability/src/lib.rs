//! sakha-observability: logging, spans, metrics, cost ledger, audit export.
//!
//! Public API skeleton — see spec `modules/15-observability-audit.md` and
//! `crates/crate-work-breakdown.md`.

pub mod audit_export;
pub mod cost_ledger;
pub mod logging;
pub mod metrics;
pub mod spans;

pub use audit_export::{
    build_session_report, export_without_redaction, AuditExporter, AuditLog, CommandRun, FileChange, SessionReport,
};
pub use cost_ledger::{CostLedger, LedgerEntry, TokenAccounting, TokenLedger};
pub use logging::{
    init_logging, redact_with, LoggingHandle, NoopRedactionHook, ObservabilityConfig, PatternRedactionHook,
    RedactionHook,
};
pub use metrics::{MetricRecorder, MetricSample};
pub use spans::TraceSpan;

/// The top-level audit facility named in spec `modules/15-observability-audit.md`
/// "Main Structs": combines the append-only [`AuditLog`] (write path) with an
/// [`AuditExporter`] (read/export path) so callers depend on one type for
/// "record an audit event, later export a session report" rather than wiring
/// the two pieces together themselves. `AuditLog` and `AuditExporter` remain
/// independently usable for callers that only need one half.
#[derive(Default)]
pub struct AuditLogger {
    pub log: AuditLog,
    pub exporter: AuditExporter,
}

impl AuditLogger {
    pub fn new() -> Self {
        Self { log: AuditLog::new(), exporter: AuditExporter::new() }
    }

    pub fn with_config(config: ObservabilityConfig) -> Self {
        Self { log: AuditLog::new(), exporter: AuditExporter::with_config(config) }
    }

    /// Appends an event to the underlying append-only log. See [`AuditLog::append`].
    pub fn append(&self, event: sakha_core::EventEnvelope) {
        self.log.append(event);
    }

    /// Builds and exports (as pretty JSON) a `SessionReport` for `session_id`
    /// from every event recorded so far, in one call.
    pub fn export_session_report(&self, session_id: sakha_core::SessionId, total_cost_micros: u64) -> SakhaObservabilityResult<String> {
        let report = build_session_report(&self.log, session_id, total_cost_micros);
        self.exporter.export_json(&report)
    }
}

type SakhaObservabilityResult<T> = sakha_core::SakhaResult<T>;

/// A per-loop-tick diagnostic tracker, per spec "Implement loop diagnostics"
/// and the "Loop iteration count" metric. Decoupled from `sakha-loop` (which
/// is not yet implemented) per the crate's external-integration rule: this
/// type only needs a loop id and a stream of "tick happened" / "tick made
/// progress" signals from whatever driver eventually wires it up, so it can
/// be built, tested, and used standalone today.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LoopDiagnostic {
    pub loop_id: Option<sakha_core::LoopId>,
    /// Total number of ticks recorded via [`record_tick`](Self::record_tick).
    pub iterations: u64,
    /// Number of consecutive ticks recorded with `made_progress = false`.
    pub consecutive_no_progress: u64,
    /// Set once `consecutive_no_progress` reaches `stall_threshold`.
    pub stalled: bool,
    /// Number of no-progress ticks in a row before `stalled` is set. Defaults
    /// to 3 (via [`LoopDiagnostic::new`]) — small enough to catch a stuck
    /// loop quickly, large enough to tolerate a single transient no-op tick.
    pub stall_threshold: u64,
}

impl LoopDiagnostic {
    pub fn new(loop_id: sakha_core::LoopId) -> Self {
        Self { loop_id: Some(loop_id), iterations: 0, consecutive_no_progress: 0, stalled: false, stall_threshold: 3 }
    }

    pub fn with_stall_threshold(loop_id: sakha_core::LoopId, stall_threshold: u64) -> Self {
        Self { stall_threshold: stall_threshold.max(1), ..Self::new(loop_id) }
    }

    /// Records one loop tick. `made_progress` should reflect whether the tick
    /// changed anything observable (a file written, a goal advanced, etc.);
    /// `stalled` latches true once `stall_threshold` consecutive no-progress
    /// ticks have been recorded, and is cleared by the next progressing tick.
    pub fn record_tick(&mut self, made_progress: bool) {
        self.iterations += 1;
        if made_progress {
            self.consecutive_no_progress = 0;
            self.stalled = false;
        } else {
            self.consecutive_no_progress += 1;
            if self.consecutive_no_progress >= self.stall_threshold {
                self.stalled = true;
            }
        }
    }
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
            commands_run: vec![audit_export::CommandRun {
                command: "cargo test".into(),
                tool_call_id: None,
                turn_id: None,
            }],
            files_changed: vec![audit_export::FileChange {
                file_path: "src/lib.rs".into(),
                tool_call_id: None,
                turn_id: None,
            }],
            total_cost_micros: 42,
            unparsed_events: Vec::new(),
        };
        let json = exporter.export_json(&report).unwrap();
        let back: SessionReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.session_id, report.session_id);
        assert_eq!(back.commands_run.len(), report.commands_run.len());
        assert_eq!(back.commands_run[0].command, report.commands_run[0].command);
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

    #[test]
    fn audit_logger_appends_and_exports_session_report() {
        let logger = AuditLogger::new();
        let session = sakha_core::SessionId::new();
        logger.append(sakha_core::EventEnvelope::for_session(
            session,
            sakha_core::EventKind::ToolCallCompleted,
            serde_json::json!({"command": "cargo build"}),
        ));
        let json = logger.export_session_report(session, 10).unwrap();
        assert!(json.contains("cargo build"));
    }

    #[test]
    fn loop_diagnostic_flags_stall_after_consecutive_no_progress_ticks() {
        let loop_id = sakha_core::LoopId::new();
        let mut diag = LoopDiagnostic::with_stall_threshold(loop_id, 2);
        diag.record_tick(true);
        assert!(!diag.stalled);
        diag.record_tick(false);
        assert!(!diag.stalled);
        diag.record_tick(false);
        assert!(diag.stalled);
        assert_eq!(diag.iterations, 3);
        diag.record_tick(true);
        assert!(!diag.stalled);
    }
}
