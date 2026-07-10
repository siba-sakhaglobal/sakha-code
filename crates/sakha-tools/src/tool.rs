//! The `Tool` trait and its supporting types. See `04-core-domain-model.md`
//! `Tool` and `modules/07-tool-system.md`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use sakha_core::{ArtifactRef, SakhaResult};
use sakha_security::{PermissionDecision, PermissionKind};

/// Newtype for a tool's registered name (e.g. `"file.read"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ToolName(pub String);

impl ToolName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

impl std::fmt::Display for ToolName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// JSON schema (subset) describing a tool's expected input shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInputSchema(pub serde_json::Value);

/// JSON schema describing a tool's output shape, for documentation/validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutputSchema(pub serde_json::Value);

/// Declares which permission categories a tool may need, so the executor can
/// pre-flight a permission check before `plan`/`execute`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolPermissionSpec {
    pub required: Vec<PermissionKind>,
}

/// Whether repeated calls with the same input are safe to dedupe/replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdempotencyPolicy {
    Idempotent,
    NotIdempotent,
    IdempotentWithKey,
}

/// A stable identity for deduplicating tool calls with identical intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdempotencyKey(pub String);

/// Static description of a tool, returned by `Tool::spec()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: ToolName,
    pub description: String,
    pub input_schema: ToolInputSchema,
    pub output_schema: ToolOutputSchema,
    pub permission_spec: ToolPermissionSpec,
    pub idempotency_policy: IdempotencyPolicy,
}

/// Input that has passed schema validation, ready for planning/execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatedInput(pub serde_json::Value);

/// A dry-run description of what a tool call would do, shown to a human
/// before approval for risky actions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolPlan {
    pub summary: String,
    pub affected_paths: Vec<String>,
    pub is_destructive: bool,
}

/// Status of a completed (or attempted) tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Denied,
    TimedOut,
}

/// The normalized result of executing a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub status: ToolCallStatus,
    pub output_json: serde_json::Value,
    pub raw_output_ref: Option<ArtifactRef>,
    pub error_message: Option<String>,
}

impl ToolResult {
    pub fn success(output_json: serde_json::Value) -> Self {
        Self {
            status: ToolCallStatus::Succeeded,
            output_json,
            raw_output_ref: None,
            error_message: None,
        }
    }

    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            status: ToolCallStatus::Failed,
            output_json: serde_json::Value::Null,
            raw_output_ref: None,
            error_message: Some(message.into()),
        }
    }
}

/// A short, model-friendly summary of a `ToolResult`, used to keep the
/// conversation compact when the full result is large.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSummary {
    pub text: String,
    pub truncated: bool,
}

/// A permission-check callback injected by the host (agent/CLI/daemon) into a
/// `ToolContext`. Given a permission kind + subject/reason, returns a
/// decision. This is the "permission bridge hook" described in
/// `modules/07-tool-system.md`: it lets the executor ask an out-of-crate
/// policy engine (interactive approval, `sakha-security::PermissionPolicy`,
/// or a test double) without `sakha-tools` depending on how that decision is
/// actually made.
pub type PermissionChecker = std::sync::Arc<
    dyn Fn(&crate::tool::PermissionCheckRequest) -> PermissionDecision + Send + Sync,
>;

/// The information passed to a `PermissionChecker` for one required
/// permission kind on one tool call.
#[derive(Debug, Clone)]
pub struct PermissionCheckRequest {
    pub kind: PermissionKind,
    pub tool_name: String,
    pub subject: String,
    pub reason: String,
}

/// Ambient context passed to tool planning/execution: workspace root,
/// permission decision already granted (if any), an optional permission
/// checker callback, and cancellation.
#[derive(Clone)]
pub struct ToolContext {
    pub workspace_root: std::path::PathBuf,
    pub permission_decision: Option<PermissionDecision>,
    /// When set, the executor's permission bridge calls this instead of (or
    /// in addition to) a static `PermissionPolicy`, so hosts can implement
    /// interactive approval, allow/deny lists, or test fixtures.
    pub permission_checker: Option<PermissionChecker>,
    pub cancellation: tokio_util::sync::CancellationToken,
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("workspace_root", &self.workspace_root)
            .field("permission_decision", &self.permission_decision)
            .field("permission_checker", &self.permission_checker.is_some())
            .finish()
    }
}

impl ToolContext {
    pub fn new(workspace_root: impl Into<std::path::PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            permission_decision: None,
            permission_checker: None,
            cancellation: tokio_util::sync::CancellationToken::new(),
        }
    }

    /// Attaches a permission checker callback, used by the executor's
    /// permission bridge in place of (or alongside) a static policy.
    pub fn with_permission_checker(mut self, checker: PermissionChecker) -> Self {
        self.permission_checker = Some(checker);
        self
    }
}

/// All tools implement this contract. See `04-core-domain-model.md` `Tool`.
#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;

    fn validate(&self, input: serde_json::Value) -> SakhaResult<ValidatedInput>;

    async fn plan(&self, input: &ValidatedInput, context: &ToolContext) -> SakhaResult<ToolPlan>;

    async fn execute(&self, input: &ValidatedInput, context: &ToolContext) -> SakhaResult<ToolResult>;

    fn summarize(&self, result: &ToolResult) -> ToolSummary;
}
