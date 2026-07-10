//! Route-level integration tests, driven through `tower::ServiceExt::oneshot`
//! per the task spec ("Route tests via axum tower::ServiceExt oneshot").
//! Mirrors the crate-work-breakdown "Tests" list: health endpoint, session
//! create, SSE reconnect, permission decision.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use sakha_daemon::{build_router, DaemonState};

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn json_request(method: &str, uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

#[tokio::test]
async fn health_endpoint_returns_ok() {
    let app = build_router(DaemonState::new());
    let response = app.oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&bytes[..], b"ok");
}

#[tokio::test]
async fn session_create_then_get_round_trips() {
    let app = build_router(DaemonState::new());

    let create_response = app
        .clone()
        .oneshot(json_request("POST", "/sessions", json!({})))
        .await
        .unwrap();
    assert_eq!(create_response.status(), StatusCode::OK);
    let created = body_json(create_response).await;
    let session_id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["status"], "Active");

    let get_response = app
        .oneshot(Request::builder().uri(format!("/sessions/{session_id}")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(get_response.status(), StatusCode::OK);
    let fetched = body_json(get_response).await;
    assert_eq!(fetched["id"], session_id);
}

#[tokio::test]
async fn get_unknown_session_returns_404() {
    let app = build_router(DaemonState::new());
    let random_id = sakha_core::SessionId::new().to_string();
    let response = app
        .oneshot(Request::builder().uri(format!("/sessions/{random_id}")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn malformed_session_id_returns_400() {
    let app = build_router(DaemonState::new());
    let response = app
        .oneshot(Request::builder().uri("/sessions/not-a-uuid").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn session_input_records_turn_and_emits_lifecycle_events() {
    let app = build_router(DaemonState::new());

    let create_response = app.clone().oneshot(json_request("POST", "/sessions", json!({}))).await.unwrap();
    let created = body_json(create_response).await;
    let session_id = created["id"].as_str().unwrap().to_string();

    let input_response = app
        .clone()
        .oneshot(json_request("POST", &format!("/sessions/{session_id}/input"), json!({"text": "hello agent"})))
        .await
        .unwrap();
    assert_eq!(input_response.status(), StatusCode::OK);
    let input_result = body_json(input_response).await;
    assert_eq!(input_result["session_id"], session_id);
    assert!(input_result["turn_id"].as_str().is_some());

    let turns_response = app
        .clone()
        .oneshot(Request::builder().uri(format!("/sessions/{session_id}/turns")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(turns_response.status(), StatusCode::OK);
    let turns = body_json(turns_response).await;
    assert_eq!(turns.as_array().unwrap().len(), 1);
    assert_eq!(turns[0]["input_text"], "hello agent");

    let audit_response = app
        .oneshot(Request::builder().uri(format!("/sessions/{session_id}/audit")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let audit_events = body_json(audit_response).await;
    let kinds: Vec<String> = audit_events.as_array().unwrap().iter().map(|e| e["kind"].as_str().unwrap().to_string()).collect();
    assert!(kinds.contains(&"SessionStarted".to_string()));
    assert!(kinds.contains(&"TurnStarted".to_string()));
    assert!(kinds.contains(&"ModelRequestStarted".to_string()));
}

#[tokio::test]
async fn session_cancel_updates_status_and_emits_event() {
    let app = build_router(DaemonState::new());
    let created = body_json(app.clone().oneshot(json_request("POST", "/sessions", json!({}))).await.unwrap()).await;
    let session_id = created["id"].as_str().unwrap().to_string();

    let cancel_response = app
        .clone()
        .oneshot(json_request("POST", &format!("/sessions/{session_id}/cancel"), json!({"reason": "user requested"})))
        .await
        .unwrap();
    assert_eq!(cancel_response.status(), StatusCode::OK);
    let cancel_result = body_json(cancel_response).await;
    assert_eq!(cancel_result["status"], "blocked");

    let get_response = app
        .oneshot(Request::builder().uri(format!("/sessions/{session_id}")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let fetched = body_json(get_response).await;
    assert_eq!(fetched["status"], "Blocked");
}

#[tokio::test]
async fn sse_reconnect_with_last_event_id_replays_only_newer_events() {
    let state = DaemonState::new();
    let session_id = sakha_core::SessionId::new();

    // Seed two events directly through the shared event bus (as a real turn
    // would), then request the SSE stream with Last-Event-ID set to the
    // first event's sequence: only the second should be eligible for replay.
    let e1 = sakha_core::EventEnvelope::for_session(session_id, sakha_core::EventKind::SessionStarted, json!({}));
    let seq1 = e1.sequence;
    state.emit(e1);
    let e2 = sakha_core::EventEnvelope::for_session(session_id, sakha_core::EventKind::TurnStarted, json!({}));
    let seq2 = e2.sequence;
    state.emit(e2);

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/sessions/{session_id}/events"))
                .header("last-event-id", seq1.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(axum::http::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );

    // The SSE body is a live, never-ending stream (backlog replay chained
    // with a keep-alive live feed), so `to_bytes` would hang forever; poll
    // frames directly and stop as soon as the replayed event has been seen
    // or a bounded timeout elapses.
    use futures::StreamExt;
    let mut stream = response.into_body().into_data_stream();
    let mut collected = String::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(500), stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                collected.push_str(&String::from_utf8_lossy(&chunk));
                if collected.contains(&format!("id: {seq2}")) {
                    break;
                }
            }
            Ok(Some(Err(_))) | Ok(None) => break,
            Err(_) => continue,
        }
    }
    assert!(collected.contains(&format!("id: {seq2}")), "expected replay of newer event, got: {collected:?}");
    assert!(!collected.contains(&format!("id: {seq1}")), "must not replay the already-seen event, got: {collected:?}");
}

#[tokio::test]
async fn permission_flow_list_then_decide() {
    let state = DaemonState::new();
    let session_id = sakha_core::SessionId::new();
    let request = sakha_security::PermissionRequest::new(sakha_security::PermissionKind::FileWrite, "a.txt", "write file");
    let (id, rx) = state.permissions.raise(Some(session_id), request);

    let app = build_router(state);

    let list_response = app.clone().oneshot(Request::builder().uri("/permissions").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(list_response.status(), StatusCode::OK);
    let pending = body_json(list_response).await;
    assert_eq!(pending.as_array().unwrap().len(), 1);
    assert_eq!(pending[0]["id"], id.to_string());

    let decide_response = app
        .oneshot(json_request("POST", &format!("/permissions/{id}/decision"), json!({"decision": "allow"})))
        .await
        .unwrap();
    assert_eq!(decide_response.status(), StatusCode::OK);
    let decided = body_json(decide_response).await;
    assert_eq!(decided["id"], id.to_string());

    let decision = rx.await.unwrap();
    assert!(decision.is_allowed());
}

#[tokio::test]
async fn deciding_unknown_permission_returns_404() {
    let app = build_router(DaemonState::new());
    let random_id = sakha_daemon::PermissionRequestId::new().to_string();
    let response = app
        .oneshot(json_request("POST", &format!("/permissions/{random_id}/decision"), json!({"decision": "deny"})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn loop_create_then_list_and_pause() {
    let app = build_router(DaemonState::new());

    let create_response = app
        .clone()
        .oneshot(json_request("POST", "/loops", json!({"objective": "keep CI green"})))
        .await
        .unwrap();
    assert_eq!(create_response.status(), StatusCode::OK);
    let created = body_json(create_response).await;
    let loop_id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["objective"], "keep CI green");

    let pause_response = app
        .oneshot(Request::builder().method("POST").uri(format!("/loops/{loop_id}/pause")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(pause_response.status(), StatusCode::OK);
    let paused = body_json(pause_response).await;
    assert_eq!(paused["state"], "paused");
}

#[tokio::test]
async fn invalid_loop_action_on_unknown_id_is_rejected() {
    let app = build_router(DaemonState::new());
    let random_id = sakha_core::LoopId::new().to_string();
    let response = app
        .oneshot(Request::builder().method("POST").uri(format!("/loops/{random_id}/pause")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR.min(response.status()).max(StatusCode::BAD_REQUEST));
    assert!(response.status().is_client_error() || response.status().is_server_error());
}

#[tokio::test]
async fn audit_endpoint_reflects_emitted_events() {
    let state = DaemonState::new();
    let session_id = sakha_core::SessionId::new();
    state.emit(sakha_core::EventEnvelope::for_session(session_id, sakha_core::EventKind::SessionStarted, json!({})));

    let app = build_router(state);
    let response = app.oneshot(Request::builder().uri("/audit").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let events = body_json(response).await;
    assert_eq!(events.as_array().unwrap().len(), 1);
    assert_eq!(events[0]["kind"], "SessionStarted");
}
