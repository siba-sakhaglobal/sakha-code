//! Tool preview route: `POST /tools/:name/preview`. See spec
//! `modules/19-api-contracts.md` "Core Endpoints" and
//! `modules/07-tool-system.md` (`Tool::plan` dry-run description).
//!
//! Runs a tool's `validate` -> `plan` steps (never `execute`) so a caller can
//! see what a tool call *would* do — affected paths, whether it's
//! destructive — before approving it. This mirrors the "dry-run description"
//! contract `ToolPlan` already exists for; the daemon just needed to expose
//! it over HTTP.

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use sakha_tools::{ToolContext, ToolName, ToolPlan};

use crate::api::ApiError;
use crate::state::DaemonState;

#[derive(Debug, Deserialize, Default)]
pub struct ToolPreviewRequest {
    #[serde(default)]
    pub input: serde_json::Value,
    /// Workspace root the tool would operate against. Defaults to the
    /// current process working directory when omitted (preview only; no
    /// writes happen regardless).
    #[serde(default)]
    pub workspace_root: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ToolPreviewResponse {
    pub tool: String,
    pub summary: String,
    pub affected_paths: Vec<String>,
    pub is_destructive: bool,
}

impl ToolPreviewResponse {
    fn from_plan(tool: &str, plan: ToolPlan) -> Self {
        Self { tool: tool.to_string(), summary: plan.summary, affected_paths: plan.affected_paths, is_destructive: plan.is_destructive }
    }
}

/// `POST /tools/:name/preview`: validates `input` against the named tool's
/// schema then calls `Tool::plan` to produce a dry-run description, without
/// ever calling `Tool::execute`. Unknown tool names or invalid input surface
/// as `ApiError`s (`not_found` / `invalid_input`) rather than a panic.
async fn preview_tool(
    State(state): State<DaemonState>,
    Path(name): Path<String>,
    body: Option<Json<ToolPreviewRequest>>,
) -> Result<Json<ToolPreviewResponse>, ApiError> {
    let req = body.map(|b| b.0).unwrap_or_default();
    let tool_name = ToolName::new(name.clone());
    let tool = state
        .tools
        .get(&tool_name)
        .ok_or_else(|| ApiError::new("not_found", format!("unknown tool: {name}")))?;

    let validated = tool.validate(req.input).map_err(|e| ApiError::new("invalid_input", e.to_string()))?;

    let workspace_root = req.workspace_root.unwrap_or_else(|| ".".to_string());
    let context = ToolContext::new(workspace_root);

    let plan = tool.plan(&validated, &context).await.map_err(|e| ApiError::new("preview_failed", e.to_string()))?;

    Ok(Json(ToolPreviewResponse::from_plan(&name, plan)))
}

pub fn router() -> Router<DaemonState> {
    Router::new().route("/tools/:name/preview", post(preview_tool))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn app() -> axum::Router {
        crate::api::build_router(DaemonState::new())
    }

    #[tokio::test]
    async fn preview_unknown_tool_is_not_found() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/tools/does.not.exist/preview")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn preview_file_read_returns_plan() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/tools/file.read/preview")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::json!({"input": {"path": "Cargo.toml"}}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
