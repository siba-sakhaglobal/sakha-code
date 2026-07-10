//! Source attribution: tracks where each piece of assembled context came
//! from, for audit and for the compression retrieval path.

use serde::{Deserialize, Serialize};

use sakha_core::ArtifactRef;

/// Where a context item originated (mirrors `ContextSource` referenced by
/// `04-core-domain-model.md` `ContextItem.source`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ContextSource {
    ConversationHistory,
    ToolOutput { tool_call_id: String },
    Memory { record_id: String },
    Research { url: String },
    Handoff,
    UserInput,
}

/// A trust level for content pulled into context, used to decide how much
/// scrutiny (e.g. injection filtering) to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    Untrusted,
    LowTrust,
    Trusted,
    System,
}

/// Attributes one section of assembled context back to its source and raw
/// artifact, for audit trails.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributionRecord {
    pub source: ContextSource,
    pub trust_level: TrustLevel,
    pub raw_artifact: Option<ArtifactRef>,
}
