//! The provider-neutral request/response/event model and the `ProviderClient`
//! trait every adapter implements. See `04-core-domain-model.md`
//! `ProviderClient` and `modules/02-llm-provider-gateway.md`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::ReceiverStream;

use sakha_core::{ModelRequestId, SakhaError, SakhaResult};

use crate::capabilities::ProviderCapabilities;
use crate::cost::UsageRecord;
use crate::tool_calls::ToolCallDelta;

/// A single message in a provider-neutral conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMessage {
    pub role: MessageRole,
    pub content: String,
    /// Present when `role == Tool`: which tool call this message answers.
    pub tool_call_id: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// A tool definition offered to the model, provider-neutral JSON schema form.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// A request to a model provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRequest {
    pub id: ModelRequestId,
    pub model: String,
    pub messages: Vec<ModelMessage>,
    pub tools: Vec<ToolDefinition>,
    pub max_output_tokens: Option<u64>,
    pub temperature: Option<f32>,
    pub response_json_schema: Option<serde_json::Value>,
    pub reasoning_effort: Option<String>,
    pub stream: bool,
}

impl ModelRequest {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            id: ModelRequestId::new(),
            model: model.into(),
            messages: Vec::new(),
            tools: Vec::new(),
            max_output_tokens: None,
            temperature: None,
            response_json_schema: None,
            reasoning_effort: None,
            stream: true,
        }
    }
}

/// A complete (non-streaming) model response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelResponse {
    pub request_id: ModelRequestId,
    pub text: String,
    pub tool_calls: Vec<crate::tool_calls::AssembledToolCall>,
    pub usage: UsageRecord,
    pub stop_reason: StopReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    ContentFilter,
    Error,
}

/// A single streamed event from a provider. Mirrors `04-core-domain-model.md`
/// `ModelStreamDelta` at a finer granularity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModelEvent {
    TextDelta { text: String },
    ToolCallDelta(ToolCallDelta),
    UsageUpdate(UsageRecord),
    Stopped(StopReason),
    Error { message: String },
}

