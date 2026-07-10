//! Pending-permission queue: the bridge between a running agent turn (which
//! needs a human/UI decision on a `PermissionRequest`) and the
//! `POST /permissions/:id/decision` route. An agent-loop task calls
//! `PermissionQueue::request(...).await`, which suspends until some HTTP
//! caller resolves the same id via `PermissionQueue::decide(...)`. See spec
//! `modules/19-api-contracts.md` "Core Endpoints" and
//! `modules/14-security-sandbox-permissions.md`.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use uuid::Uuid;

use sakha_core::{SakhaError, SakhaResult, SessionId};
use sakha_security::{PermissionDecision, PermissionRequest};

/// Unique id for one pending permission request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PermissionRequestId(pub Uuid);

impl PermissionRequestId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for PermissionRequestId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for PermissionRequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for PermissionRequestId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// A permission request that is currently awaiting a human/UI decision.
#[derive(Debug, Clone, Serialize)]
pub struct PendingPermission {
    pub id: PermissionRequestId,
    pub session_id: Option<SessionId>,
    pub request: PermissionRequest,
}

struct PendingEntry {
    pending: PendingPermission,
    responder: Option<oneshot::Sender<PermissionDecision>>,
}

/// Queue of pending permission requests awaiting an out-of-band decision.
/// Safe default: if nothing ever decides a request, `request()` waits
/// forever (the caller is expected to apply its own timeout/cancellation),
/// matching the "agent loop awaits" contract in the task spec.
#[derive(Default)]
pub struct PermissionQueue {
    pending: Mutex<HashMap<PermissionRequestId, PendingEntry>>,
}

impl PermissionQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Raises a new pending permission request and returns both its id (for
    /// UI display / routing) and a future that resolves once a decision is
    /// posted via `decide`.
    pub fn raise(&self, session_id: Option<SessionId>, request: PermissionRequest) -> (PermissionRequestId, oneshot::Receiver<PermissionDecision>) {
        let id = PermissionRequestId::new();
        let (tx, rx) = oneshot::channel();
        let pending = PendingPermission { id, session_id, request };
        self.pending.lock().unwrap().insert(id, PendingEntry { pending, responder: Some(tx) });
        (id, rx)
    }

    /// Convenience wrapper: raises a request and awaits its decision.
    pub async fn request(&self, session_id: Option<SessionId>, request: PermissionRequest) -> SakhaResult<PermissionDecision> {
        let (_id, rx) = self.raise(session_id, request);
        rx.await.map_err(|_| SakhaError::integrity("sakha-daemon", "permission decision channel closed before a decision was made"))
    }

    /// Resolves a pending request with a decision, waking the awaiting
    /// agent-loop task. Returns an error if the id is unknown or already
    /// resolved.
    pub fn decide(&self, id: PermissionRequestId, decision: PermissionDecision) -> SakhaResult<()> {
        let mut pending = self.pending.lock().unwrap();
        let entry = pending
            .remove(&id)
            .ok_or_else(|| SakhaError::invalid_input("sakha-daemon", format!("unknown permission request: {id}")))?;
        if let Some(responder) = entry.responder {
            let _ = responder.send(decision);
        }
        Ok(())
    }

    /// Lists all currently pending (undecided) requests.
    pub fn list_pending(&self) -> Vec<PendingPermission> {
        self.pending.lock().unwrap().values().map(|e| e.pending.clone()).collect()
    }

    pub fn get(&self, id: PermissionRequestId) -> Option<PendingPermission> {
        self.pending.lock().unwrap().get(&id).map(|e| e.pending.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sakha_security::PermissionKind;
    use std::sync::Arc;

    #[tokio::test]
    async fn request_suspends_until_decide_is_called() {
        let queue = Arc::new(PermissionQueue::new());
        let session = SessionId::new();
        let req = PermissionRequest::new(PermissionKind::FileWrite, "a.txt", "write file");

        let queue_clone = Arc::clone(&queue);
        let (id, rx) = queue_clone.raise(Some(session), req);

        assert_eq!(queue.list_pending().len(), 1);

        let handle = tokio::spawn(async move { rx.await.unwrap() });

        // Give the spawned task a chance to start awaiting.
        tokio::task::yield_now().await;

        queue.decide(id, PermissionDecision::allowed()).unwrap();
        let decision = handle.await.unwrap();
        assert!(decision.is_allowed());
        assert!(queue.list_pending().is_empty());
    }

    #[tokio::test]
    async fn deciding_unknown_id_errors() {
        let queue = PermissionQueue::new();
        let result = queue.decide(PermissionRequestId::new(), PermissionDecision::allowed());
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn deciding_twice_errors_on_second_call() {
        let queue = PermissionQueue::new();
        let req = PermissionRequest::new(PermissionKind::ShellRunArbitrary, "rm -rf /tmp/x", "cleanup");
        let (id, _rx) = queue.raise(None, req);
        queue.decide(id, PermissionDecision::denied("too risky")).unwrap();
        assert!(queue.decide(id, PermissionDecision::allowed()).is_err());
    }
}
