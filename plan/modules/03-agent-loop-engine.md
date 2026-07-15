# Module 03: Agent Loop Engine

## Responsibility

Run the model-tool loop for interactive and autonomous tasks. Parse model events, execute tools, update state, and decide termination.

## Rust Crate

`crates/sakha-agent`

## Main Structs

- `Agent`
- `AgentConfig`
- `AgentState`
- `TurnContext`
- `MessageGraph`
- `ToolCallState`
- `LoopDecision`
- `TerminationGuard`
- `PlanState`
- `Scratchpad`
- `AgentHandoff`

## Main Traits

### `AgentPolicy`

- `system_prompt(ctx) -> PromptBlock`
- `allowed_tools(ctx) -> Vec<ToolName>`
- `should_continue(state) -> LoopDecision`
- `on_tool_result(result) -> PolicyAction`

### `ToolCallInterpreter`

- `parse(event) -> ToolCallDelta`
- `assemble(delta) -> ToolCall`
- `validate(call) -> ValidatedToolCall`

## Loop Types

- `InteractiveLoop`
- `GoalLoop`
- `RepairLoop`
- `ReviewLoop`
- `ResearchLoop`
- `VerificationLoop`
- `SubAgentLoop`

## Features

- ReAct-style tool loop.
- Plan-first mode.
- Streaming response to UI.
- Tool call execution.
- Parallel tool calls where safe.
- Tool result compression.
- Scratchpad/checkpoint update.
- Explicit stop reasons.
- Handoff artifact generation.
- Blocked-state reporting.
- Infinite-loop guard.

## Termination Criteria

- Model final answer.
- Goal acceptance criteria met.
- Verification passes.
- Budget exhausted.
- Human approval needed.
- Repeated failure.
- No-progress detector.
- Unsafe action blocked.
- External dependency unavailable.

## Implementation Tasks

1. Define `AgentState` state machine.
2. Implement message graph.
3. Implement prompt/context assembler.
4. Implement stream event loop.
5. Implement tool call parser and assembler.
6. Implement parallel-safe tool dispatch.
7. Implement no-progress detector.
8. Implement handoff writer.
9. Implement blocked/completed statuses.
10. Add fixture-based loop tests.

## Failure Modes

- Model emits invalid JSON tool input.
- Tool succeeds but result too large.
- Model repeats same failed action.
- Tool side effect happens before permission check.
- Verification passes incorrectly.
- Context compression hides required detail.

## Tests

- Simple answer no tools.
- One tool call then final answer.
- Multi-tool loop.
- Invalid tool call recovery.
- Budget stop.
- Repeated failure stop.
- Handoff includes plan, status, touched files, next steps.

