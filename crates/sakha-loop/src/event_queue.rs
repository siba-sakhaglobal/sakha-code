//! `LoopQueue`: work queue with leases for event-driven loop ticks. See spec
//! "Required Capabilities" -> "Loop queue with leases".

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use sakha_core::{LoopId, SakhaResult};

/// A single queued unit of work for a loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueMessage {
    pub loop_id: LoopId,
    pub dedup_key: String,
    pub payload: serde_json::Value,
}

/// A leased message, held exclusively by one worker until acked/nacked.
#[derive(Debug, Clone)]
pub struct LeasedMessage {
    pub message: QueueMessage,
    pub lease_id: uuid::Uuid,
}

/// Work queue abstraction with lease-based delivery and dedup by key.
#[async_trait]
pub trait LoopQueue: Send + Sync {
    async fn enqueue(&self, message: QueueMessage) -> SakhaResult<()>;
    async fn lease_next(&self) -> SakhaResult<Option<LeasedMessage>>;
    async fn ack(&self, lease_id: uuid::Uuid) -> SakhaResult<()>;
    async fn nack(&self, lease_id: uuid::Uuid) -> SakhaResult<()>;
}

/// An in-memory `LoopQueue` for tests and as a safe default. Deduplicates by
/// `dedup_key`.
#[derive(Default)]
pub struct InMemoryLoopQueue {
    inner: std::sync::Mutex<InMemoryQueueState>,
}

#[derive(Default)]
struct InMemoryQueueState {
    pending: Vec<QueueMessage>,
    seen_keys: std::collections::HashSet<String>,
    leased: std::collections::HashMap<uuid::Uuid, QueueMessage>,
}

impl InMemoryLoopQueue {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl LoopQueue for InMemoryLoopQueue {
    async fn enqueue(&self, message: QueueMessage) -> SakhaResult<()> {
        let mut state = self.inner.lock().unwrap();
        if state.seen_keys.contains(&message.dedup_key) {
            return Ok(());
        }
        state.seen_keys.insert(message.dedup_key.clone());
        state.pending.push(message);
        Ok(())
    }

    async fn lease_next(&self) -> SakhaResult<Option<LeasedMessage>> {
        let mut state = self.inner.lock().unwrap();
        if state.pending.is_empty() {
            return Ok(None);
        }
        let message = state.pending.remove(0);
        let lease_id = uuid::Uuid::new_v4();
        state.leased.insert(lease_id, message.clone());
        Ok(Some(LeasedMessage { message, lease_id }))
    }

    async fn ack(&self, lease_id: uuid::Uuid) -> SakhaResult<()> {
        self.inner.lock().unwrap().leased.remove(&lease_id);
        Ok(())
    }

    async fn nack(&self, lease_id: uuid::Uuid) -> SakhaResult<()> {
        let mut state = self.inner.lock().unwrap();
        if let Some(message) = state.leased.remove(&lease_id) {
            state.pending.push(message);
        }
        Ok(())
    }
}
