//! Per-session event bus: fans out `EventEnvelope`s to SSE and WebSocket
//! subscribers, and retains a bounded in-memory backlog per session so a
//! reconnecting client can replay events after a given sequence number
//! (`Last-Event-ID`). See spec `modules/19-api-contracts.md` "Event Protocol"
//! and "Tests" -> "SSE reconnect with last event ID".

use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::broadcast;

use sakha_core::{EventEnvelope, SessionId};

/// How many past events to retain per session for reconnect replay.
const BACKLOG_CAPACITY: usize = 512;

/// A single session's broadcast channel plus a bounded backlog buffer.
struct SessionChannel {
    sender: broadcast::Sender<EventEnvelope>,
    backlog: Vec<EventEnvelope>,
}

impl SessionChannel {
    fn new() -> Self {
        let (sender, _receiver) = broadcast::channel(256);
        Self { sender, backlog: Vec::new() }
    }

    fn push_backlog(&mut self, envelope: &EventEnvelope) {
        self.backlog.push(envelope.clone());
        if self.backlog.len() > BACKLOG_CAPACITY {
            let overflow = self.backlog.len() - BACKLOG_CAPACITY;
            self.backlog.drain(0..overflow);
        }
    }
}

/// Central event bus: one broadcast channel + backlog per session.
#[derive(Default)]
pub struct EventBus {
    channels: Mutex<HashMap<SessionId, SessionChannel>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Publishes an event for a session, appending it to the backlog and
    /// notifying any live subscribers. Publishing never fails: if there are
    /// no subscribers the broadcast send error is swallowed (the backlog
    /// still retains the event for later reconnect/history reads).
    pub fn publish(&self, session_id: SessionId, envelope: EventEnvelope) {
        let mut channels = self.channels.lock().unwrap();
        let channel = channels.entry(session_id).or_insert_with(SessionChannel::new);
        channel.push_backlog(&envelope);
        let _ = channel.sender.send(envelope);
    }

    /// Subscribes to live events for a session (created lazily). Returns the
    /// receiver plus a snapshot of the current backlog for callers that want
    /// to replay history immediately (e.g. SSE reconnect).
    pub fn subscribe(&self, session_id: SessionId) -> (broadcast::Receiver<EventEnvelope>, Vec<EventEnvelope>) {
        let mut channels = self.channels.lock().unwrap();
        let channel = channels.entry(session_id).or_insert_with(SessionChannel::new);
        (channel.sender.subscribe(), channel.backlog.clone())
    }

    /// Returns backlog events for a session with `sequence` greater than
    /// `last_event_id`, used to serve SSE reconnects via `Last-Event-ID`.
    pub fn events_since(&self, session_id: SessionId, last_event_id: u64) -> Vec<EventEnvelope> {
        let channels = self.channels.lock().unwrap();
        channels
            .get(&session_id)
            .map(|c| c.backlog.iter().filter(|e| e.sequence > last_event_id).cloned().collect())
            .unwrap_or_default()
    }

    /// Returns the full retained backlog for a session (used by `GET /audit`
    /// style introspection and tests).
    pub fn history(&self, session_id: SessionId) -> Vec<EventEnvelope> {
        let channels = self.channels.lock().unwrap();
        channels.get(&session_id).map(|c| c.backlog.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sakha_core::EventKind;
    use serde_json::json;

    #[test]
    fn publish_then_events_since_returns_only_newer_events() {
        let bus = EventBus::new();
        let session = SessionId::new();
        let e1 = EventEnvelope::for_session(session, EventKind::SessionStarted, json!({}));
        let seq1 = e1.sequence;
        bus.publish(session, e1);
        let e2 = EventEnvelope::for_session(session, EventKind::TurnStarted, json!({}));
        bus.publish(session, e2);

        let since = bus.events_since(session, seq1);
        assert_eq!(since.len(), 1);
        assert_eq!(since[0].kind, EventKind::TurnStarted);
    }

    #[tokio::test]
    async fn subscribe_receives_events_published_after_subscription() {
        let bus = EventBus::new();
        let session = SessionId::new();
        let (mut rx, backlog) = bus.subscribe(session);
        assert!(backlog.is_empty());

        bus.publish(session, EventEnvelope::for_session(session, EventKind::SessionStarted, json!({})));
        let received = rx.recv().await.unwrap();
        assert_eq!(received.kind, EventKind::SessionStarted);
    }
}
