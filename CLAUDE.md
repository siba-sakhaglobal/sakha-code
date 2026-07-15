# Sakha Coding Agent — Project Instructions

Sakha is a clean-room, provider-neutral coding agent written in Rust (15-crate cargo workspace). It was built from the spec in `plan/` (read `plan/INDEX.md` first) and is developed with AI assistance — this file plus `docs/handoff.md` carry the context a fresh session needs.

## Build & Test

- Rust 1.89+ required. On the original dev machine cargo lives at `~/.cargo/bin` (add to PATH in shell tools).
- `cargo build --workspace` / `cargo test --workspace` must stay green — every feature lands with tests.
- Run the CLI: `cargo run -q -p sakha-cli -- <command>` (binary name: `sakha`).

## Architecture Ground Rules (from plan/, enforced so far)

- Dependency direction: `ui -> daemon -> agent -> loop -> tools/provider/context/security -> core`. Never invert.
- Provider-neutral: any OpenAI-compatible endpoint via `ProviderProfile`; no hard vendor dependency. Provider-opaque payloads (e.g. Gemini thought signatures in `extra_content`) are passed through verbatim, never interpreted.
- Secrets: env vars or OS keyring only — never in config files, never logged. Redaction lives in `sakha-security`/`sakha-observability`.
- Every side effect goes through the tool registry with schema validation.
- Clean-room rule: do not port code from other agent products; implement from `plan/` specs.

## Key Facts Discovered During Development

- **Gemini OpenAI-compat quirks** (all handled in `sakha-provider`, keep regression tests green):
  1. assistant `tool_calls` must be echoed back in history;
  2. `extra_content.google.thought_signature` on tool calls is mandatory to echo;
  3. tool-call deltas carry NO per-call `index` — assembler starts a new call when a new `id` hits an occupied index.
- **TOML config trap**: `db_path` is top-level; placed after `[provider]` it silently becomes `provider.db_path`. Docs warn about this — keep the warning.
- **Model tiers matter**: `gemini-3.1-flash-lite` degrades UI rewrites and corrupts files on small edits (escaped quotes); `gemini-3-flash-preview` executes large specs in one pass. Use lite tiers only for read/search/small-data tasks.
- `SAKHA_DEBUG_BODY=1` dumps outgoing provider request bodies + tool failure detail (no secrets in bodies).

## Layout

- `crates/` — the 15 workspace crates (see `plan/crates/crate-work-breakdown.md` for the file/type/test map).
- `plan/` — full product/architecture spec (22 module docs, 10 phase tasks, roadmap). Treat as spec, not code.
- `examples/react-login-demo/` — the E2E demo app sakha itself built (Express+SQLite backend, React dashboard); used as the live capability testbed. `npm install` then `npm run server` + `npm run dev`.
- `docs/` — user docs (getting-started, configuration, providers, web-search, skills, architecture) + `docs/handoff.md` (session continuation context).

## Conventions

- Conventional commits; commit messages explain the *why*; every fix cites its root cause.
- Doc comments on public items cite the spec module they implement (e.g. "spec `modules/07-tool-system.md`").
- New workspace deps go in `[workspace.dependencies]` (pinned); crates reference with `workspace = true`.
