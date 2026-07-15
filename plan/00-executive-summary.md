# Executive Summary

## Product Goal

Sakha Coding Agent is a clean-room coding assistant for local and remote software engineering automation.

It should support:

- Interactive coding assistance.
- Long-running autonomous engineering loops.
- Multi-agent development and verification.
- Provider-neutral LLM routing.
- Built-in context compression using Headroom-compatible compression.
- Web research and source-grounded planning.
- Safe file, shell, git, browser, MCP, and CI automation.
- CLI, TUI, web, and desktop surfaces.

## Core Principles

1. **Provider-neutral by default**: any OpenAI-compatible endpoint should work with URL, key, model, and capability config.
2. **Clean-room implementation**: rebuild behavior from specs and tests, not copied source.
3. **Long-running by design**: every task has state, checkpoints, budgets, and resumable handoffs.
4. **Compression-aware context**: all context enters through a routing/compression layer.
5. **Tool safety first**: every side effect is permissioned, audited, and replayable where possible.
6. **Loop engineering over prompting**: humans define loops, policies, rubrics, and goals; agents execute.
7. **Local-first trust model**: local workspace and local state remain the source of truth.
8. **Transparent automation**: every agent decision, tool result, compression event, and verification result is inspectable.

## Non-Goals

- Do not clone branding, legal notices, or private provider-specific behavior from any existing product.
- Do not depend on a single LLM provider.
- Do not make shell automation invisible.
- Do not let unbounded loops call tools or models indefinitely.
- Do not rely on context window compaction alone for memory.

## Target Users

- Individual developers doing local coding.
- Teams running background maintenance agents.
- Platform teams building internal coding-agent automation.
- Researchers/evaluators testing agent loops, compression, and tool reliability.

## First Production Slice

The first shippable product should include:

- Rust CLI with interactive chat and non-interactive `run` mode.
- OpenAI-compatible provider gateway.
- File read/write, shell, git diff, ripgrep, and test-run tools.
- SQLite session store and handoff state.
- Headroom compression adapter for tool outputs, logs, files, and conversation history.
- Loop engine with max-iteration, max-cost, max-time, and verification gates.
- Web search/research module with source tracking.
- Basic TUI or web UI for session inspection.

