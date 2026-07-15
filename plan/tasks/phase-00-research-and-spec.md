# Phase 00: Research and Spec

## Goal

Produce clean-room specifications and acceptance criteria before implementation.

## Inputs

- `../INDEX.md`
- `../references.md`
- Existing feature inventory from current repository.

## Tasks

### 00.1 Feature Inventory

- List every feature category.
- Mark each as `KEEP`, `REWRITE`, `ADAPT`, `DROP`, or `QUARANTINE`.
- Identify external dependencies.
- Identify legal/safety risks.

### 00.2 Behavior Specs

- Write user stories for interactive chat.
- Write user stories for file editing.
- Write user stories for long-running goal mode.
- Write user stories for web research.
- Write user stories for compression/retrieval.
- Write user stories for multi-agent worktrees.

### 00.3 Acceptance Criteria

- Define pass/fail criteria per module.
- Define smoke test scenario.
- Define security gates.
- Define performance targets.
- Define compression accuracy requirements.

### 00.4 Architecture Decision Records

Create ADRs for:

- Rust core.
- Tokio runtime.
- SQLite state.
- Tauri desktop.
- Provider-neutral gateway.
- Headroom integration mode.
- MCP extension boundary.

### 00.5 Eval Fixtures

- Create provider stream fixtures.
- Create tool-call fixtures.
- Create patch fixtures.
- Create compression fixtures.
- Create loop termination fixtures.
- Create web research fixtures.

## Deliverables

- `../specs/features/README.md`
- `../specs/adrs/000-template.md`
- `evals/fixtures/*`
- Updated module docs if gaps found.

## Definition of Done

- Every planned feature maps to a module.
- Every module has acceptance criteria.
- Every risky feature has a mitigation or quarantine decision.
- Implementation can begin without using old code as reference.
