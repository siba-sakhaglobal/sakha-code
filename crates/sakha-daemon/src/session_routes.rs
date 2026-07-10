//! Session REST routes: `POST /sessions`, `GET /sessions/:id`,
//! `POST /sessions/:id/input`, `POST /sessions/:id/cancel`. See spec
//! `modules/19-api-contracts.md` "Core Endpoints".
//!
//! `POST /sessions/:id/input` is the "run endpoint driving sakha-agent" named
//! in the task spec. `sakha_agent::Agent::run_turn` is currently a public-API
//! stub that always returns `SakhaError::not_implemented` (see
//! `crates/sakha-agent/src/agent.rs`); the daemon does not paper over that —
//! it records the turn, emits the full `TurnStarted`/`ModelRequestStarted`
//! lifecycle events a real run would produce, then surfaces the stub's
//! `not_implemented` outcome as a `SessionBlocked` event and a structured
//! `202`-style response so callers can distinguish "accepted but agent loop
//! isn't wired yet" from a hard failure. Once `Agent::run_turn` is
//! implemented, only `drive_turn` below needs to change.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use sakha_core::{EventEnvelope, EventKind, SessionId, TurnId};
use sakha_memory::{SessionRecord, SessionStatus, SessionStore, TurnRecord};

use crate::api::ApiError;
use crate::state::DaemonState;

