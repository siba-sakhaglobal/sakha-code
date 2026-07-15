# Module 07: Tool System

## Responsibility

Provide safe, schema-driven tools for agent actions.

## Rust Crate

`crates/sakha-tools`

## Main Structs

- `ToolRegistry`
- `ToolSpec`
- `ToolName`
- `ToolInputSchema`
- `ToolOutputSchema`
- `ToolCall`
- `ToolResult`
- `ToolContext`
- `ToolExecutor`
- `ToolPermissionSpec`
- `ToolAuditRecord`
- `IdempotencyKey`

## Tool Trait

```text
Tool {
  spec() -> ToolSpec
  validate(input) -> ValidatedInput
  plan(input, ctx) -> ToolPlan
  execute(input, ctx) -> ToolResult
  summarize(result) -> ToolSummary
}
```

## Built-In Tools

- `file.read`
- `file.write`
- `file.patch`
- `file.delete`
- `file.move`
- `file.list`
- `search.ripgrep`
- `git.status`
- `git.diff`
- `git.apply_patch`
- `git.branch`
- `git.worktree`
- `git.commit`
- `shell.run`
- `test.run`
- `package.install`
- `web.search`
- `web.fetch`
- `browser.open`
- `mcp.call_tool`
- `memory.write`
- `memory.search`
- `compression.retrieve`
- `loop.create`
- `loop.status`
- `subagent.spawn`

## Tool Execution Pipeline

1. Parse model tool request.
2. Validate against schema.
3. Compute risk.
4. Request permission if needed.
5. Compute idempotency key.
6. Execute in sandbox.
7. Capture stdout/stderr/artifacts.
8. Compress result.
9. Store audit record.
10. Return normalized result to agent.

## Permission Categories

- Read-only.
- Write workspace.
- Run safe command.
- Run arbitrary command.
- Network access.
- Secret access.
- External side effect.
- Destructive filesystem.
- Git remote write.
- Browser automation.

## Implementation Tasks

1. Define tool schemas.
2. Implement registry.
3. Implement executor.
4. Implement permission bridge.
5. Implement built-in read/search/git status tools.
6. Implement patch engine.
7. Implement shell runner.
8. Implement tool result compression.
9. Implement audit log.
10. Add tool tests and golden outputs.

## Tests

- Schema validation rejects bad input.
- Permission denied prevents execution.
- Idempotent duplicate is detected.
- Tool output is compressed and raw artifact stored.
- Destructive tool requires explicit approval.
- Tool timeout kills child process.

