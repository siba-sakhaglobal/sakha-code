//! OpenAI-compatible adapter: implements `ProviderClient` against any
//! OpenAI-compatible chat-completions API (OpenAI, Gemini-compat, xAI/Grok,
//! Ollama/vLLM/LM Studio/LiteLLM/OpenRouter).

use std::time::Duration;

use async_trait::async_trait;
use tokio_stream::wrappers::ReceiverStream;

use sakha_core::{ExponentialBackoff, RetryAfterOrBackoff, RetryPolicy, SakhaError, SakhaResult};

use crate::capabilities::ProviderCapabilities;
use crate::client::{
    MessageRole, ModelEvent, ModelEventStream, ModelRequest, ModelResponse, ProviderClient,
    ProviderHealth, StopReason, TokenCountRequest, TokenCountResult,
};
use crate::config::ProviderProfile;
use crate::cost::{estimate_cost_micros, UsageRecord};
use crate::sse::SseParser;
use crate::tool_calls::{AssembledToolCall, ToolCallAssembler, ToolCallDelta};

/// `ProviderClient` implementation for OpenAI-compatible HTTP APIs.
pub struct OpenAiCompatibleClient {
    pub profile: ProviderProfile,
    http: reqwest::Client,
    /// Resolved API key (already read from the environment via
    /// `profile.api_key_ref`). Kept out of `Debug`/logs.
    api_key: Option<String>,
    retry: ExponentialBackoff,
    /// Optional per-1k-token pricing (input, output) in micro-USD, used to
    /// populate `UsageRecord::cost_micros` on responses. `None` when pricing
    /// is unknown for this model (cost stays zero rather than guessed).
    pricing_micros_per_1k: Option<(u64, u64)>,
}

impl OpenAiCompatibleClient {
    pub fn new(profile: ProviderProfile) -> Self {
        let api_key = resolve_api_key(&profile);
        Self {
            profile,
            http: reqwest::Client::new(),
            api_key,
            retry: ExponentialBackoff::default(),
            pricing_micros_per_1k: None,
        }
    }

    /// Overrides the retry policy (e.g. shorter backoff/attempts for tests).
    pub fn with_retry_policy(mut self, retry: ExponentialBackoff) -> Self {
        self.retry = retry;
        self
    }

    /// Sets per-1k-token pricing (input, output) in micro-USD so `complete()`
    /// and streamed `UsageUpdate` events carry a populated `cost_micros`.
    pub fn with_pricing_micros_per_1k(mut self, input: u64, output: u64) -> Self {
        self.pricing_micros_per_1k = Some((input, output));
        self
    }

