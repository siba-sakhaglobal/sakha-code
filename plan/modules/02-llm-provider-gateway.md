# Module 02: LLM Provider Gateway

## Responsibility

Normalize LLM provider access across OpenAI-compatible APIs, Gemini, xAI/Grok, OpenAI, local gateways, and future providers.

## Rust Crate

`crates/sakha-provider`

## Main Structs

- `ProviderRegistry`
- `ProviderProfile`
- `ProviderCapabilities`
- `ModelProfile`
- `ModelRequest`
- `ModelResponse`
- `ModelStream`
- `ModelEvent`
- `ToolCallDelta`
- `UsageRecord`
- `ProviderError`
- `RetryPolicy`
- `RateLimitState`

## Main Traits

### `ProviderClient`

- `stream(request: ModelRequest) -> Stream<ModelEvent>`
- `complete(request: ModelRequest) -> ModelResponse`
- `count_tokens(request: TokenCountRequest) -> TokenCountResult`
- `health() -> ProviderHealth`
- `capabilities() -> ProviderCapabilities`

### `ProviderAdapter`

- `build_http_request(request: ModelRequest) -> HttpRequest`
- `parse_stream_event(bytes) -> ModelEvent`
- `parse_response(bytes) -> ModelResponse`
- `normalize_error(status, body) -> ProviderError`

## Provider Profiles

### OpenAI Compatible

Fields:

- `base_url`
- `api_key_ref`
- `model`
- `small_fast_model`
- `reasoning_model`
- `max_tokens_field`
- `supports_tools`
- `supports_json_schema`
- `supports_streaming_usage`
- `supports_reasoning_effort`
- `extra_headers`
- `extra_body`

### Gemini

Use OpenAI-compatible mode with:

- `base_url=https://generativelanguage.googleapis.com/v1beta/openai/`
- capability overrides for Gemini-specific tool/schema behavior.

### xAI/Grok

Use OpenAI-compatible mode with:

- `base_url=https://api.x.ai/v1`
- capability overrides for model names and streaming behavior.

### Local Gateway

Support:

- Ollama
- vLLM
- LM Studio
- LiteLLM
- OpenRouter

## Features

- Provider-neutral message schema.
- Streaming normalizer.
- Tool call normalizer.
- JSON schema response mode.
- Provider capability detection.
- Retry and backoff.
- Rate-limit handling.
- Model fallback chain.
- Cost tracking.
- Token estimation fallback.
- Request redaction for logs.

## Implementation Tasks

1. Define provider-neutral message model.
2. Implement OpenAI-compatible adapter.
3. Implement SSE parser.
4. Implement tool call stream assembler.
5. Implement provider profile loader.
6. Implement capability registry.
7. Implement retry policy.
8. Implement usage/cost ledger.
9. Implement provider health check.
10. Implement integration tests with mocked streams.

## Failure Modes

- Provider rejects unsupported `response_format`.
- Provider streams partial malformed tool arguments.
- Tool call chunks arrive out of order.
- Provider omits usage.
- Rate limit body format differs.
- Timeout during stream.

## Tests

- Non-streaming completion.
- Streaming text.
- Streaming tool call.
- JSON schema mode.
- Provider-specific max token field.
- Retry only idempotent failures.
- Redaction of API keys and auth headers.

