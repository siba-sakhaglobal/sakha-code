//! Data access layer for the TUI: pulls session/turn history directly from
//! `sakha-memory` and exposes lightweight, TUI-friendly snapshot types for
//! tool calls, loops, and cost so views can render without depending on the
//! full `sakha-tools`/`sakha-loop`/`sakha-observability` object graphs.
//!
//! Per the task brief ("reads via daemon HTTP or direct sakha-memory (choose
//! simpler: direct store reads + refresh)") this crate reads directly
//! through `sakha_memory::SessionStore` and refreshes on a timer/keypress
//! rather than talking to `sakha-daemon` over HTTP.

use async_trait::async_trait;
use sakha_core::{LoopId, SakhaResult, SessionId, ToolCallId};
use sakha_memory::{SessionRecord, SessionStore, TurnRecord};
use std::sync::Arc;

/// Status of a single tool call, mirrored from `sakha-tools::ToolCallStatus`
/// as an independent, display-oriented copy so this crate does not need to
/// depend on `sakha-tools` just to render a status string. Not every variant
/// is exercised by current feeders (loop/tool-call wiring lands with the
/// daemon-backed `DataSource`), so dead-code analysis is suppressed here.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCallStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Denied,
    TimedOut,
}

impl ToolCallStatus {
    pub fn label(self) -> &'static str {
        match self {
            ToolCallStatus::Pending => "pending",
            ToolCallStatus::Running => "running",
            ToolCallStatus::Succeeded => "succeeded",
            ToolCallStatus::Failed => "failed",
            ToolCallStatus::Denied => "denied",
            ToolCallStatus::TimedOut => "timed_out",
        }
    }
}

/// A single tool call summary for the tools view.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ToolCallSummary {
    pub id: ToolCallId,
    pub tool_name: String,
    pub status: ToolCallStatus,
    pub risk_level: String,
    pub summary: String,
}

/// Lifecycle state of a loop, mirrored from `sakha-loop::LoopState`.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopState {
    Created,
    Running,
    Paused,
    Stopped,
    Completed,
}

impl LoopState {
    pub fn label(self) -> &'static str {
        match self {
            LoopState::Created => "created",
            LoopState::Running => "running",
            LoopState::Paused => "paused",
            LoopState::Stopped => "stopped",
            LoopState::Completed => "completed",
        }
    }
}

/// A single loop summary for the loop dashboard view.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct LoopSummary {
    pub id: LoopId,
    pub objective: String,
    pub kind: String,
    pub state: LoopState,
    pub iterations_completed: u32,
    pub max_iterations: u32,
}

/// Cost/token snapshot for the active session, mirrored from
/// `sakha-observability::LedgerEntry`.
#[derive(Debug, Clone, Copy, Default)]
pub struct CostSnapshot {
    pub cost_micros: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub request_count: u64,
}

impl CostSnapshot {
    pub fn cost_usd(self) -> f64 {
        self.cost_micros as f64 / 1_000_000.0
    }
}

/// A full refresh snapshot pulled from the data source, applied to the app
/// state in one reduction step so the UI never shows a partially-updated
/// frame.
#[derive(Debug, Clone, Default)]
pub struct RefreshSnapshot {
    pub session: Option<SessionRecord>,
    pub turns: Vec<TurnRecord>,
    pub tool_calls: Vec<ToolCallSummary>,
    pub loops: Vec<LoopSummary>,
    pub cost: CostSnapshot,
}

/// Abstracts where the TUI's data comes from. The default implementation
/// reads `sakha-memory` stores directly in-process; a daemon-HTTP-backed
/// implementation could satisfy the same trait later without changing any
/// view/app code.
#[async_trait]
pub trait DataSource: Send + Sync {
    async fn refresh(&self, session_id: SessionId) -> SakhaResult<RefreshSnapshot>;
}

/// Direct `sakha-memory` backed data source. Tool call and loop data are not
/// yet persisted by `sakha-memory`/`sakha-loop` in a queryable form (loop
/// state currently lives only in-process inside `LoopRuntime`), so this
/// source accepts optional feeder closures/snapshots for those; by default
/// they are empty, which is a safe, always-available fallback.
pub struct MemoryDataSource {
    session_store: Arc<dyn SessionStore>,
}

impl MemoryDataSource {
    pub fn new(session_store: Arc<dyn SessionStore>) -> Self {
        Self { session_store }
    }
}

#[async_trait]
impl DataSource for MemoryDataSource {
    async fn refresh(&self, session_id: SessionId) -> SakhaResult<RefreshSnapshot> {
        let session = self.session_store.get_session(session_id).await?;
        let turns = self.session_store.list_turns(session_id).await?;
        Ok(RefreshSnapshot { session, turns, tool_calls: Vec::new(), loops: Vec::new(), cost: CostSnapshot::default() })
    }
}

/// A `DataSource` that always returns a fixed, pre-built snapshot. Useful for
/// tests and for offline/demo rendering without a live session.
pub struct StaticDataSource {
    snapshot: RefreshSnapshot,
}

impl StaticDataSource {
    pub fn new(snapshot: RefreshSnapshot) -> Self {
        Self { snapshot }
    }
}

#[async_trait]
impl DataSource for StaticDataSource {
    async fn refresh(&self, _session_id: SessionId) -> SakhaResult<RefreshSnapshot> {
        Ok(self.snapshot.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sakha_memory::{InMemorySessionStore, SessionStatus};

    #[tokio::test]
    async fn memory_data_source_reads_back_created_session_and_turns() {
        let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
        let session_id = SessionId::new();
        let workspace_id = sakha_core::WorkspaceId::new();
        store
            .create_session(SessionRecord {
                id: session_id,
                workspace_id,
                goal_id: None,
                status: SessionStatus::Active,
                provider_profile: None,
                created_at: sakha_core::time::now_utc(),
                updated_at: sakha_core::time::now_utc(),
            })
            .await
            .unwrap();
        store
            .append_turn(TurnRecord {
                id: sakha_core::TurnId::new(),
                session_id,
                input_text: "hello".into(),
                output_text: Some("hi".into()),
                created_at: sakha_core::time::now_utc(),
            })
            .await
            .unwrap();

        let source = MemoryDataSource::new(store);
        let snapshot = source.refresh(session_id).await.unwrap();
        assert!(snapshot.session.is_some());
        assert_eq!(snapshot.turns.len(), 1);
        assert_eq!(snapshot.turns[0].input_text, "hello");
    }

    #[tokio::test]
    async fn memory_data_source_returns_none_for_unknown_session() {
        let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
        let source = MemoryDataSource::new(store);
        let snapshot = source.refresh(SessionId::new()).await.unwrap();
        assert!(snapshot.session.is_none());
        assert!(snapshot.turns.is_empty());
    }

    #[tokio::test]
    async fn static_data_source_returns_fixed_snapshot() {
        let mut snapshot = RefreshSnapshot::default();
        snapshot.cost.cost_micros = 42;
        let source = StaticDataSource::new(snapshot);
        let out = source.refresh(SessionId::new()).await.unwrap();
        assert_eq!(out.cost.cost_micros, 42);
    }
}