/// A request to count tokens for a prospective request, without sending it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenCountRequest {
    pub model: String,
    pub messages: Vec<ModelMessage>,
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct TokenCountResult {
    pub estimated_tokens: u64,
    /// True if the count came from the provider's exact tokenizer rather
    /// than a heuristic estimate.
    pub exact: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHealth {
    Healthy,
    Degraded,
    Unavailable,
}

/// A bounded channel of `ModelEvent`s, wrapped as a `Stream` for callers that
/// want combinator-based consumption (`tokio_stream::StreamExt`).
pub type ModelEventStream = ReceiverStream<SakhaResult<ModelEvent>>;

/// Normalizes all model providers behind one async contract. See
/// `04-core-domain-model.md` `ProviderClient`.
#[async_trait]
pub trait ProviderClient: Send + Sync {
    /// Streams a model response as an ordered sequence of `ModelEvent`s over
    /// a bounded `tokio::sync::mpsc` channel, exposed as a `Stream`.
    async fn stream(&self, request: ModelRequest) -> SakhaResult<ModelEventStream>;

    /// Performs a non-streaming completion, internally draining a stream if
    /// the adapter is stream-only.
    async fn complete(&self, request: ModelRequest) -> SakhaResult<ModelResponse>;

    async fn count_tokens(&self, request: TokenCountRequest) -> SakhaResult<TokenCountResult>;

    async fn health(&self) -> SakhaResult<ProviderHealth>;

    fn capabilities(&self) -> ProviderCapabilities;
}

/// A `ProviderClient` that always reports unavailable and returns
/// `not_implemented` errors. Used as a safe default/fallback so the workspace
/// never requires a live network provider to compile or run tests.
#[derive(Debug, Default)]
pub struct NullProviderClient;

#[async_trait]
impl ProviderClient for NullProviderClient {
    async fn stream(&self, _request: ModelRequest) -> SakhaResult<ModelEventStream> {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let _ = tx
            .send(Err(SakhaError::not_implemented("sakha-provider", "NullProviderClient::stream")))
            .await;
        Ok(ReceiverStream::new(rx))
    }

    async fn complete(&self, _request: ModelRequest) -> SakhaResult<ModelResponse> {
        Err(SakhaError::not_implemented("sakha-provider", "NullProviderClient::complete"))
    }

    async fn count_tokens(&self, _request: TokenCountRequest) -> SakhaResult<TokenCountResult> {
        Ok(TokenCountResult::default())
    }

    async fn health(&self) -> SakhaResult<ProviderHealth> {
        Ok(ProviderHealth::Unavailable)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
    }
}

/// A `ProviderClient` used in tests: replays a fixed, configured sequence of
/// `ModelEvent`s (or a canned `ModelResponse`) without any network I/O. See
/// spec `crate-work-breakdown.md` "Include a MockProvider for tests."
#[derive(Debug, Clone)]
pub struct MockProviderClient {
    /// Events replayed in order by `stream()`. Cloned into a fresh channel on
    /// every call so the mock can be invoked more than once.
    pub events: Vec<ModelEvent>,
    /// Response returned by `complete()`. If `None`, `complete()` derives a
    /// response by draining `events` (mirrors how a stream-only adapter would
    /// implement `complete`).
    pub response: Option<ModelResponse>,
    pub capabilities: ProviderCapabilities,
    pub health: ProviderHealth,
}

impl Default for MockProviderClient {
    fn default() -> Self {
        Self {
            events: vec![
                ModelEvent::TextDelta { text: "hello".to_string() },
                ModelEvent::Stopped(StopReason::EndTurn),
            ],
            response: None,
            capabilities: ProviderCapabilities {
                supports_tools: true,
                supports_json_schema: true,
                supports_streaming: true,
                supports_streaming_usage: true,
                supports_reasoning_effort: false,
                supports_parallel_tool_calls: true,
                max_context_tokens: None,
                max_output_tokens: None,
            },
            health: ProviderHealth::Healthy,
        }
    }
}

impl MockProviderClient {
    pub fn new(events: Vec<ModelEvent>) -> Self {
        Self { events, ..Self::default() }
    }

    pub fn with_response(mut self, response: ModelResponse) -> Self {
        self.response = Some(response);
        self
    }
}

#[async_trait]
impl ProviderClient for MockProviderClient {
    async fn stream(&self, request: ModelRequest) -> SakhaResult<ModelEventStream> {
        let (tx, rx) = tokio::sync::mpsc::channel(self.events.len().max(1));
        for event in self.events.clone() {
            let _ = tx.send(Ok(event)).await;
        }
        let _ = request;
        Ok(ReceiverStream::new(rx))
    }

    async fn complete(&self, request: ModelRequest) -> SakhaResult<ModelResponse> {
        if let Some(response) = &self.response {
            return Ok(response.clone());
        }
        // Derive a response by draining the configured events, mirroring how
        // a stream-only adapter would synthesize `complete()`.
        let mut text = String::new();
        let mut assembler = crate::tool_calls::ToolCallAssembler::new();
        let mut usage = UsageRecord::default();
        let mut stop_reason = StopReason::EndTurn;
        for event in &self.events {
            match event {
                ModelEvent::TextDelta { text: t } => text.push_str(t),
                ModelEvent::ToolCallDelta(delta) => assembler.push(delta.clone()),
                ModelEvent::UsageUpdate(u) => usage = usage.merge(*u),
                ModelEvent::Stopped(reason) => stop_reason = *reason,
                ModelEvent::Error { message } => {
                    return Err(SakhaError::transient("sakha-provider", message.clone()))
                }
            }
        }
        let tool_calls = assembler.finish();
        if !tool_calls.is_empty() {
            stop_reason = StopReason::ToolUse;
        }
        Ok(ModelResponse {
            request_id: request.id,
            text,
            tool_calls,
            usage,
            stop_reason,
        })
    }

    async fn count_tokens(&self, request: TokenCountRequest) -> SakhaResult<TokenCountResult> {
        // Simple whitespace-based heuristic estimate; deterministic for tests.
        let chars: usize = request.messages.iter().map(|m| m.content.len()).sum();
        Ok(TokenCountResult {
            estimated_tokens: (chars / 4).max(1) as u64,
            exact: false,
        })
    }

    async fn health(&self) -> SakhaResult<ProviderHealth> {
        Ok(self.health)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities.clone()
    }
}

#[cfg(test)]
mod mock_tests {
    use super::*;

    #[tokio::test]
    async fn mock_provider_streams_configured_events_in_order() {
        use tokio_stream::StreamExt;
        let client = MockProviderClient::new(vec![
            ModelEvent::TextDelta { text: "a".into() },
            ModelEvent::TextDelta { text: "b".into() },
            ModelEvent::Stopped(StopReason::EndTurn),
        ]);
        let mut stream = client.stream(ModelRequest::new("mock")).await.unwrap();
        let mut texts = Vec::new();
        while let Some(event) = stream.next().await {
            if let Ok(ModelEvent::TextDelta { text }) = event {
                texts.push(text);
            }
        }
        assert_eq!(texts, vec!["a".to_string(), "b".to_string()]);
    }

    #[tokio::test]
    async fn mock_provider_complete_derives_text_from_events_when_no_response_set() {
        let client = MockProviderClient::new(vec![
            ModelEvent::TextDelta { text: "hel".into() },
            ModelEvent::TextDelta { text: "lo".into() },
            ModelEvent::Stopped(StopReason::EndTurn),
        ]);
        let response = client.complete(ModelRequest::new("mock")).await.unwrap();
        assert_eq!(response.text, "hello");
        assert_eq!(response.stop_reason, StopReason::EndTurn);
    }

    #[tokio::test]
    async fn mock_provider_complete_assembles_tool_calls_from_deltas() {
        let client = MockProviderClient::new(vec![
            ModelEvent::ToolCallDelta(ToolCallDelta {
                index: 0,
                id: Some("call_1".into()),
                name: Some("read_file".into()),
                arguments_fragment: "{\"path\":".into(),
            }),
            ModelEvent::ToolCallDelta(ToolCallDelta {
                index: 0,
                id: None,
                name: None,
                arguments_fragment: "\"a.txt\"}".into(),
            }),
            ModelEvent::Stopped(StopReason::ToolUse),
        ]);
        let response = client.complete(ModelRequest::new("mock")).await.unwrap();
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "read_file");
        assert_eq!(response.tool_calls[0].arguments_json, "{\"path\":\"a.txt\"}");
        assert_eq!(response.stop_reason, StopReason::ToolUse);
    }

    #[tokio::test]
    async fn mock_provider_with_response_overrides_derived_response() {
        let canned = ModelResponse {
            request_id: sakha_core::ModelRequestId::new(),
            text: "canned".into(),
            tool_calls: vec![],
            usage: UsageRecord::default(),
            stop_reason: StopReason::EndTurn,
        };
        let client = MockProviderClient::default().with_response(canned);
        let response = client.complete(ModelRequest::new("mock")).await.unwrap();
        assert_eq!(response.text, "canned");
    }
}
