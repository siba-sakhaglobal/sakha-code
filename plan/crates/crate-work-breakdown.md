# Crate Work Breakdown

This file maps future Rust crates to expected files, structs, traits, and tests.

## `sakha-core`

### Files

- `src/id.rs`
- `src/error.rs`
- `src/event.rs`
- `src/budget.rs`
- `src/artifact.rs`
- `src/time.rs`
- `src/lib.rs`

### Types

- `SessionId`
- `TurnId`
- `ToolCallId`
- `GoalId`
- `LoopId`
- `ArtifactRef`
- `SakhaError`
- `EventEnvelope`
- `Budget`
- `BudgetLedger`

### Tests

- ID serialization.
- Error display.
- Budget debit/credit.
- Event envelope ordering.

## `sakha-provider`

### Files

- `src/client.rs`
- `src/openai_compatible.rs`
- `src/sse.rs`
- `src/tool_calls.rs`
- `src/capabilities.rs`
- `src/cost.rs`
- `src/config.rs`
- `src/lib.rs`

### Types

- `ProviderClient`
- `ProviderProfile`
- `ModelRequest`
- `ModelMessage`
- `ModelEvent`
- `ToolCallDelta`
- `ProviderCapabilities`
- `UsageRecord`

### Tests

- SSE parser.
- Tool call assembly.
- Error normalization.
- Provider config loading.

## `sakha-agent`

### Files

- `src/agent.rs`
- `src/state.rs`
- `src/messages.rs`
- `src/policy.rs`
- `src/tool_loop.rs`
- `src/termination.rs`
- `src/handoff.rs`
- `src/lib.rs`

### Types

- `Agent`
- `AgentState`
- `AgentPolicy`
- `TurnContext`
- `ToolCallState`
- `LoopDecision`
- `HandoffArtifact`

### Tests

- Tool loop.
- Termination.
- Handoff.
- Invalid tool recovery.

## `sakha-tools`

### Files

- `src/tool.rs`
- `src/registry.rs`
- `src/executor.rs`
- `src/permissions.rs`
- `src/builtins/file.rs`
- `src/builtins/git.rs`
- `src/builtins/shell.rs`
- `src/builtins/search.rs`
- `src/lib.rs`

### Types

- `Tool`
- `ToolSpec`
- `ToolRegistry`
- `ToolExecutor`
- `ToolCall`
- `ToolResult`
- `ToolContext`

### Tests

- Schema validation.
- Permission denied.
- File patch.
- Shell timeout.

## `sakha-compression`

### Files

- `src/compressor.rs`
- `src/headroom.rs`
- `src/sidecar.rs`
- `src/mcp.rs`
- `src/policy.rs`
- `src/retrieval.rs`
- `src/stats.rs`
- `src/lib.rs`

### Types

- `ContextCompressor`
- `CompressionPolicy`
- `HeadroomClient`
- `HeadroomSidecar`
- `CompressionMarker`
- `RetrievalStore`
- `CompressionStats`

### Tests

- Compress/retrieve.
- Headroom unavailable.
- Policy routing.
- Marker mapping.

## `sakha-memory`

### Files

- `src/db.rs`
- `src/migrations.rs`
- `src/session.rs`
- `src/memory.rs`
- `src/handoff.rs`
- `src/research.rs`
- `src/artifact_store.rs`
- `src/lib.rs`

### Types

- `MemoryStore`
- `SessionStore`
- `ArtifactStore`
- `HandoffStore`
- `ResearchStore`

### Tests

- Migration.
- Session resume.
- Artifact dedup.
- Research dedup.

## `sakha-loop`

### Files

- `src/spec.rs`
- `src/runtime.rs`
- `src/scheduler.rs`
- `src/event_queue.rs`
- `src/watchdog.rs`
- `src/replay.rs`
- `src/verification_loop.rs`
- `src/lib.rs`

### Types

- `LoopSpec`
- `LoopRuntime`
- `LoopTick`
- `Trigger`
- `LoopWatchdog`
- `ReplayTrace`
- `LoopSkill`

