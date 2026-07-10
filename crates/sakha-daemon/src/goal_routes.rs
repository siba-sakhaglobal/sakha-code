//! Goal REST routes: `GET /goals`, `POST /goals`. See spec
//! `modules/19-api-contracts.md` "Core Endpoints".
//!
//! The full `Goal` domain type from `04-core-domain-model.md` (`plan`,
//! `budget`, `acceptance_criteria`, `handoff_artifact`, `GoalState`) has no
//! concrete Rust type anywhere in the workspace yet (no `sakha-*` crate
//! defines `Plan`, `Budget`, `Criterion`, or `GoalState`) — building that
//! whole subsystem is out of scope for wiring up these two endpoints. This
//! module implements the honest subset the daemon can actually back today
//! (id/title/objective/timestamps) behind an in-memory `GoalStore`, mirroring
//! how `session_routes`/`loop_routes` are structured, so the endpoints exist
//! and are real (not stubs returning `not_implemented`) rather than papering
//! over the missing domain type with fabricated fields.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use sakha_core::GoalId;

use crate::api::ApiError;
use crate::state::DaemonState;

/// One tracked goal.
#[derive(Debug, Clone, Serialize)]
pub struct GoalDto {
    pub id: String,
    pub title: String,
    pub objective: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct GoalRecord {
    pub id: GoalId,
    pub title: String,
    pub objective: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<GoalRecord> for GoalDto {
    fn from(record: GoalRecord) -> Self {
        Self { id: record.id.to_string(), title: record.title, objective: record.objective, created_at: record.created_at }
    }
}

/// In-memory store of tracked goals. Kept in the daemon (rather than
/// `sakha-memory`) until the full `Goal` domain type lands, at which point
/// this should move to a `GoalStore` trait alongside `SessionStore`.
#[derive(Default)]
pub struct GoalStore {
    goals: std::sync::Mutex<Vec<GoalRecord>>,
}

impl GoalStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(&self, title: String, objective: String) -> GoalRecord {
        let record = GoalRecord { id: GoalId::new(), title, objective, created_at: sakha_core::time::now_utc() };
        self.goals.lock().unwrap().push(record.clone());
        record
    }

    pub fn list(&self) -> Vec<GoalRecord> {
        self.goals.lock().unwrap().clone()
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateGoalRequest {
    pub title: String,
    #[serde(default)]
    pub objective: Option<String>,
}

/// `GET /goals`.
async fn list_goals(State(state): State<DaemonState>) -> Json<Vec<GoalDto>> {
    Json(state.goals.list().into_iter().map(GoalDto::from).collect())
}

/// `POST /goals`.
async fn create_goal(
    State(state): State<DaemonState>,
    Json(req): Json<CreateGoalRequest>,
) -> Result<Json<GoalDto>, ApiError> {
    if req.title.trim().is_empty() {
        return Err(ApiError::new("invalid_title", "goal title must not be empty"));
    }
    let objective = req.objective.unwrap_or_else(|| req.title.clone());
    let record = state.goals.create(req.title, objective);
    Ok(Json(record.into()))
}

pub fn router() -> Router<DaemonState> {
    Router::new().route("/goals", get(list_goals).post(create_goal))
}

pub type SharedGoalStore = Arc<GoalStore>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_create_then_list_round_trips() {
        let store = GoalStore::new();
        let created = store.create("Ship feature X".into(), "Ship feature X end to end".into());
        let listed = store.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);
        assert_eq!(listed[0].title, "Ship feature X");
    }
}
