# Sakha Agent Architecture — AS-BUILT

**Date**: 2025-07-10  
**Version**: 0.1.0  
**Workspace**: `e:/Project Git/GitHub/sakha`

---

## Executive Summary

Sakha is a Rust-based autonomous coding agent orchestration framework. It implements a bounded ReAct (Reasoning-Acting) loop with streaming LLM provider integration, structured tool execution, context budgeting, compression, and session persistence. The architecture emphasizes safety (permission gates, sandbox checks, idempotency), observability (event envelopes, audit trails), and modularity (15 crates, clear dependency boundaries).

---

## 1. Crate Dependency Graph

```
┌─────────────────────────────────────────────────────────────────┐
│ Presentation Layer                                              │
├─────────────────────────────────────────────────────────────────┤
│  sakha-cli          sakha-daemon         sakha-tui             │
│  (CLI commands)     (HTTP API server)    (TUI interface)       │
└────────┬──────────────────┬──────────────────────┬──────────────┘
         │                  │                      │
┌────────▼──────────────────▼──────────────────────▼──────────────┐
│ Orchestration & State Layer                                     │
├─────────────────────────────────────────────────────────────────┤
│  sakha-loop (loop specs, scheduler, tick executor, watchdog)   │
│  sakha-agent (agent loop, turn execution, policy, termination) │
└────────┬──────────────────────────────────────────┬─────────────┘
         │                                          │
┌────────▼──────────────────────────────────────────▼─────────────┐
│ Core Collaborators                                              │
├─────────────────────────────────────────────────────────────────┤
│  sakha-provider          sakha-tools          sakha-security   │
│  (LLM gateway,           (registry,           (permissions,    │
│   streaming, cost)       execution pipeline)  sandbox, secrets) │
│                                                                  │
│  sakha-context           sakha-compression    sakha-memory     │
│  (prompt planning,       (headroom adapter,   (SQLite stores:  │
│   budgeting,             local fallback)      sessions,        │
│   attribution)                                handoffs,        │
│                                               artifacts,       │
│  sakha-mcp               sakha-research       research)        │
│  (MCP server bridge)     (web fetches,                         │
│                          source tracking)                      │
│                                                                  │
│  sakha-observability                                           │
│  (audit log, cost ledger, tracing, metrics)                   │
└────────┬──────────────────────────────────────────┬─────────────┘
         │                                          │
┌────────▼──────────────────────────────────────────▼─────────────┐
│ sakha-core (no internal dependencies)                           │
├─────────────────────────────────────────────────────────────────┤
│  IDs (SessionId, GoalId, ToolCallId, TurnId, etc.)            │
│  Errors (SakhaError, ErrorClass taxonomy)                     │
│  Budget (limits, dimensions, ledger with atomics)              │
│  Events (EventEnvelope, EventKind, global sequence)            │
│  Artifacts (ArtifactRef, ArtifactKind, content_hash_hex)      │
│  Retry policy (ExponentialBackoff, RetryPolicy)               │
│  Runtime (CancellationToken, EventBus, EventStream)            │
│  Time (now_utc helpers)                                        │
└─────────────────────────────────────────────────────────────────┘
```

**Key Dependency Rules**:
- **No circular dependencies**: verified by `workspace resolver = "2"`.
- **Downward only**: UI → Daemon → Agent/Loop → Tools/Provider/Context/Security/Memory → Core.
- **Core is leaf**: zero internal workspace dependencies; uses only standard/external crates.
- **Trait-based collaboration**: agent doesn't hardcode provider/compressor implementations; all are injected via `AgentDeps`.

---

## 2. Runtime Flow: One Agent Turn

This traces a single user input through the entire agent→model→tool loop.

### 2.1 Entry Point: CLI

```rust
// crates/sakha-cli/src/main.rs:114-145
fn main() {
    let cli = Cli::parse();
    let exit_code = match cli.command {
        Command::Run(args) => commands::run::execute(args),
        Command::Chat(args) => commands::chat::execute(args),
        // ... other commands
    };
    std::process::exit(exit_code);
}
```

Dispatch: `run`/`chat`/`session`/`loop`/`providers`/`tools`/`memory`/`mcp`/`config`/`eval`/`daemon`/`doctor`.

