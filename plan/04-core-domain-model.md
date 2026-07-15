# Core Domain Model

This file names the main modules, structs, traits, events, and persisted entities. Names are proposed Rust-style names.

## Crate Layout

```text
crates/
  sakha-core/
  sakha-agent/
  sakha-provider/
  sakha-tools/
  sakha-context/
  sakha-compression/
  sakha-memory/
  sakha-loop/
  sakha-mcp/
  sakha-security/
  sakha-ui-protocol/
  sakha-daemon/
  sakha-cli/
  sakha-tui/
  sakha-evals/
```

## Core Traits

### `AgentRuntime`

Owns lifecycle for an agent session.

Methods:

- `start_session(request: StartSessionRequest) -> SessionHandle`
- `resume_session(session_id: SessionId) -> SessionHandle`
- `run_turn(input: UserInput) -> TurnResult`
- `cancel(reason: CancelReason)`
- `checkpoint() -> CheckpointId`
- `shutdown()`

### `ProviderClient`

Normalizes all model providers.

Methods:

- `complete(request: ModelRequest) -> ModelResponse`
- `stream(request: ModelRequest) -> Stream<ModelEvent>`
- `count_tokens(request: TokenCountRequest) -> TokenCountResult`
- `capabilities() -> ProviderCapabilities`

### `Tool`

All tools implement a common contract.

Fields:

- `name`
- `description`
- `input_schema`
- `output_schema`
- `permission_spec`
- `idempotency_policy`

Methods:

- `validate(input: JsonValue) -> ValidatedInput`
- `plan(input: ValidatedInput, context: ToolContext) -> ToolPlan`
- `execute(input: ValidatedInput, context: ToolContext) -> ToolResult`
- `summarize(result: ToolResult) -> ToolSummary`

### `ContextCompressor`

Routes context through Headroom or fallback compressors.

Methods:

- `classify(item: ContextItem) -> ContentKind`
- `compress(item: ContextItem, policy: CompressionPolicy) -> CompressedContextItem`
- `retrieve(marker: CompressionMarker) -> ContextItem`
- `stats(scope: CompressionScope) -> CompressionStats`

### `LoopController`

Runs long-running loops safely.

Methods:

- `create_loop(spec: LoopSpec) -> LoopId`
- `tick(loop_id: LoopId) -> LoopTickResult`
- `pause(loop_id: LoopId)`
- `resume(loop_id: LoopId)`
- `stop(loop_id: LoopId, reason: StopReason)`
- `detect_stall(loop_id: LoopId) -> StallReport`

### `Verifier`

Checks agent work.

Methods:

- `verify_artifact(artifact: ArtifactRef, rubric: Rubric) -> VerificationResult`
- `verify_diff(diff: DiffRef, checks: Vec<CheckSpec>) -> VerificationResult`
- `grade_turn(turn: TurnRecord) -> Grade`

## Main Entities

### Session

```text
Session {
  id: SessionId,
  workspace_id: WorkspaceId,
  goal_id: Option<GoalId>,
  status: SessionStatus,
  provider_profile: ProviderProfileId,
  created_at,
  updated_at,
  budget: Budget,
  checkpoint_id: Option<CheckpointId>
}
```

### Turn

```text
Turn {
  id: TurnId,
  session_id: SessionId,
  input: UserInput,
  model_request_id: Option<ModelRequestId>,
  tool_calls: Vec<ToolCallId>,
  output: AssistantOutput,
  usage: UsageRecord,
  verification: Option<VerificationResult>,
  compression_stats: CompressionStats
}
```

### ToolCall

```text
ToolCall {
  id: ToolCallId,
  turn_id: TurnId,
  tool_name: ToolName,
  input_json: JsonValue,
  status: ToolCallStatus,
  permission_decision: PermissionDecision,
  started_at,
  completed_at,
  raw_output_ref: ArtifactRef,
  compressed_output_ref: Option<ArtifactRef>
}
```

### Goal

```text
Goal {
  id: GoalId,
  title: String,
  objective: String,
  state: GoalState,
  plan: Plan,
  budget: Budget,
  acceptance_criteria: Vec<Criterion>,
  handoff_artifact: ArtifactRef
}
```

### LoopSpec

```text
LoopSpec {
  id: LoopId,
  trigger: Trigger,
  objective: String,
  loop_kind: LoopKind,
  schedule: Option<CronSpec>,
  event_source: Option<EventSource>,
  max_iterations: u32,
  max_wall_time,
  max_cost,
  verifier: VerifierSpec,
  handoff_policy: HandoffPolicy
}
```

### ContextItem

```text
ContextItem {
  id: ContextItemId,
  source: ContextSource,
  kind: ContentKind,
  trust_level: TrustLevel,
  raw_artifact: ArtifactRef,
  compressed_artifact: Option<ArtifactRef>,
  retrieval_marker: Option<CompressionMarker>,
  token_estimate_raw: u64,
  token_estimate_compressed: u64
}
```

## Event Types

- `SessionStarted`
- `TurnStarted`
- `ModelRequestStarted`
- `ModelStreamDelta`
- `ToolCallRequested`
- `PermissionRequested`
- `ToolCallStarted`
- `ToolCallCompleted`
- `ContextCompressed`
- `CompressionRetrievalRequested`
- `VerificationStarted`
- `VerificationCompleted`
- `LoopTickStarted`
- `LoopTickCompleted`
- `CheckpointWritten`
- `BudgetExceeded`
- `StallDetected`
- `SessionCompleted`
- `SessionBlocked`

## Persistence Tables

- `workspaces`
- `sessions`
- `turns`
- `messages`
- `model_requests`
- `tool_calls`
- `artifacts`
- `context_items`
- `compression_markers`
- `goals`
- `plans`
- `loop_specs`
- `loop_ticks`
- `verification_results`
- `permissions`
- `audit_events`
- `provider_profiles`
- `secrets_metadata`
- `memory_records`

