# Phase 05: Headroom Integration

## Goal

Make Headroom a built-in context compression layer.

## Tasks

### 05.1 Compression Abstraction

- Define `ContextCompressor`.
- Define `CompressionPolicy`.
- Define `CompressionMarker`.
- Define `CompressionStats`.

### 05.2 Headroom Adapter

- Support sidecar mode.
- Support MCP mode.
- Support proxy mode.
- Health check Headroom.
- Store Headroom config.

### 05.3 Retrieval Store

- Store raw artifacts.
- Store compressed artifacts.
- Store marker mapping.
- Expose `compression.retrieve` tool.

### 05.4 Pipeline Integration

- Compress tool outputs.
- Compress shell logs.
- Compress file trees.
- Compress web pages.
- Compress conversation history.

### 05.5 Accuracy Evals

- Create fixtures for logs, JSON, code, prose.
- Compare answer quality with and without compression.
- Track retrieval rate.
- Detect over-compression.

## Definition of Done

- Large tool output is compressed before model.
- Model can retrieve raw content by marker.
- Compression stats show tokens saved.
- If Headroom fails, policy determines fail-open/fail-closed.
- Compression evals pass threshold.

