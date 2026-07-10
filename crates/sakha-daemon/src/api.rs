//! Top-level API error type and router assembly. See spec
//! `modules/19-api-contracts.md` "Core Endpoints".

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::state::DaemonState;

/// A normalized API error response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into() }
    }

    /// The HTTP status this error code maps to. `not_found` -> 404,
    /// `invalid_*`/`malformed_*` -> 400, everything else -> 500, matching
    /// spec "API rejects invalid loop spec" (a client error, not a crash).
    fn status(&self) -> StatusCode {
        if self.code == "not_found" {
            StatusCode::NOT_FOUND
        } else if self.code.starts_with("invalid_") || self.code.contains("malformed") {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        (status, axum::Json(self)).into_response()
    }
}

async fn health() -> &'static str {
    "ok"
}

#[derive(Debug, Serialize)]
pub struct AuditEventDto {
    pub sequence: u64,
    pub session_id: Option<String>,
    pub kind: String,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    pub payload: serde_json::Value,
}

impl From<sakha_core::EventEnvelope> for AuditEventDto {
    fn from(e: sakha_core::EventEnvelope) -> Self {
        Self {
            sequence: e.sequence,
            session_id: e.session_id.map(|s| s.to_string()),
            kind: format!("{:?}", e.kind),
            occurred_at: e.occurred_at,
            payload: e.payload,
        }
    }
}

/// `GET /audit`: returns every audited event across all sessions, in append
/// order. See spec `modules/19-api-contracts.md` "Core Endpoints".
async fn get_audit(State(state): State<DaemonState>) -> Json<Vec<AuditEventDto>> {
    Json(state.audit.all_events().into_iter().map(AuditEventDto::from).collect())
}

/// `GET /sessions/:id/audit`: audit trail scoped to one session.
async fn get_session_audit(State(state): State<DaemonState>, Path(id): Path<String>) -> Result<Json<Vec<AuditEventDto>>, ApiError> {
    let session_id: sakha_core::SessionId = id.parse().map_err(|_| ApiError::new("invalid_id", "malformed session id"))?;
    Ok(Json(state.audit.events_for_session(session_id).into_iter().map(AuditEventDto::from).collect()))
}

/// Builds the full axum router: health, session, loop, permission, SSE, and
/// WebSocket routes. Binding to `127.0.0.1` only is enforced by the caller
/// (`main.rs`); this function is transport-agnostic so it's reusable from
/// `tower::ServiceExt::oneshot` tests.
pub fn build_router(state: DaemonState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/audit", get(get_audit))
        .route("/sessions/:id/audit", get(get_session_audit))
        .merge(crate::session_routes::router())
        .merge(crate::loop_routes::router())
        .merge(crate::permission_routes::router())
        .merge(crate::sse::router())
        .merge(crate::ws::router())
        .with_state(state)
}