### 2.2 `run` Command → Agent Turn

**File**: `crates/sakha-cli/src/commands/run.rs` (via `crates/sakha-cli/src/runtime.rs`)

```
1. Parse args → extract user prompt + optional config overrides
2. Load config (SakhaConfig from ~/.sakha/config.toml or defaults)
3. Build collaborators:
   - ProviderClient (MockProviderClient by default; OpenAiCompatibleClient if configured)
   - SessionStore (SqliteSessionStore if db_path set, else InMemorySessionStore)
   - ToolRegistry (sakha_tools::default_registry() with all built-ins)
   - PermissionPolicy (permissive-by-default for CLI)
4. Create or resume session:
   - SessionRecord { id, workspace_id, goal_id, status, provider_profile, created/updated_at }
   - Persist to SessionStore
5. Spawn Agent + run one turn:
   - new(session_id, LoopType::Interactive, config, deps)
   - agent.run_turn(user_input) → LoopDecision
6. Handle result:
   - LoopDecision::Stop { reason } → finish, print reason
   - LoopDecision::Blocked { reason, next_action } → print human review prompt
```

### 2.3 Agent::run_turn() — The Core Loop

**File**: `crates/sakha-agent/src/agent.rs:168-240+`

```
TURN START
│
├─ Phase = Planning
├─ Create TerminationGuard(max_failures, max_iterations)
├─ Seed messages from user_input + prior turn history
│
├─ LOOP {
│    │
│    ├─ [Termination Check]
│    │   ├─ guard.evaluate(&state)
│    │   │   ├─ check: consecutive_failures >= max?
│    │   │   ├─ check: iterations >= max?
│    │   │   └─ LoopDecision::Stop { reason }
│    │   │
│    │   ├─ policy.should_continue(&state)
│    │   │   └─ DefaultAgentPolicy checks: phase == Completed?
│    │   │
│    │   └─ budget.ledger.is_exhausted(LoopIterations)?
│    │
│    ├─ Debit budget: BudgetDimension::LoopIterations -= 1
│    │
│    ├─ Phase = AwaitingModel
│    │
│    ├─ [Context Assembly] (if not yet cached)
│    │   ├─ context_planner.build_context(ContextBuildRequest {
│    │   │    session_id, user_input, prior_turns, ...
│    │   │  })
│    │   │   ├─ DefaultContextPlanner pulls:
│    │   │   │   ├─ User input (high priority)
│    │   │   │   ├─ Session memory hits
│    │   │   │   ├─ Research feeds
│    │   │   │   └─ Prior turn handoff artifacts
│    │   │   │
│    │   │   └─ Returns ContextBundle {
│    │   │        items: [ { content, priority, token_estimate, attribution } ]
│    │   │      }
│    │   │
│    │   ├─ [Budget: Context Trimming]
│    │   │   └─ budgeter.trim(bundle) → keep high-priority, drop low-priority
│    │   │
│    │   └─ compressor.compress(item) for each item
│    │       ├─ PassthroughCompressor (default): returns unchanged
│    │       ├─ HeadroomBackedCompressor: calls external sidecar
│    │       │   (if unavailable: falls back to LocalFallbackCompressor)
│    │       └─ LocalFallbackCompressor: head+tail lines + marker
│    │
│    ├─ [Prompt Assembly]
│    │   └─ prompt_assembler.assemble(PromptRequest {
│    │        agent_role, tools, memory_digest, user_input, skills, ...
│    │      })
│    │      ├─ DefaultPromptAssembler
│    │      └─ Returns AssembledPrompt (system + context sections + user query)
│    │
│    ├─ [Model Request]
│    │   └─ allowed_tools = policy.allowed_tools(&state)
│    │   │   (filters registry by risk level + permission grants)
│    │   │
│    │   └─ provider.complete(ModelRequest {
│    │       messages: [ { role, content, tool_call_id } ],
│    │       tools: [ ToolDefinition { name, description, schema } ],
│    │       model, temperature, max_tokens, ...
│    │      })
│    │      ├─ OpenAiCompatibleClient.complete()
│    │      │   ├─ POST to configured base_url/v1/chat/completions
│    │      │   ├─ Parse SSE stream: ModelEventStream
│    │      │   ├─ Assemble ToolCallDelta → AssembledToolCall
│    │      │   └─ Return ModelResponse { text, tool_calls, usage, stop_reason }
│    │      │
│    │      └─ Debit budget:
│    │          ├─ BudgetDimension::InputTokens -= response.usage.input_tokens
│    │          └─ BudgetDimension::OutputTokens -= response.usage.output_tokens
│    │
│    ├─ [No Tool Calls → Complete]
│    │   if response.tool_calls.is_empty() || stop_reason == EndTurn
│    │   ├─ phase = Completed
│    │   ├─ last_response_text = response.text
│    │   ├─ consecutive_failures = 0
│    │   └─ BREAK → finish()
│    │
│    ├─ [Tool Execution Phase]
│    │   ├─ phase = ExecutingTools
│    │   │
│    │   ├─ Parse & validate JSON arguments
│    │   │   └─ Malformed args → tool_error_message(), exclude from execution
│    │   │
│    │   ├─ For each valid (ExecutorToolCall { id, tool_name, input_json }):
│    │   │   │
│    │   │   ├─ [Parallel or Sequential dispatch]
│    │   │   │   if config.allow_parallel_tool_calls && len > 1:
│    │   │   │   │   └─ tokio::JoinSet for concurrent execution
│    │   │   │   │       (each in its own ToolExecutor instance)
│    │   │   │   else:
│    │   │   │   │   └─ Sequential for loop
│    │   │   │
│    │   │   └─ ToolExecutor::execute(call, ToolContext { workspace_root })
│    │   │       (see § 2.4 below)
│    │   │
│    │   ├─ Collect results keyed by call index (preserves model's order)
│    │   │
│    │   ├─ [Post-Process Results in Model Order]
│    │   │   for (idx, response.tool_calls[idx]):
│    │   │   │
│    │   │   ├─ result OK:
│    │   │   │   ├─ consecutive_failures = 0
│    │   │   │   ├─ record_tool_effects(&result) → update state/plan
│    │   │   │   ├─ policy.on_tool_result(result)
│    │   │   │   │   ├─ PolicyAction::Stop → phase=Blocked, return
│    │   │   │   │   ├─ PolicyAction::Continue → proceed
│    │   │   │   │   └─ PolicyAction::EscalateToHuman → block + log
│    │   │   │   │
│    │   │   │   ├─ summarize_result(&result)
│    │   │   │   │   ├─ Cap at MAX_TOOL_RESULT_CHARS = 4000
│    │   │   │   │   ├─ If longer: truncate + add "[... truncated]" marker
│    │   │   │   │   └─ Prevent unbounded message growth
│    │   │   │   │
│    │   │   │   └─ Append ToolMessage to messages:
│    │   │   │       { role=Tool, content=summary, tool_call_id, name }
│    │   │   │
│    │   │   └─ result Err:
│    │   │       ├─ consecutive_failures += 1
│    │   │       ├─ Append error message
│    │   │       └─ Continue (may trigger stop if failures >= max)
│    │   │
│    │   └─ LOOP back to model call (tool results fed as new context)
│    │
│    └─ } LOOP
│
├─ finish(decision, user_input)
│   ├─ Write TurnRecord to SessionStore
│   ├─ Write HandoffArtifact if goal-oriented
│   ├─ Emit SessionCompleted/SessionBlocked event
│   └─ Return LoopDecision
│
TURN END → return to CLI
```

