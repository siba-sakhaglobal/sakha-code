//! Compression stats route: `GET /compression/stats`. See spec
//! `modules/19-api-contracts.md` "Core Endpoints" and `CompressionStatsDto`.
//!
//! Reads the daemon-wide `StatsRecorder` (shared with any `ContextCompressor`
//! the agent/research loops construct, per `DaemonState::compression_stats`)
//! and reports the `Global` scope by default, or a specific session/loop
//! scope via query params.

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use sakha_compression::CompressionScope;

use crate::api::ApiError;
use crate::state::DaemonState;

#[derive(Debug, Serialize, Deserialize)]
pub struct CompressionStatsDto {
    pub scope: String,
    pub items_compressed: u64,
    pub items_bypassed: u64,
    pub raw_tokens: u64,
    pub compressed_tokens: u64,
    pub tokens_saved: u64,
    pub compression_ratio: f64,
    pub retrieval_count: u64,
    pub retrieval_rate: f64,
    pub over_compression_retries: u64,
}

fn to_dto(scope_label: String, stats: sakha_compression::CompressionStats) -> CompressionStatsDto {
    CompressionStatsDto {
        scope: scope_label,
        items_compressed: stats.items_compressed,
        items_bypassed: stats.items_bypassed,
        raw_tokens: stats.raw_tokens,
        compressed_tokens: stats.compressed_tokens,
        tokens_saved: stats.tokens_saved(),
        compression_ratio: stats.compression_ratio(),
        retrieval_count: stats.retrieval_count,
        retrieval_rate: stats.retrieval_rate(),
        over_compression_retries: stats.over_compression_retries,
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct CompressionStatsQuery {
    /// Optional scope selector: `"session:<id>"`, `"loop:<id>"`,
    /// `"turn:<id>"`, or omitted/`"global"` for the aggregate across every
    /// scope tracked so far.
    #[serde(default)]
    pub scope: Option<String>,
}

fn parse_scope(raw: &str) -> Result<CompressionScope, ApiError> {
    if raw.eq_ignore_ascii_case("global") {
        return Ok(CompressionScope::Global);
    }
    let (kind, id) = raw.split_once(':').ok_or_else(|| {
        ApiError::new("invalid_scope", "scope must be 'global' or '<kind>:<id>' (session/turn/loop)")
    })?;
    match kind {
        "session" => id
            .parse()
            .map(CompressionScope::Session)
            .map_err(|_| ApiError::new("invalid_scope", "malformed session id in scope")),
        "turn" => id
            .parse()
            .map(CompressionScope::Turn)
            .map_err(|_| ApiError::new("invalid_scope", "malformed turn id in scope")),
        "loop" => id
            .parse()
            .map(CompressionScope::Loop)
            .map_err(|_| ApiError::new("invalid_scope", "malformed loop id in scope")),
        other => Err(ApiError::new("invalid_scope", format!("unknown scope kind: {other}"))),
    }
}

/// `GET /compression/stats`: returns the requested scope's compression
/// stats, defaulting to the `Global` aggregate.
async fn get_compression_stats(
    State(state): State<DaemonState>,
    Query(query): Query<CompressionStatsQuery>,
) -> Result<Json<CompressionStatsDto>, ApiError> {
    let scope = match query.scope {
        Some(raw) => parse_scope(&raw)?,
        None => CompressionScope::Global,
    };
    let label = query_label(&scope);
    let stats = state.compression_stats.get(&scope);
    Ok(Json(to_dto(label, stats)))
}

fn query_label(scope: &CompressionScope) -> String {
    match scope {
        CompressionScope::Global => "global".to_string(),
        CompressionScope::Session(id) => format!("session:{id}"),
        CompressionScope::Turn(id) => format!("turn:{id}"),
        CompressionScope::Loop(id) => format!("loop:{id}"),
    }
}

pub fn router() -> Router<DaemonState> {
    Router::new().route("/compression/stats", get(get_compression_stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn global_stats_default_to_zeroed_when_nothing_recorded() {
        let state = DaemonState::new();
        let app = crate::api::build_router(state);
        let response = app.oneshot(Request::builder().uri("/compression/stats").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn stats_reflect_recorded_compression() {
        let state = DaemonState::new();
        state.compression_stats.record_compression(CompressionScope::Global, 1000, 100, false);
        let app = crate::api::build_router(state);
        let response = app.oneshot(Request::builder().uri("/compression/stats").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let dto: CompressionStatsDto = serde_json::from_slice(&body).unwrap();
        assert_eq!(dto.items_compressed, 1);
        assert_eq!(dto.raw_tokens, 1000);
    }

    #[tokio::test]
    async fn invalid_scope_is_bad_request() {
        let state = DaemonState::new();
        let app = crate::api::build_router(state);
        let response = app
            .oneshot(Request::builder().uri("/compression/stats?scope=bogus").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
