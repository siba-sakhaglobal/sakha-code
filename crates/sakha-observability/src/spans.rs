//! Trace spans linking model requests, tool calls, and turns for audit.
//!
//! `TraceSpan` is a lightweight, serializable record independent of the
//! live `tracing::Span` so it can be persisted/exported and correlated
//! against `sakha_core` IDs (turn/tool-call/session) after the fact.

use serde::{Deserialize, Serialize};

/// A lightweight span record, independent of the `tracing` crate's live
/// span so it can be persisted/exported.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSpan {
    pub id: uuid::Uuid,
    pub parent_id: Option<uuid::Uuid>,
    pub name: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub ended_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Optional correlation to a `sakha_core` domain ID (turn, tool call,
    /// session, loop tick, etc), stored as a string so this stays decoupled
    /// from any one ID type.
    pub correlation_id: Option<String>,
}

impl TraceSpan {
    pub fn start(name: impl Into<String>, parent_id: Option<uuid::Uuid>) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            parent_id,
            name: name.into(),
            started_at: chrono::Utc::now(),
            ended_at: None,
            correlation_id: None,
        }
    }

    /// Starts a span as a child of `parent`, inheriting `parent.id` as
    /// `parent_id`. Useful for building a tool-call span under a turn span,
    /// or a model-request span under a tool-call span.
    pub fn child_of(parent: &TraceSpan, name: impl Into<String>) -> Self {
        Self::start(name, Some(parent.id))
    }

    /// Attaches a correlation ID (e.g. a `ToolCallId` or `TurnId`
    /// `to_string()`) so the span can be joined back to a domain entity
    /// during audit export.
    pub fn with_correlation(mut self, correlation_id: impl Into<String>) -> Self {
        self.correlation_id = Some(correlation_id.into());
        self
    }

    pub fn end(&mut self) {
        self.ended_at = Some(chrono::Utc::now());
    }

    pub fn duration(&self) -> Option<chrono::Duration> {
        self.ended_at.map(|end| end - self.started_at)
    }

    pub fn duration_ms(&self) -> Option<i64> {
        self.duration().map(|d| d.num_milliseconds())
    }

    pub fn is_finished(&self) -> bool {
        self.ended_at.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_span_records_duration_after_end() {
        let mut span = TraceSpan::start("tool.execute", None);
        span.end();
        assert!(span.duration().is_some());
    }

    #[test]
    fn unfinished_span_has_no_duration() {
        let span = TraceSpan::start("tool.execute", None);
        assert!(span.duration().is_none());
        assert!(!span.is_finished());
    }

    #[test]
    fn child_of_inherits_parent_id() {
        let parent = TraceSpan::start("turn", None);
        let child = TraceSpan::child_of(&parent, "tool_call");
        assert_eq!(child.parent_id, Some(parent.id));
    }

    #[test]
    fn with_correlation_sets_id() {
        let span = TraceSpan::start("model_request", None).with_correlation("turn-123");
        assert_eq!(span.correlation_id.as_deref(), Some("turn-123"));
    }
}