### 2.4 Tool Execution Pipeline

**File**: `crates/sakha-tools/src/executor.rs`

For each `ToolCall { id, tool_name, input_json }`:

```
1. [Lookup in Registry]
   └─ ToolRegistry::get(tool_name)
      └─ Returns Arc<dyn Tool> or error

2. [Validate Input JSON]
   └─ tool.schema().validate(input_json)?

3. [Risk Classification]
   └─ RiskClassifier::classify(tool_name, input_json)
      └─ Returns RiskLevel { Low, Medium, High, Critical }

4. [Permission Check]
   ├─ PermissionPolicy::check(tool_name, risk_level)
   │   ├─ Lookup explicit grant in permission_policy.rules
   │   ├─ If not found: auto-allow if risk_level == Low, else deny
   │   └─ Return: Allowed | Denied { reason }
   │
   └─ On Denied:
       └─ Emit PermissionRequested event
       └─ Return error (CLI/daemon must handle approval flow)

5. [Idempotency Cache]
   ├─ key = content_hash_hex(tool_name || input_json)
   ├─ Check idempotency_cache.get(key)
   ├─ If hit:
   │   ├─ was_deduped = true
   │   └─ Return cached ToolResult
   └─ If miss: proceed to execute

6. [Sandbox Validation] (security-check only, not real isolation)
   ├─ SandboxProfile (from tool spec or default)
   ├─ NoopSandbox::validate(SandboxRequest {
   │    profile, workspace_root, write_paths, needs_network
   │  })
   │  ├─ Check: network access allowed by profile?
   │  ├─ Check: writes are read-only compliant?
   │  ├─ Check: write_paths stay within workspace_root?
   │  │   (lexically normalize; block `../../../etc/passwd` escapes)
   │  └─ Return: Ok | Err(reason)
   │
   └─ On Err: return SandboxOutcome::blocked(reason)

7. [Execute Tool]
   ├─ tool.execute(validated_input, ToolContext { workspace_root })
   │   ├─ Built-in tools:
   │   │   ├─ file::{read,write,patch,delete,move,list}
   │   │   ├─ git::{status,diff,apply_patch,branch,worktree,commit}
   │   │   ├─ shell::run
   │   │   └─ search::ripgrep
   │   │
   │   └─ Each tool is Arc<dyn Tool>; all non-panicking
   │
   ├─ Capture output:
   │   ├─ stdout, stderr, exit_code
   │   └─ Store as artifact (ArtifactRef with content_hash)
   │
   └─ Return ToolResult { status, output, artifacts, metadata }

8. [Audit Record]
   └─ Write ToolAuditRecord to audit_log:
       {
         call_id, tool_name, started: true, completed: true,
         risk_level, was_deduped, raw_output_ref
       }

9. [Result Summary & Compression]
   ├─ ToolResult flows back to agent
   ├─ Compress output (optional, via agent's compressor)
   │   ├─ If oversized: LocalFallbackCompressor creates marker + artifact
   │   └─ Result includes marker for later retrieval
   └─ Return to agent for message assembly
```

