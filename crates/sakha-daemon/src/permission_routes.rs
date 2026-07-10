//! Permission routes: `GET /permissions` (list pending), and
//! `POST /permissions/:id/decision`. See spec `modules/19-api-contracts.md`
//! "Core Endpoints" and "Tests" -> "Permission decision". Backed by
//! `crate::permission_queue::PermissionQueue`, the pending-permission queue
//! the agent loop awaits on before proceeding with a gated tool call.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use sakha_security::PermissionDecision;

use crate::api::ApiError;
use crate::permission_queue::PermissionRequestId;
use crate::state::DaemonState;

#[derive(Debug, Serialize)]
pub struct PendingPermissionDto {
    pub id: String,
    pub session_id: Option<String>,
    pub kind: String,
    pub subject: String,
    pub reason: String,
    pub tool_name: Option<String>,
}

impl From<crate::permission_queue::PendingPermission> for PendingPermissionDto {
    fn from(p: crate::permission_queue::PendingPermission) -> Self {
        Self {
            id: p.id.to_string(),
            session_id: p.session_id.map(|s| s.to_string()),
            kind: format!("{:?}", p.request.kind),
            subject: p.request.subject.clone(),
            reason: p.request.reason.clone(),
            tool_name: p.request.tool_name.clone(),
        }
    }
}

/// `GET /permissions`: lists every currently pending permission request
/// awaiting a decision.
async fn list_pending(State(state): State<DaemonState>) -> Json<Vec<PendingPermissionDto>> {
    Json(state.permissions.list_pending().into_iter().map(PendingPermissionDto::from).collect())
}

async fn get_pending(State(state): State<DaemonState>, Path(id): Path<String>) -> Result<Json<PendingPermissionDto>, ApiError> {
    let req_id: PermissionRequestId = id.parse().map_err(|_| ApiError::new("invalid_id", "malformed permission request id"))?;
    state
        .permissions
        .get(req_id)
        .map(|p| Json(p.into()))
        .ok_or_else(|| ApiError::new("not_found", "permission request not found or already decided"))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecisionKind {
    Allow,
    Deny,
}

#[derive(Debug, Deserialize)]
pub struct PermissionDecisionRequest {
    pub decision: PermissionDecisionKind,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PermissionDecisionResponse {
    pub id: String,
    pub decision: String,
}

/// `POST /permissions/:id/decision`: resolves a pending permission request,
/// waking the awaiting agent-loop task via the oneshot channel it was raised
/// with.
async fn decide_permission(
    State(state): State<DaemonState>,
    Path(id): Path<String>,
    Json(req): Json<PermissionDecisionRequest>,
) -> Result<Json<PermissionDecisionResponse>, ApiError> {
    let req_id: PermissionRequestId = id.parse().map_err(|_| ApiError::new("invalid_id", "malformed permission request id"))?;

    let pending = state
        .permissions
        .get(req_id)
        .ok_or_else(|| ApiError::new("not_found", "permission request not found or already decided"))?;

    let decision = match req.decision {
        PermissionDecisionKind::Allow => PermissionDecision::allowed(),
        PermissionDecisionKind::Deny => PermissionDecision::denied(req.reason.clone().unwrap_or_else(|| "denied by caller".to_string())),
    };

    state
        .permissions
        .decide(req_id, decision.clone())
        .map_err(|e| ApiError::new("decision_failed", e.to_string()))?;

    if let Some(session_id) = pending.session_id {
        state.emit(sakha_core::EventEnvelope::for_session(
            session_id,
            sakha_core::EventKind::PermissionRequested,
            serde_json::json!({
                "permission_id": req_id.to_string(),
                "decision": format!("{decision:?}"),
            }),
        ));
    }

    Ok(Json(PermissionDecisionResponse { id: req_id.to_string(), decision: format!("{decision:?}") }))
}

pub fn router() -> Router<DaemonState> {
    Router::new()
        .route("/permissions", get(list_pending))
        .route("/permissions/:id", get(get_pending))
        .route("/permissions/:id/decision", post(decide_permission))
}
