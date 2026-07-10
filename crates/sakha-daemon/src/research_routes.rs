//! Research evidence route: `GET /research/:id/evidence`. See spec
//! `modules/19-api-contracts.md` "Core Endpoints" and `EvidencePackDto`.
//!
//! Research loops (see `sakha-research::EvidencePack`) build a citable
//! answer for a question; the daemon needs somewhere to park the finished
//! pack so a UI/CLI polling `GET /research/:id/evidence` can fetch it after
//! the loop completes. `EvidenceStore` is that in-memory home, keyed by an
//! opaque research-run id (typically a `LoopId` or `GoalId` string, whatever
//! the caller used when the pack was stored) — mirrors how `goal_routes`
//! keeps its own `GoalStore` until a durable home exists.

use std::sync::Mutex;

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use sakha_research::EvidencePack;

use crate::api::ApiError;
use crate::state::DaemonState;

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationDto {
    pub url: String,
    pub quote: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvidencePackDto {
    pub research_id: String,
    pub question: String,
    pub answer: String,
    pub citations: Vec<CitationDto>,
    pub grounded: bool,
    pub built_at: chrono::DateTime<chrono::Utc>,
}

impl EvidencePackDto {
    fn from_pack(research_id: String, pack: &EvidencePack) -> Self {
        Self {
            research_id,
            question: pack.question.clone(),
            answer: pack.answer.clone(),
            citations: pack.citations.iter().map(|c| CitationDto { url: c.url.clone(), quote: c.quote.clone() }).collect(),
            grounded: pack.is_grounded(),
            built_at: pack.built_at,
        }
    }
}

/// In-memory store of finished evidence packs, keyed by research-run id.
#[derive(Default)]
pub struct EvidenceStore {
    packs: Mutex<std::collections::HashMap<String, EvidencePack>>,
}

impl EvidenceStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records/overwrites the evidence pack for `research_id`. Called by
    /// whatever drives a research loop to completion (loop runtime today;
    /// future callers as the research loop wiring lands).
    pub fn put(&self, research_id: impl Into<String>, pack: EvidencePack) {
        self.packs.lock().unwrap().insert(research_id.into(), pack);
    }

    pub fn get(&self, research_id: &str) -> Option<EvidencePack> {
        self.packs.lock().unwrap().get(research_id).cloned()
    }
}

/// `GET /research/:id/evidence`.
async fn get_research_evidence(
    State(state): State<DaemonState>,
    Path(id): Path<String>,
) -> Result<Json<EvidencePackDto>, ApiError> {
    let pack = state
        .research_evidence
        .get(&id)
        .ok_or_else(|| ApiError::new("not_found", "no evidence pack recorded for this research id"))?;
    Ok(Json(EvidencePackDto::from_pack(id, &pack)))
}

pub fn router() -> Router<DaemonState> {
    Router::new().route("/research/:id/evidence", get(get_research_evidence))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn missing_evidence_pack_is_not_found() {
        let state = DaemonState::new();
        let app = crate::api::build_router(state);
        let response =
            app.oneshot(Request::builder().uri("/research/unknown-id/evidence").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn stored_evidence_pack_is_returned() {
        let state = DaemonState::new();
        let mut pack = EvidencePack::new("what is rust?").with_answer("Rust is a systems language.");
        pack.add_citation(sakha_research::Citation::new(
            "https://rust-lang.org",
            "Rust is a systems programming language",
            Default::default(),
        ));
        state.research_evidence.put("run-1", pack);

        let app = crate::api::build_router(state);
        let response = app.oneshot(Request::builder().uri("/research/run-1/evidence").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let dto: EvidencePackDto = serde_json::from_slice(&body).unwrap();
        assert_eq!(dto.research_id, "run-1");
        assert!(dto.grounded);
        assert_eq!(dto.citations.len(), 1);
    }
}