---

## 3. Event & Budget Flow

### 3.1 Events

**File**: `crates/sakha-core/src/event.rs`

Every significant operation emits an `EventEnvelope`:

```rust
pub struct EventEnvelope {
    pub sequence: u64,              // Global monotonic counter (process-wide)
    pub session_id: Option<SessionId>,
    pub kind: EventKind,            // Discriminant
    pub occurred_at: DateTime<Utc>,
    pub payload: serde_json::Value,
}
```

**EventKind variants**:
- `SessionStarted`, `SessionCompleted`, `SessionBlocked`
- `TurnStarted`
- `ModelRequestStarted`, `ModelStreamDelta`
- `ToolCallRequested`, `ToolCallStarted`, `ToolCallCompleted`
- `PermissionRequested`
- `ContextCompressed`, `CompressionRetrievalRequested`
- `VerificationStarted`, `VerificationCompleted`
- `LoopTickStarted`, `LoopTickCompleted`
- `CheckpointWritten`
- `BudgetExceeded`, `StallDetected`

**Event Flow**:
1. Events are posted to `sakha_core::EventBus` (trait: `EventSink`, `EventStream`).
2. Listeners (CLI, daemon, loops) consume via `EventStream::next()`.
3. `sakha-observability` records events to `AuditLog` (append-only, in-memory).
4. `AuditExporter` builds `SessionReport` at session end.

### 3.2 Budget

**File**: `crates/sakha-core/src/budget.rs`

