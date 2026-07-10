//! Usage and cost accounting for provider requests.

use serde::{Deserialize, Serialize};

/// Token/cost usage for a single model request, in the units the provider
/// returned plus a normalized micro-USD cost estimate.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct UsageRecord {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    /// Cost expressed in micro-USD (1e-6 USD) to avoid floating point drift,
    /// consistent with `sakha_core::Budget::max_cost_micros`.
    pub cost_micros: u64,
}

impl UsageRecord {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }

    pub fn merge(mut self, other: UsageRecord) -> Self {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cached_input_tokens += other.cached_input_tokens;
        self.cost_micros += other.cost_micros;
        self
    }
}

/// Estimates cost in micro-USD from usage and per-1k-token pricing.
pub fn estimate_cost_micros(usage: &UsageRecord, input_cost_micros_per_1k: u64, output_cost_micros_per_1k: u64) -> u64 {
    let input = (usage.input_tokens * input_cost_micros_per_1k) / 1000;
    let output = (usage.output_tokens * output_cost_micros_per_1k) / 1000;
    input + output
}
