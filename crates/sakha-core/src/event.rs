//! Event envelope: the common wrapper for every event flowing through the
//! runtime's event bus (session lifecycle, tool calls, compression, loops,
//! checkpoints, budgets). See `04-core-domain-model.md` "Event Types".

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::id::SessionId;
use crate::time::now_utc;

/// The payload discriminant for an event. Kept intentionally loose (a tag +
/// JSON body) so new event kinds can be added without breaking the envelope
/// shape; strongly-typed payloads can be layered on top in higher crates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    SessionStarted,
    TurnStarted,
    ModelRequestStarted,
    ModelStreamDelta,
    ToolCallRequested,
    PermissionRequested,
    ToolCallStarted,
    ToolCallCompleted,
    ContextCompressed,
    CompressionRetrievalRequested,
    VerificationStarted,
    VerificationCompleted,
    LoopTickStarted,
    LoopTickCompleted,
    CheckpointWritten,
    BudgetExceeded,
    StallDetected,
    SessionCompleted,
    SessionBlocked,
}

/// Global monotonic sequence generator for `EventEnvelope::sequence`.
///
/// Sequence numbers are assigned process-wide (not per-session) so that a
/// global merge/replay of events from multiple sessions preserves emission
/// order. Per-session ordering is a strict subsequence of this ordering.
static GLOBAL_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn next_sequence() -> u64 {
    GLOBAL_SEQUENCE.fetch_add(1, Ordering::SeqCst)
}

/// The common envelope wrapping every runtime event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    /// Monotonically increasing sequence number, assigned at construction time.
    /// Guarantees a total, gap-free-per-process ordering for replay/audit.
    pub sequence: u64,
    pub session_id: Option<SessionId>,
    pub kind: EventKind,
    pub occurred_at: DateTime<Utc>,
    /// Arbitrary structured payload for the event kind.
    pub payload: serde_json::Value,
}

impl EventEnvelope {
    /// Constructs a new envelope, assigning the next global sequence number
    /// and stamping the current UTC time.
    pub fn new(session_id: Option<SessionId>, kind: EventKind, payload: serde_json::Value) -> Self {
        Self {
            sequence: next_sequence(),
            session_id,
            kind,
            occurred_at: now_utc(),
            payload,
        }
    }

    pub fn for_session(session_id: SessionId, kind: EventKind, payload: serde_json::Value) -> Self {
        Self::new(Some(session_id), kind, payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sequence_numbers_strictly_increase() {
        let e1 = EventEnvelope::new(None, EventKind::SessionStarted, json!({}));
        let e2 = EventEnvelope::new(None, EventKind::TurnStarted, json!({}));
        let e3 = EventEnvelope::new(None, EventKind::SessionCompleted, json!({}));
        assert!(e1.sequence < e2.sequence);
        assert!(e2.sequence < e3.sequence);
    }

    #[test]
    fn envelope_roundtrips_through_json() {
        let session = SessionId::new();
        let event = EventEnvelope::for_session(
            session,
            EventKind::ToolCallCompleted,
            json!({"tool": "read_file", "status": "ok"}),
        );
        let s = serde_json::to_string(&event).unwrap();
        let back: EventEnvelope = serde_json::from_str(&s).unwrap();
        assert_eq!(back.sequence, event.sequence);
        assert_eq!(back.session_id, event.session_id);
        assert_eq!(back.kind, event.kind);
    }

    #[test]
    fn ordering_is_preserved_when_sorted_by_sequence() {
        let mut events = vec![
            EventEnvelope::new(None, EventKind::LoopTickCompleted, json!({})),
            EventEnvelope::new(None, EventKind::LoopTickStarted, json!({})),
            EventEnvelope::new(None, EventKind::BudgetExceeded, json!({})),
        ];
        let original_sequences: Vec<u64> = events.iter().map(|e| e.sequence).collect();
        events.sort_by_key(|e| e.sequence);
        let sorted_sequences: Vec<u64> = events.iter().map(|e| e.sequence).collect();
        // Since they were constructed in increasing sequence order, sorting
        // should be a no-op equal to the original construction order.
        assert_eq!(original_sequences, sorted_sequences);
        assert!(sorted_sequences.windows(2).all(|w| w[0] < w[1]));
    }
}
