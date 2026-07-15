# System Architecture

## Process Topology

```text
CLI/TUI/Web/Desktop
       |
       v
Local Sakha Daemon  <---->  SQLite State Store
       |
       +---- Provider Gateway ---- LLM Providers
       |
       +---- Tool Runtime -------- Shell/File/Git/MCP/Browser/Web
       |
       +---- Context Layer ------- Headroom / Local Compressors / Retrieval Store
       |
       +---- Loop Engine --------- Scheduler / Event Queue / Verifier
       |
       +---- Security Layer ------ Permission Policy / Sandbox / Secrets
```

## Main Runtime Flow

1. UI or CLI sends a `RunRequest`.
2. Runtime loads workspace policy, session state, memory, and provider profile.
3. Context planner builds context bundle.
4. Compression layer routes context through Headroom/fallback compression.
5. Provider gateway streams model events.
6. Agent loop parses tool calls.
7. Tool runtime validates permissions and executes tools.
8. Tool outputs are stored raw, compressed, and summarized.
9. Verifier runs deterministic checks.
10. Loop controller decides continue, retry, handoff, block, or complete.
11. Runtime writes checkpoint and audit events.

## Dependency Direction

Lower modules must not depend on higher modules.

```text
ui -> daemon -> agent -> loop -> tools/provider/context/security -> core
```

`sakha-core` owns IDs, errors, event envelopes, budgets, and common types.

## Core Crates

### `sakha-core`

- IDs and typed newtypes.
- Error model.
- Event envelope.
- Budget model.
- Artifact refs.
- JSON schema utilities.

### `sakha-provider`

- Provider registry.
- OpenAI-compatible adapter.
- Streaming parser.
- Tool call normalizer.
- Model capability registry.

### `sakha-agent`

- Agent loop.
- Message assembler.
- Planning state.
- Tool call interpreter.
- Termination detector.

### `sakha-tools`

- Tool trait.
- Tool registry.
- Built-in tools.
- Tool executor.
- Permission bridge.

### `sakha-context`

- Context planner.
- Context budgeter.
- Source attribution.
- Prompt/cache layout.

### `sakha-compression`

- Headroom adapter.
- Compression policy.
- Retrieval marker handling.
- Fallback compressors.

### `sakha-memory`

- SQLite store.
- Session history.
- Goal/plan state.
- Handoff artifacts.
- Research memory.

### `sakha-loop`

- Loop specs.
- Scheduler.
- Event triggers.
- Work queue.
- Stall detection.
- Verification loop.

### `sakha-security`

- Permission policy.
- Sandbox adapters.
- Secret references.
- Redaction.
- Audit checks.

### `sakha-daemon`

- Local HTTP/WebSocket/SSE API.
- Session multiplexing.
- UI protocol.
- Background loop runner.

## Threading Model

- Tokio runtime for async tasks.
- Dedicated process supervisor tasks for shell/PTY.
- Bounded channels between model stream, tool loop, and UI stream.
- Cancellation token propagated across every long-running task.
- Blocking filesystem/git operations isolated in blocking thread pool.

## Failure Rules

- Every tool call writes an audit event before execution.
- Every loop tick writes start and terminal state.
- Every model request records provider, model, request hash, and usage.
- Every compressed item can be traced to raw artifact unless policy says irreversible.
- Every blocked goal must include a human-readable blocker and next action.

