# Product Scope and Feature Map

## A. Conversation and Coding

### Features

- Interactive chat with streaming.
- Non-interactive prompt execution.
- Context-aware codebase Q&A.
- File editing with preview diff.
- Multi-file patch application.
- Test/lint/build invocation.
- Error diagnosis and fix loops.
- Code review mode.
- Commit message and PR summary generation.
- Refactor planning and staged implementation.

### Required Modules

- `agent-loop-engine`
- `llm-provider-gateway`
- `tool-system`
- `files-git-worktrees`
- `verification-evals`
- `context-memory-state`

## B. Automation and Long-Running Work

### Features

- Goal mode: run until objective complete or blocked.
- Scheduled loops: cron-style or interval-based tasks.
- Event loops: GitHub issue, CI failure, webhook, file change, queue item.
- Worktree isolation per task/sub-agent.
- Durable handoff notes after every session.
- Automatic resume from checkpoint.
- Loop watchdogs for infinite loop prevention.
- Budget enforcement for tokens, cost, tools, wall-clock, and retries.

### Required Modules

- `loop-engineering`
- `core-runtime`
- `subagents-teams`
- `context-memory-state`
- `observability-audit`
- `security-sandbox-permissions`

## C. Provider Support

### Features

- OpenAI-compatible chat completions.
- OpenAI Responses API adapter as optional future provider.
- Gemini OpenAI-compatible endpoint.
- xAI/Grok OpenAI-compatible endpoint.
- Local model gateways: Ollama, vLLM, LM Studio, LiteLLM, OpenRouter.
- Provider capability registry.
- Streaming normalization.
- Tool call normalization.
- Structured output normalization.
- Reasoning controls where supported.
- Per-model context/token/cost metadata.

### Required Modules

- `llm-provider-gateway`
- `api-contracts`
- `observability-audit`
- `config-secrets-policy`

## D. Context Compression

### Features

- Headroom library/proxy/MCP integration options.
- Content routing: code, JSON, logs, shell output, file tree, prose, conversation.
- Reversible compression with retrieval markers.
- Compression metrics per context item.
- Auto-retrieve if model needs omitted detail.
- Compression policy by trust level and tool type.
- Cache-aligned prompt prefix design.
- Context budget planner.

### Required Modules

- `context-compression-headroom`
- `context-memory-state`
- `tool-system`
- `llm-provider-gateway`

## E. Web Search and Research

### Features

- Search query planning.
- Multiple search providers.
- Page fetch, extraction, and citation tracking.
- Recency-aware retrieval.
- Source quality scoring.
- Research memory and deduplication.
- Search loop with stop conditions.
- Evidence pack generation for the agent.
- Citation-aware final answers.

### Required Modules

- `web-search-research`
- `loop-engineering`
- `context-compression-headroom`
- `observability-audit`

## F. Tools and Connectors

### Features

- Local tools: file, shell, git, search, patch, test, browser, HTTP.
- MCP client and MCP server.
- Plugin packs.
- Skills/prompt packs.
- External connectors: GitHub, Slack, Linear/Jira, docs, cloud logs.
- Tool allow/deny policy.
- Tool result compression.
- Tool idempotency keys.
- Tool audit log.

### Required Modules

- `tool-system`
- `mcp-plugin-connector-system`
- `extensibility-marketplace`
- `security-sandbox-permissions`

## G. UI Surfaces

### Features

- CLI commands.
- TUI interactive chat.
- Web session viewer.
- Desktop app with Tauri.
- Session timeline.
- Diff viewer.
- Tool approval prompts.
- Loop dashboard.
- Cost/token dashboard.
- Agent/team dashboard.

### Required Modules

- `ui-cli-tui-web-desktop`
- `observability-audit`
- `api-contracts`

## H. Enterprise and Safety

### Features

- Policy files.
- Secrets storage.
- Sandboxed execution profiles.
- Redaction.
- Audit export.
- Telemetry opt-in.
- Multi-workspace settings.
- Role-based controls for server mode.
- Offline mode.

### Required Modules

- `security-sandbox-permissions`
- `config-secrets-policy`
- `packaging-deployment`
- `observability-audit`

## Suggested Missing Enhancements

- Deterministic replay engine for repetitive tasks.
- Loop skill recorder that turns successful agent trajectories into reusable workflows.
- Runtime loop static analyzer to detect unbounded agent/tool cycles.
- Artifact memory graph linking files, decisions, tests, and sources.
- Shadow-mode automation: agent proposes actions but does not execute until trusted.
- Compression A/B evaluation harness to catch over-compression.
- Provider failover and model quorum mode.
- Local vector index over codebase plus symbol graph.
- LSP integration for semantic edits.
- Browser automation with DOM snapshots and permission-scoped sessions.

