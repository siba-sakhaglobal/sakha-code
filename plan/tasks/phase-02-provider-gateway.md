# Phase 02: Provider Gateway

## Goal

Implement provider-neutral LLM access with OpenAI-compatible support.

## Tasks

### 02.1 Schemas

- Define `ModelRequest`.
- Define `ModelMessage`.
- Define `ToolSpec`.
- Define `ToolCall`.
- Define `ModelEvent`.
- Define `UsageRecord`.

### 02.2 OpenAI-Compatible Adapter

- Build `/chat/completions` request.
- Support streaming SSE.
- Support non-streaming response.
- Normalize tool calls.
- Normalize JSON schema response format.
- Normalize errors.

### 02.3 Provider Profiles

- Load provider profiles from config.
- Add OpenAI default.
- Add Gemini example.
- Add xAI/Grok example.
- Add local gateway example.

### 02.4 Streaming Tests

- Text stream fixture.
- Tool-call stream fixture.
- Error fixture.
- Usage fixture.

### 02.5 Cost and Usage

- Token estimates.
- Provider usage parsing.
- Cost profile config.
- Budget integration.

## Definition of Done

- `sakha run "hello"` streams via OpenAI-compatible profile.
- Tool-call stream fixture assembles valid tool call.
- Provider errors are user-readable.
- Usage is persisted.

