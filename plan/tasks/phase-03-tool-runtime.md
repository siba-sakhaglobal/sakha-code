# Phase 03: Tool Runtime

## Goal

Implement safe tool execution and audit.

## Tasks

### 03.1 Tool Registry

- Define `Tool` trait.
- Define `ToolSpec`.
- Define schema validation.
- Implement registry lookup.

### 03.2 File Tools

- `file.read`
- `file.write`
- `file.patch`
- `file.list`
- snapshot before write
- path policy

### 03.3 Search and Git Tools

- `search.ripgrep`
- `git.status`
- `git.diff`
- `git.worktree`
- `git.apply_patch`

### 03.4 Shell Tools

- `shell.run`
- command risk classifier
- timeout
- output capture
- env redaction

### 03.5 Audit and Compression Hooks

- Audit before tool execution.
- Store raw output.
- Compress large output.
- Return summary to agent.

## Definition of Done

- Agent can read/search files.
- Agent can propose and apply patch with permission.
- Shell output is captured and compressed.
- Audit log links every tool call to session/turn.

