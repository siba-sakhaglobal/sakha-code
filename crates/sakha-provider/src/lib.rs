//! sakha-provider: LLM provider gateway (OpenAI-compatible adapters, streaming, tool-call normalization).
//!
//! Public API skeleton — see spec `modules/02-llm-provider-gateway.md` and
//! `crates/crate-work-breakdown.md`. Key contract: `ProviderClient::stream`
//! yields `ModelEvent`s over a bounded `tokio::sync::mpsc` channel wrapped as
//! a `Stream` (`ModelEventStream`).

pub mod capabilities;
pub mod client;
pub mod config;
pub mod cost;
pub mod openai_compatible;
pub mod sse;
pub mod tool_calls;

pub use capabilities::{CapabilityRegistry, ModelProfile, ProviderCapabilities};
pub use client::{
    MessageRole, MockProviderClient, ModelEvent, ModelEventStream, ModelMessage, ModelRequest,
    ModelResponse, NullProviderClient, ProviderClient, ProviderHealth, StopReason,
    TokenCountRequest, TokenCountResult, ToolDefinition,
};
pub use config::{load_profiles_from_toml, well_known, ModelTier, ProviderKind, ProviderProfile};
pub use cost::{estimate_cost_micros, UsageRecord};
pub use openai_compatible::{normalize_error, OpenAiCompatibleClient};
pub use sse::{SseEvent, SseParser};
pub use tool_calls::{AssembledToolCall, ToolCallAssembler, ToolCallDelta};

pub fn crate_name() -> &'static str {
    "sakha-provider"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn null_provider_client_reports_unavailable_health() {
        let client = NullProviderClient;
        let health = client.health().await.unwrap();
        assert_eq!(health, ProviderHealth::Unavailable);
    }

    #[tokio::test]
    async fn null_provider_client_stream_yields_not_implemented_error() {
        use tokio_stream::StreamExt;
        let client = NullProviderClient;
        let mut stream = client.stream(ModelRequest::new("test-model")).await.unwrap();
        let first = stream.next().await.expect("one event");
        assert!(first.is_err());
    }
}
