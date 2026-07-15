# Module 06: Context, Memory, and State

## Responsibility

Persist durable state across context windows, sessions, loops, and agents. Build model context from relevant state instead of dumping everything into prompts.

## Rust Crate

`crates/sakha-memory` and `crates/sakha-context`

## Main Structs

- `MemoryStore`
- `ContextPlanner`
- `ContextBudgeter`
- `SessionMemory`
- `ProjectMemory`
- `LoopMemory`
- `ResearchMemory`
- `DecisionRecord`
- `HandoffArtifact`
- `Summary`
- `MemoryIndex`
- `RelevanceScore`

## Memory Types

- Conversation history.
- User preferences.
- Project facts.
- Architecture facts.
- Decisions.
- Open tasks.
- Completed tasks.
- Failed attempts.
- Tool traces.
- Web research sources.
- Compression retrieval markers.
- Loop handoffs.
- Evals and regressions.

## Storage

- SQLite tables for metadata and search.
- Artifact store for large text/binary.
- Optional vector index for semantic retrieval.
- Optional Tantivy full-text index for local search.

## Context Assembly Strategy

1. Load current goal and plan.
2. Load last handoff.
3. Load touched files summary.
4. Load relevant project facts.
5. Load relevant research evidence.
6. Load recent tool outcomes.
7. Apply budget.
8. Compress context.
9. Emit context bundle with source refs.

## Main APIs

- `write_memory(record: MemoryRecord)`
- `search_memory(query: MemoryQuery) -> Vec<MemoryHit>`
- `summarize_session(session_id) -> Summary`
- `write_handoff(goal_id, handoff)`
- `load_handoff(goal_id) -> HandoffArtifact`
- `build_context(request: ContextBuildRequest) -> ContextBundle`

## Handoff Artifact Required Fields

- Objective.
- Current status.
- Completed work.
- Pending work.
- Files changed.
- Commands run.
- Test results.
- Decisions made.
- Blockers.
- Next suggested action.
- Compression/retrieval notes.

## Implementation Tasks

1. Define SQLite schema.
2. Implement migrations.
3. Implement artifact storage.
4. Implement memory write/search.
5. Implement handoff writer.
6. Implement context planner.
7. Implement relevance scoring.
8. Implement memory pruning/retention.
9. Implement import/export.
10. Implement UI timeline endpoints.

## Tests

- Session resumes with handoff.
- Context budgeter excludes low-value records.
- Research sources are deduplicated.
- Memory search returns relevant facts.
- Retention policy deletes old raw artifacts.
- Handoff is sufficient for a fresh agent to continue.