```rust
pub struct Budget {
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub max_cost_micros: Option<u64>,    // 1e-6 USD
    pub max_tool_calls: Option<u64>,
    pub max_wall_time: Option<Duration>,
    pub max_loop_iterations: Option<u64>,
    pub max_filesystem_writes: Option<u64>,
}

pub struct BudgetLedger {
    limits: Budget,
    // Atomic counters (no external lock needed):
    input_tokens: AtomicU64,
    output_tokens: AtomicU64,
    cost_micros: AtomicU64,
    tool_calls: AtomicU64,
    wall_time_millis: AtomicU64,
    loop_iterations: AtomicU64,
    filesystem_writes: AtomicU64,
}
```

**Ledger Operations**:
- `debit(dimension, amount)` → CAS loop (up to `MAX_CAS_ATTEMPTS=10_000`)
- `credit(dimension, amount)` → refund (for retries, etc.)
- `is_exhausted(dimension)` → check limit

**Where Debited**:
- `agent.run_turn()` debits `LoopIterations`, `InputTokens`, `OutputTokens`
- `provider.complete()` debits `InputTokens`, `OutputTokens`
- Tool execution debits `ToolCalls`
- Budget errors trigger `LoopDecision::Stop { reason: "budget exhausted" }`

---

## 4. Permission Flow

**File**: `crates/sakha-security/src/lib.rs` + `crates/sakha-tools/src/permissions.rs`

```
PermissionPolicy {
    rules: HashMap<ToolName, RiskLevel → Allow | Deny>
}

Tool Execution:
1. RiskClassifier::classify(tool_name, input_json)
   ├─ Queries tool spec for risk assessment rules
   └─ Returns: Low | Medium | High | Critical

2. PermissionPolicy::check(tool_name, risk_level)
   ├─ If explicit rule exists: return decision
   ├─ If no rule:
   │   └─ Low → auto-allow (safe by default)
   │   └─ Medium+ → return Denied
   │
   └─ On Denied: emit PermissionRequested event
                 (CLI/daemon prompts user or blocks)

3. SandboxAdapter::validate(SandboxRequest)
   ├─ NoopSandbox (default):
   │   ├─ Check profile allows network? (yes/no)
   │   ├─ Check profile allows writes? (yes/no)
   │   ├─ Check writes stay in workspace_root?
   │   └─ Return Ok | Err(reason)
   │
   └─ Future: OS-level backends (Linux seccomp, macOS sandbox-exec, etc.)

Default Policy (CLI mode):
- permissive_by_default: Low-risk auto-allow
- Projects layer workspace/org policy via PermissionPolicy::with_layer()
```

---

## 5. Memory & State Persistence

**File**: `crates/sakha-memory/`

### 5.1 Stores (Trait-Based)

```rust
// SessionStore: session lifecycle
pub trait SessionStore: Send + Sync {
    async fn create_session(&self, record: SessionRecord) -> SakhaResult<()>;
    async fn get_session(&self, id: SessionId) -> SakhaResult<Option<SessionRecord>>;
    async fn update_session_status(&self, id: SessionId, status: SessionStatus) -> SakhaResult<()>;
    async fn list_turns(&self, session_id: SessionId) -> SakhaResult<Vec<TurnRecord>>;
    async fn append_turn(&self, record: TurnRecord) -> SakhaResult<()>;
}

// HandoffStore: inter-turn/inter-loop continuity
pub trait HandoffStore: Send + Sync {
    async fn write_handoff(&self, goal_id: GoalId, artifact: HandoffArtifact) -> SakhaResult<()>;
    async fn load_handoff(&self, goal_id: GoalId) -> SakhaResult<Option<HandoffArtifact>>;
}

// ArtifactStore: result storage (file blobs, tool outputs)
pub trait ArtifactStore: Send + Sync {
    async fn put(&self, kind: ArtifactKind, data: Vec<u8>, metadata: Option<Value>) -> SakhaResult<ArtifactRef>;
    async fn get(&self, artifact_ref: &ArtifactRef) -> SakhaResult<Option<Vec<u8>>>;
}

// ResearchStore: web fetch results
pub trait ResearchStore: Send + Sync {
    async fn upsert_source(&self, record: ResearchSourceRecord) -> SakhaResult<()>;
    async fn list_sources_for_goal(&self, goal_id: GoalId) -> SakhaResult<Vec<ResearchSourceRecord>>;
}

// MemoryStore: semantic search over session + research
pub trait MemoryStore: Send + Sync {
    async fn index(&self, query: &str, kind: MemoryKind) -> SakhaResult<Vec<MemoryHit>>;
}
```

