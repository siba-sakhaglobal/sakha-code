# Module 09: Terminal and Process Automation

## Responsibility

Run commands, manage PTYs, stream output, capture logs, and supervise long-lived processes safely.

## Rust Crate

`crates/sakha-tools` and `crates/sakha-terminal`

## Main Structs

- `CommandSpec`
- `CommandPolicy`
- `ProcessSupervisor`
- `ProcessHandle`
- `PtySession`
- `OutputCapture`
- `ExitStatus`
- `CommandRisk`
- `LogCompressor`

## Features

- Non-interactive command execution.
- Interactive PTY sessions.
- Command timeout.
- Output streaming.
- stdout/stderr capture.
- Environment variable redaction.
- Shell selection.
- Working directory policy.
- Background process tracking.
- Kill tree.
- Log compression via Headroom.

## Command Risk Model

- Read-only command.
- Build/test command.
- Package install.
- Network command.
- Write workspace.
- Delete/move.
- Privileged command.
- Unknown command.

## Safety Rules

- Never run destructive commands without permission.
- Never pass secret values to model-visible output.
- Output over threshold must be summarized/compressed.
- Background processes must be registered.
- Commands inherit sanitized environment by default.

## Implementation Tasks

1. Implement command parser/risk classifier.
2. Implement process runner.
3. Implement PTY runner.
4. Implement stream capture.
5. Implement timeout and kill tree.
6. Implement env sanitizer.
7. Implement log artifact writer.
8. Implement compression hook.
9. Implement background process registry.
10. Implement UI process stream events.

## Tests

- Simple command succeeds.
- Timeout kills process tree.
- Large output compressed.
- Secret redaction works.
- PTY session streams output.
- Risk classifier requires permission for destructive command.

