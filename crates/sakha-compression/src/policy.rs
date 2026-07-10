//! Compression policy fields. See `modules/05-context-compression-headroom.md`
//! "Policy Fields" and "Content Kinds".

use serde::{Deserialize, Serialize};

/// Classifies the kind of content a `ContextItem` holds, driving compression
/// routing decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    ConversationHistory,
    ToolOutput,
    ShellLog,
    BuildLog,
    TestLog,
    JsonData,
    CodeFile,
    FileTree,
    SearchResult,
    WebPage,
    RagChunk,
    ErrorTrace,
    Plan,
    Handoff,
}

/// Which route a `ContextItem` should go through for compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompressionRoute {
    HeadroomLibrary,
    HeadroomProxy,
    HeadroomMcp,
    HeadroomSidecar,
    LocalFallback,
    Bypass,
}

/// Whether Headroom (or a fallback) should fail open (pass content through
/// uncompressed) or fail closed (block the request) when unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailPolicy {
    FailOpen,
    FailClosed,
}

/// Policy controlling how/when compression is applied. See spec "Policy Fields".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionPolicy {
    pub enabled: bool,
    pub mode: CompressionRoute,
    pub min_tokens_to_compress: u64,
    pub max_lossiness: f32,
    pub reversible_required: bool,
    pub allowed_content_kinds: Vec<ContentKind>,
    pub blocked_content_kinds: Vec<ContentKind>,
    pub retrieve_on_model_request: bool,
    pub store_raw_for_days: u32,
    pub redact_before_compress: bool,
    pub fail_open_or_closed: FailPolicy,
}

impl Default for CompressionPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: CompressionRoute::LocalFallback,
            min_tokens_to_compress: 500,
            max_lossiness: 0.2,
            reversible_required: true,
            allowed_content_kinds: Vec::new(),
            blocked_content_kinds: Vec::new(),
            retrieve_on_model_request: true,
            store_raw_for_days: 30,
            redact_before_compress: true,
            fail_open_or_closed: FailPolicy::FailOpen,
        }
    }
}

impl CompressionPolicy {
    /// Whether `kind` is permitted to be compressed under this policy.
    pub fn allows(&self, kind: ContentKind) -> bool {
        if self.blocked_content_kinds.contains(&kind) {
            return false;
        }
        self.allowed_content_kinds.is_empty() || self.allowed_content_kinds.contains(&kind)
    }
}

/// The outcome of a routing decision for one `ContextItem`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionDecision {
    pub route: CompressionRoute,
    pub should_compress: bool,
    pub reason: String,
}

impl CompressionPolicy {
    /// Decides whether/how `kind` content of `estimated_tokens` should be
    /// compressed, without performing any I/O. Pure routing logic so it's
    /// trivially unit-testable (spec "Tests": policy routing).
    pub fn decide(&self, kind: ContentKind, estimated_tokens: u64) -> CompressionDecision {
        if !self.enabled {
            return CompressionDecision {
                route: CompressionRoute::Bypass,
                should_compress: false,
                reason: "compression disabled by policy".into(),
            };
        }
        if !self.allows(kind) {
            return CompressionDecision {
                route: CompressionRoute::Bypass,
                should_compress: false,
                reason: format!("content kind {kind:?} is blocked or not allow-listed"),
            };
        }
        if estimated_tokens < self.min_tokens_to_compress {
            return CompressionDecision {
                route: CompressionRoute::Bypass,
                should_compress: false,
                reason: format!(
                    "estimated {estimated_tokens} tokens below min_tokens_to_compress {}",
                    self.min_tokens_to_compress
                ),
            };
        }
        CompressionDecision {
            route: self.mode,
            should_compress: true,
            reason: format!("routed via {:?}", self.mode),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decide_bypasses_when_disabled() {
        let mut policy = CompressionPolicy::default();
        policy.enabled = false;
        let decision = policy.decide(ContentKind::ToolOutput, 10_000);
        assert!(!decision.should_compress);
        assert_eq!(decision.route, CompressionRoute::Bypass);
    }

    #[test]
    fn decide_bypasses_blocked_content_kind() {
        let mut policy = CompressionPolicy::default();
        policy.blocked_content_kinds.push(ContentKind::ErrorTrace);
        let decision = policy.decide(ContentKind::ErrorTrace, 10_000);
        assert!(!decision.should_compress);
    }

    #[test]
    fn decide_bypasses_below_min_tokens() {
        let policy = CompressionPolicy::default();
        let decision = policy.decide(ContentKind::ToolOutput, 10);
        assert!(!decision.should_compress);
        assert_eq!(decision.route, CompressionRoute::Bypass);
    }

    #[test]
    fn decide_routes_via_policy_mode_when_above_threshold() {
        let policy = CompressionPolicy::default();
        let decision = policy.decide(ContentKind::ToolOutput, 10_000);
        assert!(decision.should_compress);
        assert_eq!(decision.route, CompressionRoute::LocalFallback);
    }
}