    fn priced(&self, mut usage: UsageRecord) -> UsageRecord {
        if let Some((input, output)) = self.pricing_micros_per_1k {
            usage.cost_micros = estimate_cost_micros(&usage, input, output);
        }
        usage
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.profile.base_url.trim_end_matches('/'))
    }

    /// Builds the JSON request body for the chat-completions endpoint,
    /// honoring provider profile capability overrides (tools, json schema,
    /// the provider-specific max-tokens field name, reasoning effort, and
    /// extra body merge).
    pub fn build_request_body(&self, request: &ModelRequest) -> serde_json::Value {
        let mut messages = Vec::with_capacity(request.messages.len());
        for m in &request.messages {
            let mut msg = serde_json::json!({
                "role": message_role_str(m.role),
                "content": m.content,
            });
            if let Some(tool_call_id) = &m.tool_call_id {
                msg["tool_call_id"] = serde_json::Value::String(tool_call_id.clone());
            }
            if let Some(name) = &m.name {
                msg["name"] = serde_json::Value::String(name.clone());
            }
            messages.push(msg);
        }

        let mut body = serde_json::json!({
            "model": request.model,
            "messages": messages,
            "stream": request.stream,
        });

        if request.stream && self.profile.supports_streaming_usage {
            body["stream_options"] = serde_json::json!({ "include_usage": true });
        }

        if self.profile.supports_tools && !request.tools.is_empty() {
            let tools: Vec<serde_json::Value> = request
                .tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.input_schema,
                        }
                    })
                })
                .collect();
            body["tools"] = serde_json::Value::Array(tools);
        }

        if self.profile.supports_json_schema {
            if let Some(schema) = &request.response_json_schema {
                body["response_format"] = serde_json::json!({
                    "type": "json_schema",
                    "json_schema": schema,
                });
            }
        }

        if let Some(max_tokens) = request.max_output_tokens {
            body[&self.profile.max_tokens_field] = serde_json::json!(max_tokens);
        }

        if let Some(temperature) = request.temperature {
            body["temperature"] = serde_json::json!(temperature);
        }

        if self.profile.supports_reasoning_effort {
            if let Some(effort) = &request.reasoning_effort {
                body["reasoning_effort"] = serde_json::json!(effort);
            }
        }

        if let serde_json::Value::Object(extra) = &self.profile.extra_body {
            if let serde_json::Value::Object(base) = &mut body {
                for (k, v) in extra {
                    base.insert(k.clone(), v.clone());
                }
            }
        }

        body
    }

    /// Returns a version of the outgoing headers safe to write to logs: the
    /// `Authorization` value and any configured `extra_headers` are replaced
    /// with `"[redacted]"`. See spec "Request redaction for logs" and test
    /// "Redaction of API keys and auth headers".
    pub fn redacted_headers_for_log(&self) -> std::collections::BTreeMap<String, String> {
        let mut out = std::collections::BTreeMap::new();
        out.insert("content-type".to_string(), "application/json".to_string());
        if self.api_key.is_some() {
            out.insert("authorization".to_string(), "[redacted]".to_string());
        }
        for (name, _) in &self.profile.extra_headers {
            out.insert(name.to_lowercase(), "[redacted]".to_string());
        }
        out
    }

    fn build_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        if let Some(key) = &self.api_key {
            if let Ok(value) = reqwest::header::HeaderValue::from_str(&format!("Bearer {key}")) {
                headers.insert(reqwest::header::AUTHORIZATION, value);
            }
        }
        for (name, value) in &self.profile.extra_headers {
            if let (Ok(name), Ok(value)) = (
                reqwest::header::HeaderName::from_bytes(name.as_bytes()),
                reqwest::header::HeaderValue::from_str(value),
            ) {
                headers.insert(name, value);
            }
        }
        headers
    }

    /// Sends the request once (no retry), returning either a streaming
    /// `reqwest::Response` or a normalized `SakhaError`.
    async fn send_once(&self, body: &serde_json::Value) -> SakhaResult<reqwest::Response> {
        let response = self
            .http
            .post(self.endpoint())
            .headers(self.build_headers())
            .json(body)
            .send()
            .await
            .map_err(|e| {
                let mut err = SakhaError::transient(
                    "sakha-provider",
                    format!("request to {} failed: {e}", self.profile.base_url),
                );
                if e.is_timeout() || e.is_connect() {
                    err = err.with_retryable(true);
                }
                err
            })?;

        if response.status().is_success() {
            return Ok(response);
        }

        let status = response.status();
        let retry_after = parse_retry_after(response.headers());
        let body_text = response.text().await.unwrap_or_default();
        Err(normalize_error(status.as_u16(), &body_text, retry_after))
    }

    /// Sends the request with retry/backoff for `Transient` errors, honoring
    /// `Retry-After` when present.
    async fn send_with_retry(&self, body: &serde_json::Value) -> SakhaResult<reqwest::Response> {
        let mut attempt = 0u32;
        loop {
            match self.send_once(body).await {
                Ok(response) => return Ok(response),
                Err(err) => {
                    let retry_after = extract_retry_after_from_error(&err);
                    let policy = RetryAfterOrBackoff::new(retry_after, self.retry);
                    match policy.next_delay(attempt, &err) {
                        Some(delay) => {
                            attempt += 1;
                            tokio::time::sleep(delay).await;
                        }
                        None => return Err(err),
                    }
                }
            }
        }
    }
}