### 5.2 Implementations

- **In-Memory** (tests, offline mode):
  - `InMemorySessionStore`, `InMemoryHandoffStore`, `InMemoryArtifactStore`, `InMemoryResearchStore`
  - Thread-safe via `Arc<Mutex<_>>`

- **SQLite** (production):
  - `SqliteSessionStore`, `SqliteHandoffStore`, `SqliteArtifactStore`, `SqliteResearchStore`
  - `Database::open(path)` → `rusqlite::Connection`
  - Blocking operations dispatched to thread pool by callers
  - Schema versioned via migrations (in `crates/sakha-memory/src/migrations/`)

### 5.3 Session Resume

When user runs `sakha chat --session-id <id>`:

```
1. CLI loads config → build_session_store(config)
2. SessionStore::get_session(id)
3. If Some(record):
   ├─ Check status: Active | Paused | Completed | Blocked
   ├─ Load turn history via SessionStore::list_turns(id)
   ├─ Load handoff via HandoffStore::load_handoff(goal_id)
   │   └─ HandoffArtifact includes:
   │       ├─ current_status, completed_work, pending_work
   │       ├─ files_changed, commands_run, test_results
   │       ├─ decisions_made, blockers
   │       └─ next_suggested_action
   │
   ├─ Spawn new agent with same config
   ├─ Pre-populate agent.state.plan, agent.state.past_turns from handoff
   └─ run_turn() continues from next_suggested_action
4. If None: 404 error
```

---

## 6. Known Limitations & Future Work

### 6.1 Sandbox: NoopSandbox Only

**Current State**:
- `sakha-security/src/sandbox.rs:1-33` documents it explicitly.
- `NoopSandbox` performs **pure-Rust checks only**:
  - Path containment (lexical normalization + `starts_with`)
  - Network gate (boolean check against profile)
  - Write permission (boolean check against profile)

- **Does NOT provide**:
  - OS-level process isolation (no Linux namespaces/seccomp, no macOS sandbox-exec, no Windows job objects)
  - Real filesystem containment (a subprocess can still do arbitrary syscalls)
  - Network isolation (checked at tool-level, not enforced on subprocess)

**Failure Mode**:
```rust
// NoopSandbox::run() always returns Err(not_implemented(...))
// because there is no real process executor backing it.
```

**Future Work** (documented in sandbox.rs:18-32):
- **Linux**: `unshare`/`bwrap`/`landlock`, seccomp-bpf filter
- **macOS**: `sandbox-exec` with `.sb` profile
- **Windows**: restricted job object + ACL + WFP (Windows Filtering Platform)

**Impact**: Tools run with full user privileges. Safe only in trusted environments (local dev, private orgs). Not suitable for multi-tenant SaaS without additional hardening.

### 6.2 Compression: Fallback Chain Only

**Current State**:
- `sakha-compression/src/lib.rs:1-5`
- Default: `PassthroughCompressor` (no-op, returns content unchanged)
- If enabled, tries `HeadroomBackedCompressor` (calls external sidecar)
- If Headroom unavailable: falls back to `LocalFallbackCompressor`

**Headroom Fallback**:
```rust
// crates/sakha-compression/src/headroom.rs:58-62
pub struct UnavailableHeadroomClient;
impl HeadroomClient for UnavailableHeadroomClient {
    async fn compress(...) -> Result {
        Err(SakhaError::not_implemented("...Headroom unavailable"))
    }
}
```

When Headroom fails/missing:
1. `HeadroomBackedCompressor` catches `Err`
2. Falls back to `LocalFallbackCompressor`:
   - Head N lines + tail N lines of content
   - Creates `CompressionMarker` + stores original as artifact
   - Returns compressed version with marker

**Limitation**: Local fallback is lossy; no LLM-based compression. For high-volume token reduction, requires external Headroom deployment.

### 6.3 Loop Tick Executor: Stubbed

**Current State**:
```rust
// crates/sakha-loop/src/runtime.rs:258-267
pub struct NullTickExecutor;
impl TickExecutor for NullTickExecutor {
    async fn run_tick(...) -> Result {
        Err(SakhaError::not_implemented("...no executor wired"))
    }
}
```

