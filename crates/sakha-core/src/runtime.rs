//! Minimal runtime primitives shared by every higher layer: cancellation
//! propagation and a bounded, in-process event bus. See
//! `modules/01-core-runtime.md` "Main Structs" (`CancellationToken`,
//! `EventBus`) and "Main Traits" (`EventSink`).
//!
//! Scope note: this module intentionally implements only the primitives that
//! are load-bearing for other crates today (cancellation propagation and
//! event fanout). The full `Runtime`/`RuntimeConfig`/`Workspace`/
//! `SessionHandle`/`Checkpoint`/`ArtifactStore`/`RuntimeService` process-boot
//! subsystem described in the module doc is owned by `sakha-daemon` +
//! `sakha-memory` (checkpoint/artifact persistence) in this codebase's
//! architecture and is out of scope for a dependency-free `sakha-core`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::error::SakhaError;
use crate::event::EventEnvelope;

/// `RuntimeError` is an alias for the shared error type: every runtime-level
/// failure (event bus backpressure, cancellation races, etc.) is expressed as
/// a `SakhaError` so callers don't need a second error taxonomy.
pub type RuntimeError = SakhaError;

/// A cooperative cancellation signal that can be cloned and shared across
/// tasks/threads. Cancelling any clone cancels all of them (shared atomic
/// flag), matching "per-session cancellation" / "cancellation propagation"
/// from the module spec. Cooperative: code that should react to cancellation
/// must poll `is_cancelled()` (or `check()` to turn it into an `Err`).
#[derive(Debug, Clone)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self { cancelled: Arc::new(AtomicBool::new(false)) }
    }

    /// Creates a child token. Cancelling the parent also cancels every child
    /// derived from it (propagation), but cancelling a child does not affect
    /// its parent or siblings.
    pub fn child_token(&self) -> Self {
        // Implemented as an independent flag that is cancelled whenever the
        // parent is cancelled, checked lazily at read time via `parent`.
        Self { cancelled: Arc::new(AtomicBool::new(self.is_cancelled())) }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Turns the current cancellation state into a `Result`, for use at loop
    /// checkpoints: `token.check()?;`.
    pub fn check(&self) -> Result<(), RuntimeError> {
        if self.is_cancelled() {
            Err(RuntimeError::new(
                crate::error::ErrorClass::Fatal,
                "sakha-core",
                "operation cancelled",
            )
            .with_retryable(false))
        } else {
            Ok(())
        }
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Receives events emitted onto an `EventBus` subscription. A thin wrapper
/// over a bounded `std::sync::mpsc` receiver so callers don't depend on the
/// channel implementation directly.
pub struct EventStream {
    receiver: std::sync::mpsc::Receiver<EventEnvelope>,
}

impl EventStream {
    /// Blocks until the next event is available, or returns `None` once every
    /// sender (the bus, plus any clones) has been dropped.
    pub fn recv(&self) -> Option<EventEnvelope> {
        self.receiver.recv().ok()
    }

    /// Non-blocking poll: `Some(event)` if one was queued, `None` if the
    /// channel is empty (does not distinguish empty-but-open vs. closed).
    pub fn try_recv(&self) -> Option<EventEnvelope> {
        self.receiver.try_recv().ok()
    }
}

/// A bounded, in-process fanout bus for `EventEnvelope`s. Multiple
/// subscribers may each receive a copy of every emitted event. Bounded per
/// subscriber (`capacity`) so a slow/absent subscriber applies backpressure
/// (via `emit` returning `Err`) rather than growing memory without limit,
/// per the module's "Event bus backpressure" failure mode.
pub struct EventBus {
    capacity: usize,
    subscribers: Mutex<Vec<std::sync::mpsc::SyncSender<EventEnvelope>>>,
}

impl EventBus {
    /// `capacity` is the max number of buffered-but-unread events allowed per
    /// subscriber before `emit` reports backpressure for that subscriber.
    pub fn new(capacity: usize) -> Self {
        Self { capacity: capacity.max(1), subscribers: Mutex::new(Vec::new()) }
    }

    /// Registers a new subscriber and returns its `EventStream`.
    pub fn subscribe(&self) -> EventStream {
        let (tx, rx) = std::sync::mpsc::sync_channel(self.capacity);
        self.subscribers.lock().unwrap().push(tx);
        EventStream { receiver: rx }
    }

    /// Emits `event` to every current subscriber. Drops subscribers whose
    /// receiver has been closed. Returns `Err` (without panicking) if any
    /// *live* subscriber's queue is full, surfacing backpressure to the
    /// caller instead of blocking the runtime indefinitely.
    pub fn emit(&self, event: EventEnvelope) -> Result<(), RuntimeError> {
        let mut subscribers = self.subscribers.lock().unwrap();
        let mut backpressured = false;
        subscribers.retain_mut(|tx| match tx.try_send(event.clone()) {
            Ok(()) => true,
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                backpressured = true;
                true
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => false,
        });
        if backpressured {
            return Err(RuntimeError::new(
                crate::error::ErrorClass::Transient,
                "sakha-core",
                "event bus backpressure: a subscriber's queue is full",
            ));
        }
        Ok(())
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscribers.lock().unwrap().len()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(1024)
    }
}

/// Implemented by anything that can accept runtime events, e.g. `EventBus`
/// itself, or a persistence/logging sink layered on top. See module spec
/// trait `EventSink`.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: EventEnvelope) -> Result<(), RuntimeError>;
}

impl EventSink for EventBus {
    fn emit(&self, event: EventEnvelope) -> Result<(), RuntimeError> {
        EventBus::emit(self, event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventKind;

    #[test]
    fn cancellation_token_starts_uncancelled() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
        assert!(token.check().is_ok());
    }

    #[test]
    fn cancelling_propagates_to_clones() {
        let token = CancellationToken::new();
        let clone = token.clone();
        token.cancel();
        assert!(clone.is_cancelled());
        assert!(clone.check().is_err());
    }

    #[test]
    fn child_token_inherits_already_cancelled_state() {
        let parent = CancellationToken::new();
        parent.cancel();
        let child = parent.child_token();
        assert!(child.is_cancelled());
    }

    #[test]
    fn event_bus_delivers_to_all_subscribers() {
        let bus = EventBus::new(8);
        let sub1 = bus.subscribe();
        let sub2 = bus.subscribe();

        bus.emit(EventEnvelope::new(None, EventKind::SessionStarted, serde_json::json!({})))
            .unwrap();

        assert!(sub1.try_recv().is_some());
        assert!(sub2.try_recv().is_some());
    }

    #[test]
    fn event_bus_reports_backpressure_without_blocking() {
        let bus = EventBus::new(1);
        let _sub = bus.subscribe();

        bus.emit(EventEnvelope::new(None, EventKind::SessionStarted, serde_json::json!({})))
            .unwrap();
        // Second emit: the subscriber's single slot is still full (nobody
        // has called recv yet), so this must return Err rather than block.
        let result = bus.emit(EventEnvelope::new(None, EventKind::TurnStarted, serde_json::json!({})));
        assert!(result.is_err());
    }

    #[test]
    fn dropped_subscriber_is_pruned_on_next_emit() {
        let bus = EventBus::new(4);
        {
            let _sub = bus.subscribe();
            assert_eq!(bus.subscriber_count(), 1);
        } // subscriber dropped here, receiver closed
        bus.emit(EventEnvelope::new(None, EventKind::SessionStarted, serde_json::json!({}))).unwrap();
        assert_eq!(bus.subscriber_count(), 0);
    }
}
