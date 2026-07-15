# Roadmap

## Phase Sequence

| Phase | Name | Outcome |
|---|---|---|
| 00 | Research and spec | Clean-room product spec and acceptance matrix |
| 01 | Rust workspace skeleton | Compile-ready workspace, core crates, schema baseline |
| 02 | Provider gateway | OpenAI-compatible streaming and tool-call normalization |
| 03 | Tool runtime | File, shell, git, search, audit, permission previews |
| 04 | Agent loop | Interactive agent loop with tools and termination guards |
| 05 | Headroom integration | Compression/retrieval path for tool output and context |
| 06 | Loop engineering | Long-running goals, scheduler, research loop, watchdog |
| 07 | UI and daemon | CLI, daemon API, TUI/web dashboard, Tauri shell |
| 08 | Security and sandbox | Policy, sandbox, secrets, audit hardening |
| 09 | Evals and release | Regression evals, packaging, docs, release gate |

## Milestones

### M0: Spec Complete

- All module docs reviewed.
- Clean-room boundaries accepted.
- Feature disposition table complete.
- Eval suite skeleton defined.

### M1: First Model Call

- `sakha run "hello"` streams through OpenAI-compatible gateway.
- Usage recorded.
- Session persisted.

### M2: First Safe Edit

- Agent reads files, proposes patch, receives permission, applies patch.
- Diff recorded.
- Handoff written.

### M3: Compression Active

- Tool output and file context pass through Headroom adapter.
- Raw artifacts retrievable.
- Compression stats visible.

### M4: First Long-Running Loop

- Goal loop runs across two sessions.
- Writes handoff.
- Resumes from checkpoint.
- Stops on budget or completion.

### M5: Research Loop

- Agent searches web, fetches sources, compresses pages, writes evidence pack.
- Final output cites sources.

### M6: Multi-Agent Worktree

- Coordinator spawns implementer and reviewer.
- Worktree isolation works.
- Reviewer gates merge.

### M7: Desktop/Web Supervision

- UI shows session timeline, tool calls, loops, costs, and compression stats.

### M8: Secure Beta

- Sandbox enabled.
- Secrets redacted.
- Audit export works.
- Eval pass threshold met.

## Critical Path

1. Core runtime.
2. Provider gateway.
3. Tool runtime.
4. Agent loop.
5. Compression layer.
6. Loop engine.
7. Verification/evals.
8. UI supervision.
9. Sandbox hardening.

## Parallelizable Work

- UI can start after API contracts.
- MCP/plugin system can start after tool registry.
- Web research can start after tool runtime.
- Evals can start after provider gateway.
- Packaging can start after CLI skeleton.