fn message_role_str(role: MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
    }
}

/// Resolves the API key from the environment variable named by
/// `profile.api_key_ref`. Returns `None` if unset (local gateways like Ollama
/// typically need no key); never panics on a missing var.
fn resolve_api_key(profile: &ProviderProfile) -> Option<String> {
    let env_var = profile.api_key_ref.as_ref()?;
    std::env::var(env_var).ok()
}

/// Parses a `Retry-After` header as a duration. Supports the delay-seconds
/// form; the HTTP-date form is not supported (rare for LLM providers) and is
/// ignored rather than erroring.
fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    let seconds: u64 = value.trim().parse().ok()?;
    Some(Duration::from_secs(seconds))
}

/// Stashes a `Retry-After` duration inside the error's `cause` field so it
/// survives the `SakhaError` boundary without widening the shared error type.
/// Encoded as `retry-after-secs=<n>` and parsed back by
/// `extract_retry_after_from_error`.
fn stash_retry_after(err: SakhaError, retry_after: Option<Duration>) -> SakhaError {
    match retry_after {
        Some(d) => err.with_cause(format!("retry-after-secs={}", d.as_secs())),
        None => err,
    }
}

fn extract_retry_after_from_error(err: &SakhaError) -> Option<Duration> {
    let cause = err.cause.as_ref()?;
    let secs_str = cause.strip_prefix("retry-after-secs=")?;
    let secs: u64 = secs_str.parse().ok()?;
    Some(Duration::from_secs(secs))
}

/// Normalizes an HTTP status + response body into a `SakhaError` with the
/// right `ErrorClass`. See spec `modules/02-llm-provider-gateway.md`
/// `ProviderAdapter::normalize_error` and `modules/22-error-taxonomy-retry.md`.
pub fn normalize_error(status: u16, body: &str, retry_after: Option<Duration>) -> SakhaError {
    let message = extract_error_message(body).unwrap_or_else(|| body.to_string());
    let err = match status {
        401 | 403 => SakhaError::fatal("sakha-provider", format!("auth error ({status}): {message}")),
        400 | 404 | 422 => {
            SakhaError::invalid_input("sakha-provider", format!("bad request ({status}): {message}"))
        }
        408 => SakhaError::transient("sakha-provider", format!("timeout ({status}): {message}")),
        429 => SakhaError::transient("sakha-provider", format!("rate limited ({status}): {message}")),
        500..=599 => SakhaError::transient("sakha-provider", format!("server error ({status}): {message}")),
        _ => SakhaError::fatal("sakha-provider", format!("unexpected status ({status}): {message}")),
    };
    stash_retry_after(err.with_subject(status.to_string()), retry_after)
}

/// Best-effort extraction of `{"error": {"message": "..."}}` (OpenAI shape)
/// or `{"message": "..."}` (some gateways) from a raw response body.
fn extract_error_message(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    value
        .get("error")
        .and_then(|e| e.get("message"))
        .or_else(|| value.get("message"))
        .and_then(|m| m.as_str())
        .map(|s| s.to_string())
}

