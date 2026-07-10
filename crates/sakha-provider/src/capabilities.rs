//! Provider/model capability registry. See spec `04-core-domain-model.md`
//! `ProviderClient::capabilities()` and `modules/02-llm-provider-gateway.md`.

use serde::{Deserialize, Serialize};

/// What a given provider/model combination supports. Drives request shaping
/// (e.g. whether to send `tools`, `response_format`, `reasoning_effort`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub supports_tools: bool,
    pub supports_json_schema: bool,
    pub supports_streaming: bool,
    pub supports_streaming_usage: bool,
    pub supports_reasoning_effort: bool,
    pub supports_parallel_tool_calls: bool,
    pub max_context_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
}

/// Metadata about a single model exposed by a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProfile {
    pub id: String,
    pub display_name: String,
    pub capabilities: ProviderCapabilities,
    pub input_cost_micros_per_1k: Option<u64>,
    pub output_cost_micros_per_1k: Option<u64>,
}

/// A simple in-memory registry of known model capabilities, keyed by model id.
#[derive(Debug, Default)]
pub struct CapabilityRegistry {
    profiles: std::collections::HashMap<String, ModelProfile>,
}

impl CapabilityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, profile: ModelProfile) {
        self.profiles.insert(profile.id.clone(), profile);
    }

    pub fn get(&self, model_id: &str) -> Option<&ModelProfile> {
        self.profiles.get(model_id)
    }
}
