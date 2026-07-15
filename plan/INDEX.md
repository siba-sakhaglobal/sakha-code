# Sakha Coding Agent Planning Workspace

This folder is the development plan for a clean-room, provider-neutral coding agent named **Sakha Coding Agent**.

The plan is intentionally split by module and task so future agents can work independently without loading one huge file.

## Read Order

1. `00-executive-summary.md` — product goal, non-goals, core principles.
2. `01-product-scope-feature-map.md` — all required user-facing capabilities.
3. `02-clean-room-and-migration-strategy.md` — legal/safety boundary for rewrite.
4. `03-tech-stack-decision.md` — recommended language/runtime/UI stack.
5. `04-core-domain-model.md` — core structs, traits, services, and data ownership.
6. `05-system-architecture.md` — process layout and module dependency graph.
7. `crates/crate-work-breakdown.md` — crate-by-crate file, type, and test breakdown (implementation source of truth).
8. `references.md` — external references used for Headroom, loop engineering, MCP, Rust, Tokio, Tauri, and long-running agents.

## Module Plans

| Module | File | Purpose |
|---|---|---|
| Core runtime | `modules/01-core-runtime.md` | Process lifecycle, workspace session, event bus, cancellation, budgets |
| LLM gateway | `modules/02-llm-provider-gateway.md` | OpenAI-compatible providers, Gemini, Grok/xAI, OpenAI, local models |
| Agent loop | `modules/03-agent-loop-engine.md` | ReAct/tool loop, planning loop, sub-agent loop, termination |
| Loop engineering | `modules/04-loop-engineering.md` | Scheduled, event-driven, verification, feedback, and research loops |
| Headroom compression | `modules/05-context-compression-headroom.md` | Built-in Headroom integration and Sakha compression abstraction |
| Context and memory | `modules/06-context-memory-state.md` | SQLite memory, summaries, vector/index state, durable handoffs |
| Tools | `modules/07-tool-system.md` | Tool registry, schemas, execution, audit, idempotency |
| Files/git/worktrees | `modules/08-files-git-worktrees.md` | Safe edit engine, diff engine, branches, isolated worktrees |
| Terminal/PTY | `modules/09-terminal-process-automation.md` | Shell, PTY, process supervision, log capture |
| MCP/plugins/connectors | `modules/10-mcp-plugin-connector-system.md` | MCP client/server, plugins, connector isolation |
| Web search/research | `modules/11-web-search-research.md` | Research loop, source ranking, browsing memory, citation policy |
| Verification/evals | `modules/12-verification-evals.md` | Tests, static checks, rubrics, regression and agent evals |
| UI surfaces | `modules/13-ui-cli-tui-web-desktop.md` | CLI, TUI, web, desktop, notifications |
| Security/sandbox | `modules/14-security-sandbox-permissions.md` | Permissions, sandbox profiles, secrets, command policy |
| Observability | `modules/15-observability-audit.md` | Logs, traces, metrics, cost, token accounting |
| Collaboration | `modules/16-subagents-teams.md` | Sub-agent orchestration, work assignment, code review |
| Config/secrets | `modules/17-config-secrets-policy.md` | Settings, env aliases, secret storage, policy layering |
| Packaging/deployment | `modules/18-packaging-deployment.md` | CLI binaries, Tauri app, Docker, server mode, update flow |
| API/contracts | `modules/19-api-contracts.md` | Internal schemas, JSON-RPC, local daemon API, event protocol |
| Extensibility | `modules/20-extensibility-marketplace.md` | Skills, prompt packs, tool packs, plugin trust model |
| Prompt system | `modules/21-prompt-system.md` | System-prompt assembly, templates, skill packs, cache-friendly layout |
| Errors/retry | `modules/22-error-taxonomy-retry.md` | Cross-cutting error taxonomy, retry/backoff, rate-limit policy |

## Task Plans

| Task File | Purpose |
|---|---|
| `tasks/roadmap.md` | Phase-by-phase roadmap |
| `tasks/phase-00-research-and-spec.md` | Clean-room research, spec, and acceptance criteria |
| `tasks/phase-01-rust-workspace-skeleton.md` | Initial Rust workspace and crate layout |
| `tasks/phase-02-provider-gateway.md` | Provider abstraction and streaming |
| `tasks/phase-03-tool-runtime.md` | Tool registry and safe executor |
| `tasks/phase-04-agent-loop.md` | Main agent loop and termination guards |
| `tasks/phase-05-headroom-integration.md` | Headroom integration and compression policy |
| `tasks/phase-06-loop-engineering.md` | Long-running loops, scheduling, web research |
| `tasks/phase-07-ui-and-daemon.md` | CLI/TUI/web/Tauri/local daemon |
| `tasks/phase-08-security-and-sandbox.md` | Permissions, sandboxing, secrets |
| `tasks/phase-09-evals-and-release.md` | Eval harness, packaging, release gates |
| `tasks/backlog.md` | Feature backlog and suggested improvements |
| `tasks/acceptance-checklists.md` | Definition of done by module |

## Implementation

The executable Rust workspace lives outside this spec folder at `e:/Project Git/GitHub/sakha/` (this folder stays spec-only per the working rules). Crate layout follows `crates/crate-work-breakdown.md`.

## Agent Working Rules

- Do not port source code line-by-line from the existing TypeScript project.
- Treat this folder as a product and architecture spec, not executable implementation.
- Every implementation task must reference one module plan and one task plan.
- Every module must define interfaces, data structures, failure modes, and test requirements.
- Preserve provider neutrality: no hard dependency on one LLM vendor.
- Build for long-running work: durable state, explicit handoff files, loop budgets, and recovery checkpoints.