Loop ticks (for `LoopKind::Goal`, `Repair`, `Research`, etc.) have no concrete executor yet. `LoopRuntime::tick()` will return `not_implemented` unless a custom `TickExecutor` is provided.

**Workaround**: CLI's `loop` commands can manage loop specs, but ticking requires external wiring (daemon, or future `sakha-team` crate).

### 6.4 MCP Integration: Adapter Provided, Not Automated

**Current State**:
- `sakha-mcp/src/` provides `HeadroomMcpClient`, `McpToolInvoker`
- Allows registering MCP servers as tools
- No automatic discovery or startup

**Gap**: Must be explicitly configured and connected by the application layer (daemon/CLI).

### 6.5 Multi-Tenancy & Team Coordination: Minimal

**Current State**:
- Single-session model. Sessions are isolated in-memory or per SQLite database.
- Sub-agents (`Agent::spawn_subagent()`) get isolated workspace dirs (`<root>/.sakha/subagents/<session_id>`)
- No lease-based concurrent access to same workspace.

**Future** (`sakha-team` crate planned in spec module 16):
- Workspace locks / lease-based coordination
- Sub-agent permission inheritance / isolation policies
- Multi-workspace governance

### 6.6 Verification Loop: Checker Only

**Current State**:
- `sakha-loop/src/verification_loop.rs` defines `CheckSpec`, `Rubric`, `Grade`
- `CommandVerifier` runs shell checks and scores results
- Loop can be configured with a `VerifierSpec`

**Gap**: No automated re-planning on failed checks; requires external (human or daemon-level) decision logic.

### 6.7 Parallel Tool Calls: Experimental

**Current State**:
```rust
// crates/sakha-agent/src/agent.rs:288-318
if config.allow_parallel_tool_calls && runnable.len() > 1 {
    let mut join_set = tokio::task::JoinSet::new();
    for (idx, tool_call) in runnable { ... }
}
```

Works; results are collected and post-processed in model's original order. **Caveat**: if two concurrent tool calls race on the same file, outcome is undefined. No automatic conflict detection or versioning.

### 6.8 Cost Estimation: Placeholder

**Current State**:
- `sakha-provider/src/cost.rs` has `estimate_cost_micros(model, input_tokens, output_tokens)`
- Uses hardcoded pricing (OpenAI rates as of spec authorship)
- Actual cost computed at model response time, not predicted upfront

**Limitation**: Budget enforcement is by-token, not by-cost. Cost ledger is informational only.

---

## 7. Dependency Inventory

### Core Async Runtime
- `tokio` (1.x, features: "full")
- `tokio-stream` (0.1, for `StreamExt`)
- `tokio-util` (0.7)

### Serialization
- `serde` (1.x, derive)
- `serde_json` (1.x)

### Networking
- `reqwest` (0.12, rustls-tls, no default-features)
- `axum` (0.7, WebSocket support) — for daemon HTTP API
- `tower` (0.5) — middleware
- `tower-http` (0.5, CORS + tracing)

### Data Structures & Utilities
- `uuid` (1.x, v4 + serde)
- `chrono` (0.4, serde)
- `bytes` (1.x)
- `futures` (0.3)
- `similar` (2.x, for diff/patch)
- `regex` (1.x)
- `url` (2.x)
- `html2text` (0.12, for web content extraction)

### CLI & TUI
- `clap` (4.x, derive)
- `ratatui` (0.28, TUI framework)
- `crossterm` (0.28, terminal control)

### Persistence
- `rusqlite` (0.31, bundled SQLite)
- `tempfile` (3.x, for tests)
- `walkdir` (2.x, directory traversal)

### Error Handling
- `thiserror` (1.x)
- `anyhow` (1.x)

### Tracing & Observability
- `tracing` (0.1)
- `tracing-subscriber` (0.3, env-filter)

### Async Traits & Utilities
- `async-trait` (0.1)

### Filesystem & Config
- `dirs` (5.x, config dir paths)
- `toml` (0.8, config parsing)

### Random & Misc
- `rand` (0.8, for ID generation)