/// Parses one OpenAI-compatible `chat.completion.chunk` SSE data payload into
/// zero or more `ModelEvent`s.
fn parse_chunk_json(data: &str) -> Vec<ModelEvent> {
    if data == "[DONE]" {
        return vec![ModelEvent::Stopped(StopReason::EndTurn)];
    }
    let value: serde_json::Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut events = Vec::new();

    if let Some(usage) = value.get("usage") {
        if !usage.is_null() {
            events.push(ModelEvent::UsageUpdate(parse_usage(usage)));
        }
    }

    if let Some(choice) = value.get("choices").and_then(|c| c.as_array()).and_then(|a| a.first()) {
        if let Some(delta) = choice.get("delta") {
            if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                if !content.is_empty() {
                    events.push(ModelEvent::TextDelta { text: content.to_string() });
                }
            }
            if let Some(tool_calls) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                for tc in tool_calls {
                    let index = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as u32;
                    let id = tc.get("id").and_then(|i| i.as_str()).map(|s| s.to_string());
                    let name = tc
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                        .map(|s| s.to_string());
                    let arguments_fragment = tc
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .and_then(|a| a.as_str())
                        .unwrap_or("")
                        .to_string();
                    events.push(ModelEvent::ToolCallDelta(ToolCallDelta {
                        index,
                        id,
                        name,
                        arguments_fragment,
                    }));
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
            events.push(ModelEvent::Stopped(parse_stop_reason(reason)));
        }
    }

    events
}

fn parse_usage(usage: &serde_json::Value) -> UsageRecord {
    let input_tokens = usage.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let output_tokens = usage.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let cached_input_tokens = usage
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    UsageRecord {
        input_tokens,
        output_tokens,
        cached_input_tokens,
        cost_micros: 0,
    }
}

fn parse_stop_reason(reason: &str) -> StopReason {
    match reason {
        "stop" => StopReason::EndTurn,
        "tool_calls" | "function_call" => StopReason::ToolUse,
        "length" => StopReason::MaxTokens,
        "content_filter" => StopReason::ContentFilter,
        _ => StopReason::EndTurn,
    }
}

