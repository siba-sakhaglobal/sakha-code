# Module 01: Core Runtime

## Responsibility

Own process lifecycle, session lifecycle, event routing, cancellation, budgets, checkpoints, and runtime services shared by every other module.

## Rust Crate

`crates/sakha-core`

## Main Structs

- `Runtime`
- `RuntimeConfig`
- `Workspace`
- `WorkspaceId`
- `SessionId`
- `SessionHandle`
- `CancellationToken`
- `Budget`
- `BudgetLedger`
- `Checkpoint`
- `ArtifactRef`
- `EventEnvelope`
- `EventBus`
- `RuntimeError`

## Main Traits

### `RuntimeService`

- `name() -> &'static str`
- `start(ctx: RuntimeContext) -> Result<()>`
- `stop(reason: ShutdownReason) -> Result<()>`
- `health() -> HealthReport`

### `ArtifactStore`

- `put_bytes(kind, bytes, metadata) -> ArtifactRef`
- `put_text(kind, text, metadata) -> ArtifactRef`
- `get(ref) -> Artifact`
- `link(parent, child, relation)`

### `EventSink`

- `emit(event: EventEnvelope)`
- `subscribe(filter: EventFilter) -> EventStream`

## Features

- Runtime boot and shutdown.
- Workspace discovery.
- Session creation/resume.
- Per-session cancellation.
- Budget tracking for:
  - input tokens
  - output tokens
  - provider cost
  - tool calls
  - wall-clock time
  - loop iterations
  - filesystem writes
- Checkpoint writing.
- Artifact storage.
- Event stream fanout to UI, logs, and persistence.
- Graceful crash recovery.

## Implementation Tasks

1. Define ID newtypes.
2. Define common error model.
3. Define `RuntimeConfig`.
4. Implement `EventBus` with bounded channels.
5. Implement `BudgetLedger`.
6. Implement `ArtifactStore` backed by filesystem + SQLite metadata.
7. Implement `SessionRegistry`.
8. Implement cancellation propagation.
9. Implement checkpoint manager.
10. Implement health reports.

## Failure Modes

- Event bus backpressure.
- Artifact write failure.
- Corrupt checkpoint.
- Budget race condition.
- Cancellation not reaching child task.
- Session resume with missing workspace.

## Tests

- Create/resume session.
- Cancel nested tasks.
- Budget exhaustion stops loop.
- Checkpoint persists and reloads.
- Event stream does not block runtime.
- Artifact hash deduplicates identical content.

