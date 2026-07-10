//! `DaemonState`: shared application state for the local daemon (axum
//! `State` extension). See spec `modules/13-ui-cli-tui-web-desktop.md` and
//! `modules/19-api-contracts.md`.

use std::sync::Arc;

use sakha_loop::LoopRuntime;
use sakha_memory::InMemorySessionStore;
use sakha_observability::{AuditLog, CostLedger, MetricRecorder};

use crate::events::EventBus;
use crate::permission_queue::PermissionQueue;

/// Shared, cloneable daemon state injected into every axum handler.
#[derive(Clone)]
pub struct DaemonState {
    pub sessions: Arc<InMemorySessionStore>,
    pub loops: Arc<LoopRuntime>,
    pub cost_ledger: Arc<CostLedger>,
    pub metrics: Arc<MetricRecorder>,
    /// Fan-out event bus for SSE/WebSocket session event streams.
    pub events: Arc<EventBus>,
    /// Pending human/UI permission decisions the agent loop awaits on.
    pub permissions: Arc<PermissionQueue>,
    /// Append-only audit trail of every envelope ever published, exposed via
    /// `GET /audit`.
    pub audit: Arc<AuditLog>,
}

impl DaemonState {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(InMemorySessionStore::new()),
            loops: Arc::new(LoopRuntime::new()),
            cost_ledger: Arc::new(CostLedger::new()),
            metrics: Arc::new(MetricRecorder::new()),
            events: Arc::new(EventBus::new()),
            permissions: Arc::new(PermissionQueue::new()),
            audit: Arc::new(AuditLog::new()),
        }
    }

    /// Publishes an event into the per-session event bus and appends it to
    /// the durable audit log. This is the single choke point every route
    /// should go through when emitting a lifecycle event, so SSE, WebSocket,
    /// and `/audit` all stay consistent.
    pub fn emit(&self, envelope: sakha_core::EventEnvelope) {
        if let Some(session_id) = envelope.session_id {
            self.events.publish(session_id, envelope.clone());
        }
        self.audit.append(envelope);
    }
}

impl Default for DaemonState {
    fn default() -> Self {
        Self::new()
    }
}
