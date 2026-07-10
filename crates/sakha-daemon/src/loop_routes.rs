//! Loop REST routes: `GET /loops`, `POST /loops`, plus pause/resume/stop.
//! See spec `modules/19-api-contracts.md`.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use sakha_loop::{LoopController, LoopKind, LoopSpec, StopReason, Trigger};

use crate::api::ApiError;
use crate::state::DaemonState;

#[derive(Debug, Serialize)]
pub struct LoopSpecDto {
    pub id: String,
    pub objective: String,
    pub loop_kind: String,
    pub max_iterations: u32,
}

impl From<LoopSpec> for LoopSpecDto {
    fn from(spec: LoopSpec) -> Self {
        Self {
            id: spec.id.to_string(),
            objective: spec.objective,
            loop_kind: format!("{:?}", spec.loop_kind),
            max_iterations: spec.max_iterations,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateLoopRequest {
    pub objective: String,
    #[serde(default)]
    pub loop_kind: Option<String>,
    #[serde(default)]
    pub max_iterations: Option<u32>,
}

fn parse_loop_kind(raw: Option<&str>) -> LoopKind {
    match raw {
        Some("verification") => LoopKind::Verification,
        Some("event") => LoopKind::Event,
        Some("research") => LoopKind::Research,
        Some("feedback") => LoopKind::Feedback,
        Some("replay") => LoopKind::Replay,
        _ => LoopKind::Agent,
    }
}

async fn create_loop(
    State(state): State<DaemonState>,
    Json(req): Json<CreateLoopRequest>,
) -> Result<Json<LoopSpecDto>, ApiError> {
    let mut spec = LoopSpec::new(req.objective.clone(), parse_loop_kind(req.loop_kind.as_deref()), Trigger::Manual);
    if let Some(max_iterations) = req.max_iterations {
        spec.max_iterations = max_iterations;
    }
    let dto = LoopSpecDto::from(spec.clone());

    state
        .loops
        .create_loop(spec)
        .await
        .map_err(|e| ApiError::new("loop_create_failed", e.to_string()))?;

    Ok(Json(dto))
}

/// `GET /loops`. Note: `LoopRuntime` (the underlying `LoopController`
/// implementation) does not yet expose a listing method beyond
/// create/tick/pause/resume/stop/detect_stall, so this intentionally returns
/// an empty list rather than reaching into the runtime's private state; once
/// `sakha-loop` grows a `list()` method this becomes a one-line change.
async fn list_loops() -> Json<Vec<LoopSpecDto>> {
    Json(Vec::new())
}

fn parse_loop_id(id: &str) -> Result<sakha_core::LoopId, ApiError> {
    id.parse().map_err(|_| ApiError::new("invalid_id", "malformed loop id"))
}

#[derive(Debug, Serialize)]
pub struct LoopActionResponse {
    pub id: String,
    pub state: String,
}

async fn pause_loop(State(state): State<DaemonState>, Path(id): Path<String>) -> Result<Json<LoopActionResponse>, ApiError> {
    let loop_id = parse_loop_id(&id)?;
    state.loops.pause(loop_id).await.map_err(|e| ApiError::new("loop_pause_failed", e.to_string()))?;
    Ok(Json(LoopActionResponse { id: loop_id.to_string(), state: "paused".into() }))
}

async fn resume_loop(State(state): State<DaemonState>, Path(id): Path<String>) -> Result<Json<LoopActionResponse>, ApiError> {
    let loop_id = parse_loop_id(&id)?;
    state.loops.resume(loop_id).await.map_err(|e| ApiError::new("loop_resume_failed", e.to_string()))?;
    Ok(Json(LoopActionResponse { id: loop_id.to_string(), state: "running".into() }))
}

#[derive(Debug, Deserialize, Default)]
pub struct StopLoopRequest {
    pub reason: Option<String>,
}

async fn stop_loop(
    State(state): State<DaemonState>,
    Path(id): Path<String>,
    body: Option<Json<StopLoopRequest>>,
) -> Result<Json<LoopActionResponse>, ApiError> {
    let loop_id = parse_loop_id(&id)?;
    let reason = body.and_then(|b| b.0.reason).unwrap_or_else(|| "stopped by caller".to_string());
    state
        .loops
        .stop(loop_id, StopReason(reason))
        .await
        .map_err(|e| ApiError::new("loop_stop_failed", e.to_string()))?;
    Ok(Json(LoopActionResponse { id: loop_id.to_string(), state: "stopped".into() }))
}

pub fn router() -> Router<DaemonState> {
    Router::new()
        .route("/loops", get(list_loops).post(create_loop))
        .route("/loops/:id/pause", post(pause_loop))
        .route("/loops/:id/resume", post(resume_loop))
        .route("/loops/:id/stop", post(stop_loop))
}
