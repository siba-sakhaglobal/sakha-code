# Phase 04: Agent Loop

## Goal

Build the main model-tool loop.

## Tasks

### 04.1 Agent State Machine

- States:
  - initializing
  - building_context
  - calling_model
  - awaiting_tool
  - executing_tool
  - verifying
  - completing
  - blocked
  - failed

### 04.2 Message Assembly

- Load system policy.
- Load relevant memory.
- Load workspace context.
- Include tool specs.
- Apply compression.

### 04.3 Tool Call Loop

- Parse tool calls.
- Validate tool inputs.
- Execute tools.
- Feed results back.
- Stream updates to UI.

### 04.4 Termination Guards

- Max iterations.
- Max cost.
- Max wall time.
- Repeated action detection.
- Repeated error detection.
- No-progress detector.

### 04.5 Handoff

- Write handoff after every run.
- Include status, touched files, commands, results, next step.

## Definition of Done

- Simple Q&A works.
- One tool call task works.
- Multi-tool task works.
- Repeated failure stops safely.
- Handoff allows resumed session.