#[derive(Debug, Serialize)]
pub struct SessionDto {
    pub id: String,
    pub workspace_id: String,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<SessionRecord> for SessionDto {
    fn from(record: SessionRecord) -> Self {
        Self {
            id: record.id.to_string(),
            workspace_id: record.workspace_id.to_string(),
            status: format!("{:?}", record.status),
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub workspace_id: Option<String>,
}

async fn create_session(
    State(state): State<DaemonState>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<Json<SessionDto>, ApiError> {
    let workspace_id = match req.workspace_id {
        Some(raw) => raw.parse().map_err(|_| ApiError::new("invalid_workspace_id", "malformed workspace id"))?,
        None => sakha_core::WorkspaceId::new(),
    };

    let record = SessionRecord {
        id: SessionId::new(),
        workspace_id,
        goal_id: None,
        status: SessionStatus::Active,
        provider_profile: None,
        created_at: sakha_core::time::now_utc(),
        updated_at: sakha_core::time::now_utc(),
    };
    state
        .sessions
        .create_session(record.clone())
        .await
        .map_err(|e| ApiError::new("session_create_failed", e.to_string()))?;

    state.emit(EventEnvelope::for_session(record.id, EventKind::SessionStarted, json!({"workspace_id": record.workspace_id.to_string()})));

    Ok(Json(record.into()))
}

async fn get_session(State(state): State<DaemonState>, Path(id): Path<String>) -> Result<Json<SessionDto>, ApiError> {
    let session_id = parse_session_id(&id)?;
    let record = state
        .sessions
        .get_session(session_id)
        .await
        .map_err(|e| ApiError::new("lookup_failed", e.to_string()))?
        .ok_or_else(|| ApiError::new("not_found", "session not found"))?;
    Ok(Json(record.into()))
}

fn parse_session_id(id: &str) -> Result<SessionId, ApiError> {
    id.parse().map_err(|_| ApiError::new("invalid_id", "malformed session id"))
}

#[derive(Debug, Deserialize)]
pub struct SessionInputRequest {
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct SessionInputResponse {
    pub turn_id: String,
    pub session_id: String,
    /// `true` once a real model/tool turn completed; `false` when the agent
    /// loop is unavailable (current stub state) and only lifecycle
    /// bookkeeping happened.
    pub driven: bool,
    pub note: Option<String>,
}

/// Drives one turn through the agent loop. Isolated from the route handler
/// so the "wire up the real agent" change is a one-function diff. Since
/// `sakha_agent::Agent` requires provider/tool/context/compression/policy
/// dependencies the daemon does not construct on every request (those are
/// process-level configuration, not per-request), and `Agent::run_turn`
/// itself is a stub, this records the turn lifecycle honestly rather than
/// fabricating a fake model response.
async fn drive_turn(state: &DaemonState, session_id: SessionId, turn_id: TurnId, input_text: &str) -> (bool, Option<String>) {
    state.emit(EventEnvelope::for_session(session_id, EventKind::TurnStarted, json!({"turn_id": turn_id.to_string(), "input": input_text})));
    state.emit(EventEnvelope::for_session(session_id, EventKind::ModelRequestStarted, json!({"turn_id": turn_id.to_string()})));

    let err = sakha_core::SakhaError::not_implemented("sakha-agent", "Agent::run_turn");
    state.emit(EventEnvelope::for_session(
        session_id,
        EventKind::SessionBlocked,
        json!({"turn_id": turn_id.to_string(), "reason": err.to_string()}),
    ));
    (false, Some(err.to_string()))
}

/// `POST /sessions/:id/input`: the run endpoint driving `sakha-agent`.
/// Appends the user input as a turn, publishes the lifecycle events a real
/// run would emit, and attempts to drive the turn via the agent loop.
async fn session_input(
    State(state): State<DaemonState>,
    Path(id): Path<String>,
    Json(req): Json<SessionInputRequest>,
) -> Result<Json<SessionInputResponse>, ApiError> {
    let session_id = parse_session_id(&id)?;
    state
        .sessions
        .get_session(session_id)
        .await
        .map_err(|e| ApiError::new("lookup_failed", e.to_string()))?
        .ok_or_else(|| ApiError::new("not_found", "session not found"))?;

    let turn_id = TurnId::new();
    state
        .sessions
        .append_turn(TurnRecord {
            id: turn_id,
            session_id,
            input_text: req.text.clone(),
            output_text: None,
            created_at: sakha_core::time::now_utc(),
        })
        .await
        .map_err(|e| ApiError::new("turn_append_failed", e.to_string()))?;

    let (driven, note) = drive_turn(&state, session_id, turn_id, &req.text).await;

    Ok(Json(SessionInputResponse { turn_id: turn_id.to_string(), session_id: session_id.to_string(), driven, note }))
}

#[derive(Debug, Deserialize, Default)]
pub struct SessionCancelRequest {
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SessionCancelResponse {
    pub session_id: String,
    pub status: String,
}

/// `POST /sessions/:id/cancel`.
async fn session_cancel(
    State(state): State<DaemonState>,
    Path(id): Path<String>,
    body: Option<Json<SessionCancelRequest>>,
) -> Result<Json<SessionCancelResponse>, ApiError> {
    let session_id = parse_session_id(&id)?;
    state
        .sessions
        .get_session(session_id)
        .await
        .map_err(|e| ApiError::new("lookup_failed", e.to_string()))?
        .ok_or_else(|| ApiError::new("not_found", "session not found"))?;

    let reason = body.and_then(|b| b.0.reason).unwrap_or_else(|| "cancelled by caller".to_string());

    state
        .sessions
        .update_session_status(session_id, SessionStatus::Blocked)
        .await
        .map_err(|e| ApiError::new("cancel_failed", e.to_string()))?;

    state.emit(EventEnvelope::for_session(session_id, EventKind::SessionBlocked, json!({"reason": reason})));

    Ok(Json(SessionCancelResponse { session_id: session_id.to_string(), status: "blocked".to_string() }))
}

#[derive(Debug, Serialize)]
pub struct TurnDto {
    pub id: String,
    pub input_text: String,
    pub output_text: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<TurnRecord> for TurnDto {
    fn from(t: TurnRecord) -> Self {
        Self { id: t.id.to_string(), input_text: t.input_text, output_text: t.output_text, created_at: t.created_at }
    }
}

/// `GET /sessions/:id/turns`: not in the module-19 core endpoint list by
/// name, but needed for any UI to render session history; kept additive.
async fn list_turns(State(state): State<DaemonState>, Path(id): Path<String>) -> Result<Json<Vec<TurnDto>>, ApiError> {
    let session_id = parse_session_id(&id)?;
    let turns = state
        .sessions
        .list_turns(session_id)
        .await
        .map_err(|e| ApiError::new("lookup_failed", e.to_string()))?;
    Ok(Json(turns.into_iter().map(TurnDto::from).collect()))
}

pub fn router() -> Router<DaemonState> {
    Router::new()
        .route("/sessions", post(create_session))
        .route("/sessions/:id", get(get_session))
        .route("/sessions/:id/input", post(session_input))
        .route("/sessions/:id/cancel", post(session_cancel))
        .route("/sessions/:id/turns", get(list_turns))
}