#[async_trait]
impl ProviderClient for OpenAiCompatibleClient {
    async fn stream(&self, request: ModelRequest) -> SakhaResult<ModelEventStream> {
        let mut body = self.build_request_body(&request);
        body["stream"] = serde_json::Value::Bool(true);

        let response = self.send_with_retry(&body).await?;

        let (tx, rx) = tokio::sync::mpsc::channel::<SakhaResult<ModelEvent>>(32);
        tokio::spawn(async move {
            use futures::StreamExt;
            let mut byte_stream = response.bytes_stream();
            let mut parser = SseParser::new();
            loop {
                match byte_stream.next().await {
                    Some(Ok(chunk)) => {
                        for sse_event in parser.feed(&chunk) {
                            if sse_event.data.is_empty() {
                                continue;
                            }
                            for event in parse_chunk_json(&sse_event.data) {
                                if tx.send(Ok(event)).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        let err = SakhaError::transient(
                            "sakha-provider",
                            format!("stream read error: {e}"),
                        );
                        let _ = tx.send(Err(err)).await;
                        return;
                    }
                    None => return,
                }
            }
        });

        Ok(ReceiverStream::new(rx))
    }

    async fn complete(&self, request: ModelRequest) -> SakhaResult<ModelResponse> {
        // Internally drains the stream and assembles a single response, since
        // this adapter is stream-first (per spec "internally draining a
        // stream if the adapter is stream-only").
        let request_id = request.id;
        let mut event_stream = self.stream(request).await?;

        use tokio_stream::StreamExt;
        let mut text = String::new();
        let mut assembler = ToolCallAssembler::new();
        let mut usage = UsageRecord::default();
        let mut stop_reason = StopReason::EndTurn;

        while let Some(event) = event_stream.next().await {
            match event? {
                ModelEvent::TextDelta { text: t } => text.push_str(&t),
                ModelEvent::ToolCallDelta(delta) => assembler.push(delta),
                ModelEvent::UsageUpdate(u) => usage = usage.merge(u),
                ModelEvent::Stopped(reason) => stop_reason = reason,
                ModelEvent::Error { message } => {
                    return Err(SakhaError::transient("sakha-provider", message))
                }
            }
        }

        let tool_calls: Vec<AssembledToolCall> = assembler.finish();
        if !tool_calls.is_empty() && stop_reason == StopReason::EndTurn {
            stop_reason = StopReason::ToolUse;
        }

        Ok(ModelResponse {
            request_id,
            text,
            tool_calls,
            usage: self.priced(usage),
            stop_reason,
        })
    }

    async fn count_tokens(&self, request: TokenCountRequest) -> SakhaResult<TokenCountResult> {
        // Providers rarely expose a dedicated token-count endpoint over the
        // OpenAI-compatible chat API; fall back to a heuristic estimate
        // (~4 chars/token) per spec "Token estimation fallback".
        let chars: usize = request.messages.iter().map(|m| m.content.len()).sum();
        Ok(TokenCountResult {
            estimated_tokens: (chars / 4).max(1) as u64,
            exact: false,
        })
    }

    async fn health(&self) -> SakhaResult<ProviderHealth> {
        let url = format!("{}/models", self.profile.base_url.trim_end_matches('/'));
        match self.http.get(url).headers(self.build_headers()).send().await {
            Ok(resp) if resp.status().is_success() => Ok(ProviderHealth::Healthy),
            Ok(_) => Ok(ProviderHealth::Degraded),
            Err(_) => Ok(ProviderHealth::Unavailable),
        }
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_tools: self.profile.supports_tools,
            supports_json_schema: self.profile.supports_json_schema,
            supports_streaming: true,
            supports_streaming_usage: self.profile.supports_streaming_usage,
            supports_reasoning_effort: self.profile.supports_reasoning_effort,
            supports_parallel_tool_calls: false,
            max_context_tokens: None,
            max_output_tokens: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ModelMessage;

    #[test]
    fn build_request_body_includes_model_and_messages() {
        let profile = ProviderProfile::openai_compatible("test", "https://example.com", "gpt-test");
        let client = OpenAiCompatibleClient::new(profile);
        let mut request = ModelRequest::new("gpt-test");
        request.messages.push(ModelMessage {
            role: MessageRole::User,
            content: "hi".into(),
            tool_call_id: None,
            name: None,
        });
        let body = client.build_request_body(&request);
        assert_eq!(body["model"], "gpt-test");
        assert_eq!(body["messages"][0]["content"], "hi");
    }

    #[test]
    fn build_request_body_uses_provider_specific_max_tokens_field() {
        let mut profile = ProviderProfile::openai_compatible("test", "https://example.com", "gpt-test");
        profile.max_tokens_field = "max_completion_tokens".to_string();
        let client = OpenAiCompatibleClient::new(profile);
        let mut request = ModelRequest::new("gpt-test");
        request.max_output_tokens = Some(256);
        let body = client.build_request_body(&request);
        assert_eq!(body["max_completion_tokens"], 256);
        assert!(body.get("max_tokens").is_none());
    }

    #[test]
    fn build_request_body_omits_tools_when_capability_disabled() {
        let mut profile = ProviderProfile::openai_compatible("test", "https://example.com", "gpt-test");
        profile.supports_tools = false;
        let client = OpenAiCompatibleClient::new(profile);
        let mut request = ModelRequest::new("gpt-test");
        request.tools.push(crate::client::ToolDefinition {
            name: "read_file".into(),
            description: "reads a file".into(),
            input_schema: serde_json::json!({"type": "object"}),
        });
        let body = client.build_request_body(&request);
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn build_request_body_includes_tools_when_capability_enabled() {
        let profile = ProviderProfile::openai_compatible("test", "https://example.com", "gpt-test");
        let client = OpenAiCompatibleClient::new(profile);
        let mut request = ModelRequest::new("gpt-test");
        request.tools.push(crate::client::ToolDefinition {
            name: "read_file".into(),
            description: "reads a file".into(),
            input_schema: serde_json::json!({"type": "object"}),
        });
        let body = client.build_request_body(&request);
        assert_eq!(body["tools"][0]["function"]["name"], "read_file");
    }

    #[test]
    fn build_request_body_sets_json_schema_response_format() {
        let profile = ProviderProfile::openai_compatible("test", "https://example.com", "gpt-test");
        let client = OpenAiCompatibleClient::new(profile);
        let mut request = ModelRequest::new("gpt-test");
        request.response_json_schema = Some(serde_json::json!({"name": "answer", "schema": {}}));
        let body = client.build_request_body(&request);
        assert_eq!(body["response_format"]["type"], "json_schema");
    }

    #[test]
    fn build_request_body_merges_extra_body() {
        let mut profile = ProviderProfile::openai_compatible("test", "https://example.com", "gpt-test");
        profile.extra_body = serde_json::json!({"safe_prompt": true});
        let client = OpenAiCompatibleClient::new(profile);
        let request = ModelRequest::new("gpt-test");
        let body = client.build_request_body(&request);
        assert_eq!(body["safe_prompt"], true);
    }

    #[test]
    fn parse_chunk_json_extracts_text_delta() {
        let data = r#"{"choices":[{"delta":{"content":"hi"}}]}"#;
        let events = parse_chunk_json(data);
        assert_eq!(events.len(), 1);
        match &events[0] {
            ModelEvent::TextDelta { text } => assert_eq!(text, "hi"),
            _ => panic!("expected text delta"),
        }
    }

    #[test]
    fn parse_chunk_json_extracts_tool_call_delta() {
        let data = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":"{\"path\":"}}]}}]}"#;
        let events = parse_chunk_json(data);
        assert_eq!(events.len(), 1);
        match &events[0] {
            ModelEvent::ToolCallDelta(delta) => {
                assert_eq!(delta.index, 0);
                assert_eq!(delta.id.as_deref(), Some("call_1"));
                assert_eq!(delta.name.as_deref(), Some("read_file"));
                assert_eq!(delta.arguments_fragment, "{\"path\":");
            }
            _ => panic!("expected tool call delta"),
        }
    }

    #[test]
    fn parse_chunk_json_done_sentinel_yields_stopped() {
        let events = parse_chunk_json("[DONE]");
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], ModelEvent::Stopped(StopReason::EndTurn)));
    }

    #[test]
    fn parse_chunk_json_extracts_usage() {
        let data = r#"{"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#;
        let events = parse_chunk_json(data);
        assert_eq!(events.len(), 1);
        match &events[0] {
            ModelEvent::UsageUpdate(usage) => {
                assert_eq!(usage.input_tokens, 10);
                assert_eq!(usage.output_tokens, 5);
            }
            _ => panic!("expected usage update"),
        }
    }

    #[test]
    fn parse_chunk_json_finish_reason_tool_calls_maps_to_tool_use() {
        let data = r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#;
        let events = parse_chunk_json(data);
        assert!(events.iter().any(|e| matches!(e, ModelEvent::Stopped(StopReason::ToolUse))));
    }

    #[test]
    fn normalize_error_maps_429_to_transient_and_retryable() {
        let err = normalize_error(429, r#"{"error":{"message":"rate limited"}}"#, None);
        assert_eq!(err.class, sakha_core::ErrorClass::Transient);
        assert!(err.retryable);
        assert!(err.message.contains("rate limited"));
    }

    #[test]
    fn normalize_error_maps_401_to_fatal_and_not_retryable() {
        let err = normalize_error(401, r#"{"error":{"message":"invalid api key"}}"#, None);
        assert_eq!(err.class, sakha_core::ErrorClass::Fatal);
        assert!(!err.retryable);
    }

    #[test]
    fn normalize_error_maps_400_to_invalid_input() {
        let err = normalize_error(400, r#"{"error":{"message":"bad schema"}}"#, None);
        assert_eq!(err.class, sakha_core::ErrorClass::InvalidInput);
    }

    #[test]
    fn normalize_error_maps_500_to_transient() {
        let err = normalize_error(500, "internal error", None);
        assert_eq!(err.class, sakha_core::ErrorClass::Transient);
        assert!(err.retryable);
    }

    #[test]
    fn normalize_error_falls_back_to_raw_body_when_not_json() {
        let err = normalize_error(500, "upstream unavailable", None);
        assert!(err.message.contains("upstream unavailable"));
    }

    #[test]
    fn retry_after_roundtrips_through_error_cause() {
        let err = normalize_error(429, "{}", Some(Duration::from_secs(3)));
        let extracted = extract_retry_after_from_error(&err);
        assert_eq!(extracted, Some(Duration::from_secs(3)));
    }

    #[test]
    fn resolve_api_key_returns_none_when_env_ref_unset() {
        let mut profile = ProviderProfile::openai_compatible("test", "https://example.com", "gpt-test");
        profile.api_key_ref = Some("SAKHA_TEST_NONEXISTENT_KEY_VAR".to_string());
        assert!(resolve_api_key(&profile).is_none());
    }

    #[test]
    fn priced_computes_cost_from_configured_pricing() {
        let profile = ProviderProfile::openai_compatible("test", "https://example.com", "gpt-test");
        let client = OpenAiCompatibleClient::new(profile).with_pricing_micros_per_1k(1000, 2000);
        let usage = UsageRecord {
            input_tokens: 1000,
            output_tokens: 500,
            cached_input_tokens: 0,
            cost_micros: 0,
        };
        let priced = client.priced(usage);
        assert_eq!(priced.cost_micros, 1000 + 1000);
    }

    #[test]
    fn priced_leaves_cost_zero_when_pricing_unset() {
        let profile = ProviderProfile::openai_compatible("test", "https://example.com", "gpt-test");
        let client = OpenAiCompatibleClient::new(profile);
        let usage = UsageRecord {
            input_tokens: 1000,
            output_tokens: 500,
            cached_input_tokens: 0,
            cost_micros: 0,
        };
        assert_eq!(client.priced(usage).cost_micros, 0);
    }

    #[test]
    fn resolve_api_key_returns_none_when_no_ref_configured() {
        let profile = ProviderProfile::openai_compatible("test", "https://example.com", "gpt-test");
        assert!(resolve_api_key(&profile).is_none());
    }

    #[test]
    fn redacted_headers_never_expose_api_key_or_extra_headers() {
        std::env::set_var("SAKHA_TEST_REDACT_KEY", "sk-super-secret-value");
        let mut profile = ProviderProfile::openai_compatible("test", "https://example.com", "gpt-test");
        profile.api_key_ref = Some("SAKHA_TEST_REDACT_KEY".to_string());
        profile.extra_headers.push(("X-Org-Token".to_string(), "org-secret".to_string()));
        let client = OpenAiCompatibleClient::new(profile);

        let redacted = client.redacted_headers_for_log();
        std::env::remove_var("SAKHA_TEST_REDACT_KEY");

        let rendered = format!("{redacted:?}");
        assert!(!rendered.contains("sk-super-secret-value"));
        assert!(!rendered.contains("org-secret"));
        assert_eq!(redacted.get("authorization").map(String::as_str), Some("[redacted]"));
        assert_eq!(redacted.get("x-org-token").map(String::as_str), Some("[redacted]"));
    }

    #[tokio::test]
    async fn mock_provider_client_satisfies_provider_client_trait_object() {
        // Sanity check that OpenAiCompatibleClient and MockProviderClient are
        // interchangeable behind the trait, exercised via the mock (no
        // network) so this test stays hermetic.
        use crate::client::{MockProviderClient, ProviderClient};
        let client: Box<dyn ProviderClient> = Box::new(MockProviderClient::default());
        let health = client.health().await.unwrap();
        assert_eq!(health, ProviderHealth::Healthy);
    }
}