### Tests

- Scheduled tick.
- Deduplication.
- Budget stop.
- Replay dry-run.

## `sakha-research`

### Files

- `src/search.rs`
- `src/fetch.rs`
- `src/extract.rs`
- `src/scoring.rs`
- `src/evidence.rs`
- `src/injection_filter.rs`
- `src/lib.rs`

### Types

- `SearchClient`
- `SearchQuery`
- `SearchResult`
- `FetchedPage`
- `EvidencePack`
- `Citation`
- `SourceScore`

### Tests

- Source scoring.
- Extraction.
- Evidence pack.
- Injection filter.

## `sakha-security`

### Files

- `src/policy.rs`
- `src/permissions.rs`
- `src/secrets.rs`
- `src/redaction.rs`
- `src/sandbox.rs`
- `src/risk.rs`
- `src/lib.rs`

### Types

- `PermissionPolicy`
- `PermissionRequest`
- `PermissionDecision`
- `SecretStore`
- `SandboxProfile`
- `RiskClassifier`

### Tests

- Policy merge.
- Redaction.
- Permission denial.
- Sandbox block.

## `sakha-daemon`

### Files

- `src/main.rs`
- `src/api.rs`
- `src/ws.rs`
- `src/sse.rs`
- `src/session_routes.rs`
- `src/loop_routes.rs`
- `src/permission_routes.rs`
- `src/state.rs`

### Types

- `DaemonState`
- `ApiError`
- `EventStream`
- `SessionRoutes`

### Tests

- Health endpoint.
- Session create.
- SSE reconnect.
- Permission decision.


## `sakha-context`

### Files

- `src/planner.rs`
- `src/budgeter.rs`
- `src/attribution.rs`
- `src/prompt/assembler.rs`
- `src/prompt/skills.rs`
- `src/prompt/templates.rs`
- `src/lib.rs`

### Types

- `ContextPlanner`
- `ContextBundle`
- `ContextBudgeter`
- `PromptAssembler`
- `AssembledPrompt`
- `PromptSection`
- `SkillPack`

### Tests

- Golden prompt assembly.
- Priority trimming under budget.
- Skill override precedence.
- Cache prefix stability.

## `sakha-mcp`

### Files

- `src/client.rs`
- `src/transport.rs`
- `src/server_registry.rs`
- `src/tool_bridge.rs`
- `src/trust.rs`
- `src/lib.rs`

### Types

- `McpClient`
- `McpTransport`
- `McpServerConfig`
- `McpToolBridge`
- `TrustPolicy`

### Tests

- Handshake/initialize.
- Tool list → registry bridge.
- Untrusted server blocked.
- Transport reconnect.

## `sakha-observability`

### Files

- `src/logging.rs`
- `src/spans.rs`
- `src/metrics.rs`
- `src/cost_ledger.rs`
- `src/audit_export.rs`
- `src/lib.rs`

### Types

- `ObservabilityConfig`
- `CostLedger`
- `TokenAccounting`
- `AuditExporter`

### Tests

- Cost accumulation per session/loop.
- Audit export round-trip.
- Redacted fields never logged.

## `sakha-cli`

### Files

- `src/main.rs`
- `src/commands/run.rs`
- `src/commands/chat.rs`
- `src/commands/session.rs`
- `src/commands/loops.rs`
- `src/commands/config.rs`
- `src/output.rs`

### Types

- `Cli` (clap root)
- `RunArgs`
- `ChatArgs`
- `OutputFormat`

### Tests

- Arg parsing.
- Non-interactive run exit codes.
- JSON output mode schema.

## `sakha-tui`

### Files

- `src/main.rs`
- `src/app.rs`
- `src/views/session.rs`
- `src/views/tools.rs`
- `src/views/loops.rs`
- `src/event.rs`

### Types

- `TuiApp`
- `View`
- `KeyMap`

### Tests

- Event → state reduction.
- Session timeline rendering (snapshot).