**Total**: ~40 external dependencies; no heavy frameworks (no web framework besides axum, no ORM beyond rusqlite, no async runtime besides tokio).

---

## 8. Key Design Decisions

### 8.1 Why NoopSandbox?

**Trade-off**: Simplicity + testability vs. security isolation.

- `NoopSandbox` makes every test runnable without OS-level setup.
- Serves as reference implementation for the `SandboxAdapter` trait.
- Real deployments will layer OS-level backends; the trait boundary is clean.

### 8.2 Why Trait-Based Providers, Compressors, Context Planners?

**Reason**: Testability + swappability.

- Mockable: `MockProviderClient`, `PassthroughCompressor`, `MinimalContextPlanner` for offline tests.
- Pluggable: Real deployments inject `OpenAiCompatibleClient`, `HeadroomBackedCompressor`, `DefaultContextPlanner`.
- No hardcoded "production" state; dependency injection via `AgentDeps`.

### 8.3 Why Atomic Counters for Budget Ledger?

**Reason**: Concurrent tool execution without external lock contention.

- `BudgetLedger` is `Arc<BudgetLedger>` shared across parallel tool tasks.
- CAS loop on atomic counters avoids mutex scalability cliff.
- Bounded retry loop (`MAX_CAS_ATTEMPTS`) prevents spin-forever adversarial inputs.

### 8.4 Why Event Envelopes with Global Sequence?

**Reason**: Total ordering for replay + audit.

- Process-wide monotonic counter ensures gap-free event log.
- Per-session subsequence is guaranteed by filtering on `session_id`.
- Enables deterministic replay of complex multi-session scenarios.

### 8.5 Why SQLite for Memory Stores?

**Reason**: Durability without operational complexity.

- Single-file database; no server to manage.
- Embedded via `rusqlite` (C binding, bundled SQLite).
- Blocking calls dispatched to thread pool by higher layers.
- Can be replaced by users with PostgreSQL/etc. via trait implementations.

---

## 9. Testing & CI Strategy

- **Unit tests**: In each crate's `#[cfg(test)]` modules (golden tests in `sakha-context`, `sakha-compression`, `sakha-memory`).
- **Integration tests**: In-memory implementations (`MockProviderClient`, `InMemorySessionStore`) used throughout.
- **Offline mode**: CLI defaults to `MockProviderClient`; no network required for smoke tests.
- **Real mode**: `sakha run --provider openai-compatible --base-url <url> --api-key-env OPENAI_API_KEY`.

No integration tests against real LLM providers in CI (cost + latency); manual/staging only.

---

## 10. Critical Path for Feature Development

1. **New Tool**: Implement `Tool` trait in `sakha-tools/src/builtins/`, register in `default_registry()`, add permission rules.
2. **New Provider**: Implement `ProviderClient` trait in `sakha-provider/src/`, update config enum.
3. **New Loop Type**: Add `LoopKind` variant in `sakha-loop/src/spec.rs`, wire executor in daemon.
4. **New Context Source**: Implement `ContextPlanner` trait in `sakha-context/`, inject via `AgentDeps`.
5. **New Event Handler**: Emit `EventEnvelope` at the site, listen via `EventStream` in observer layer.

All paths go through trait boundaries → safe refactoring.

---

## 11. Conclusion

Sakha is a well-structured, modular agent framework with clear concerns:

- **Orchestration** (agent, loop) depends on collaborators but doesn't dictate their implementations.
- **Safety** (budgets, permissions, sandboxing) is layered; hooks exist for OS-level upgrades.
- **Observability** (events, audit, cost) is wired in; can be replayed or exported.
- **Persistence** (SQLite + traits) enables session resume and multi-process coordination.
- **Testing** (in-memory, mock providers) is first-class; no hard production dependencies.

Known gaps (NoopSandbox, stubbed loop executor, cost estimation) are documented and don't block core workflows; they're clear extension points for future work.

The workspace is dependency-acyclic, language-idiomatic (Rust 2021, trait-driven, async-first), and production-ready for single-session/single-workspace deployments. Multi-tenancy, OS sandboxing, and advanced loop orchestration are deferred to future crates or external layers.
