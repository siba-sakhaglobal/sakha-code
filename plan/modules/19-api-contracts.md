# Module 19: API Contracts

## Responsibility

Define stable schemas for daemon API, UI event streams, plugin APIs, and provider-neutral agent messages.

## Rust Crate

`crates/sakha-ui-protocol`

## API Styles

- Local REST for simple commands.
- WebSocket for live sessions.
- SSE for model/tool event streaming.
- JSON-RPC for plugin/MCP bridge.
- JSON Schema for persisted configs.

## Core Endpoints

- `GET /health`
- `POST /sessions`
- `GET /sessions/:id`
- `POST /sessions/:id/input`
- `POST /sessions/:id/cancel`
- `GET /sessions/:id/events`
- `GET /goals`
- `POST /goals`
- `GET /loops`
- `POST /loops`
- `POST /tools/:name/preview`
- `POST /permissions/:id/decision`
- `GET /audit`
- `GET /compression/stats`
- `GET /research/:id/evidence`

## Schemas

- `SessionDto`
- `TurnDto`
- `MessageDto`
- `ToolCallDto`
- `PermissionRequestDto`
- `DiffDto`
- `LoopSpecDto`
- `LoopTickDto`
- `ProviderProfileDto`
- `CompressionStatsDto`
- `EvidencePackDto`
- `AuditEventDto`

## Event Protocol

Every event:

```text
{
  id,
  type,
  session_id,
  sequence,
  timestamp,
  payload
}
```

## Versioning

- API version in route prefix or header.
- Schema version in persisted records.
- Backward-compatible additive changes preferred.
- Migration required for breaking changes.

## Implementation Tasks

1. Define DTOs.
2. Generate JSON Schemas.
3. Implement REST handlers.
4. Implement WebSocket/SSE streams.
5. Implement API auth for server mode.
6. Implement compatibility tests.
7. Generate TypeScript client.
8. Generate Rust client.
9. Add OpenAPI export.
10. Add contract tests.

## Tests

- DTO serialization round trip.
- Event ordering.
- SSE reconnect with last event ID.
- API rejects invalid loop spec.
- Generated TS client compiles.

