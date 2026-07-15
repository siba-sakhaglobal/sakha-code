# Module 13: UI, CLI, TUI, Web, and Desktop

## Responsibility

Provide usable interfaces for humans to supervise, approve, inspect, and run agents.

## Crates and Packages

- `crates/sakha-cli`
- `crates/sakha-tui`
- `crates/sakha-daemon`
- `apps/sakha-web`
- `apps/sakha-desktop`

## CLI Commands

- `sakha chat`
- `sakha run <prompt>`
- `sakha goal create`
- `sakha goal status`
- `sakha loop create`
- `sakha loop list`
- `sakha loop pause`
- `sakha loop resume`
- `sakha providers list`
- `sakha tools list`
- `sakha memory search`
- `sakha mcp add`
- `sakha config get/set`
- `sakha eval run`
- `sakha doctor`

## TUI Screens

- Chat.
- Tool approval.
- Diff viewer.
- Session timeline.
- Loop dashboard.
- Provider/cost dashboard.
- Memory browser.
- Research evidence viewer.
- Compression stats viewer.

## Web/Desktop Screens

- Workspace overview.
- Active sessions.
- Goal board.
- Loop scheduler.
- Agent team board.
- Tool audit log.
- File diff viewer.
- Terminal stream.
- Research notebook.
- Settings/secrets.
- Evals dashboard.

## UI Event Protocol

Events:

- `session.started`
- `model.delta`
- `tool.requested`
- `permission.requested`
- `tool.output`
- `diff.ready`
- `verification.result`
- `loop.tick`
- `compression.stats`
- `budget.update`
- `session.completed`

## Implementation Tasks

1. Define UI protocol schema.
2. Implement CLI with `clap`.
3. Implement local daemon API.
4. Implement TUI chat.
5. Implement permission prompt.
6. Implement diff viewer.
7. Implement web session timeline.
8. Implement loop dashboard.
9. Implement Tauri shell.
10. Implement notification system.

## Tests

- CLI command parses.
- TUI renders model/tool events.
- Permission approval reaches tool runtime.
- Web socket reconnect works.
- Desktop app connects to local daemon.
- Diff viewer handles large diffs.

