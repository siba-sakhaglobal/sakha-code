# Development Handoff — Session Context

Continuation context for AI-assisted development on a new machine. Read `CLAUDE.md` first, then this. Last updated: 2026-07-15.

## Where the project stands

The workspace was built in one multi-agent pass from the `plan/` spec (~610 tests at first green), then hardened through real end-to-end usage against Google Gemini. All work is committed on `master`/`main`; every commit message carries the rationale.

### Commit history (oldest → newest)

| Commit | What |
|---|---|
| `ebe909b` | Initial 15-crate workspace scaffold + sakha-core |
| `42395c0` | Integration pass — workspace build + tests green |
| `fc084f9`…`275a52a` | Review-findings fixes (102 critical/major findings from per-crate spec review) |
| `767e713` | Docs (README, getting-started, configuration, architecture) + final verification |
| `ecb1c10` | `sakha run` session persistence (M1 gap) + db_path TOML placement doc fix |
| `42ed551` | Provider catalog (19 presets), `sakha login` (OpenRouter OAuth PKCE + keyring paste flow), `.env` autoload |
| `6ff6850` | Gemini-compat tool-calling fixes (tool_calls echo, thought signatures, indexless parallel calls) + `SAKHA_DEBUG_BODY` |
| `50c8fe7` | Multi-backend web search/fetch pool (7 backends, quota failover) + `web.search`/`web.fetch` agent tools |
| `5dfd2ac` | Skills system: SKILL.md discovery (.sakha/.agents/.claude/.gemini tiers), `skill.activate` tool, `--skill` flag |

### Verified end-to-end (against live Gemini)

- M1: `sakha run` streams, persists session, records usage. Provider health check.
- Multi-round tool calling: sakha scaffolded, then iteratively extended `examples/react-login-demo` (React+Vite+Express+SQLite full-stack app) purely via its own tool calls.
- Web research loop: `web.search` (snippets) → selective `web.fetch` → file edits, quota failover pool.
- Skills: `design-md` skill (installed in the demo's `.agents/skills/` and `~/.sakha/skills/`) drives DESIGN.md-token-faithful UI work.
- Self-repair: sakha fixed its own bugs when fed evidence (dep pin for Node 22, SQLite migration duplicates, JSX quote corruption).

## Environment setup on a new machine

1. Rust 1.89+, Node 20/22 (for the demo), git.
2. Provider: `sakha login gemini --api-key <key>` (keyring) or put `GEMINI_API_KEY=...` in `./.env` or `~/.sakha/.env` (auto-loaded, never committed).
3. `~/.sakha/config.toml` — note `db_path` must be top-level (above `[provider]`):
   ```toml
   db_path = "<home>/.sakha/sessions.sqlite3"

   [provider]
   selection = "open_ai_compatible"
   base_url = "https://generativelanguage.googleapis.com/v1beta/openai/"
   model = "gemini-3-flash-preview"
   api_key_env = "GEMINI_API_KEY"
   preset = "gemini"
   ```
4. Optional search keys (any subset; pool fails over automatically): `FIRECRAWL_API_KEY`, `BRAVE_API_KEY`, `TAVILY_API_KEY`, `SERPER_API_KEY`, `SERPAPI_API_KEY`, `EXA_API_KEY`, `SEARXNG_BASE_URL`. Firecrawl works keyless (rate-limited). API keys used during original development are NOT in this repo — create fresh ones (portals listed in `docs/web-search.md`).

## Known limitations / deliberate stubs

- Sandbox is `NoopSandbox` — OS adapters (Linux namespaces, Windows job objects, macOS sandbox) are future work (`plan/modules/14`).
- Compression defaults to the local fallback compressor; Headroom HTTP adapter exists but needs a real endpoint (`plan/modules/05`).
- Sub-agents work inside `sakha-agent` (budget-sliced nested agents) but have no CLI surface yet (`plan/modules/16`).
- `sakha eval` command exists as a skeleton; the regression eval suite (phase 09) is not populated.
- ~30 clippy warnings (dead-code/derivable-impls style only) and 17 minor review findings intentionally deferred.

## Agreed next steps (in rough priority order)

1. **agentmemory integration** (user-requested, design agreed): Layer 1 = config-only MCP bridge via existing `sakha-mcp` (`npx @agentmemory/mcp`); Layer 2 = `ExternalMemory` trait in `sakha-context` (recall/observe/session hooks) + REST adapter to `http://localhost:3111/agentmemory/*`, no-op fallback, config-gated. Purpose: persistent cross-LLM project memory.
2. **Release pipeline**: GitHub Actions building win/mac/linux binaries on tag; later winget/scoop/homebrew.
3. **Eval harness**: populate `sakha eval` with scaffolding tasks (the react-login-demo flow is the template) to score providers/models objectively.
4. **Live pricing fetch**: demo currently seeds pricing researched via snippets; a `web.fetch` pass against official pricing pages would make it authoritative.
5. **Sub-agent CLI surface** (`sakha agents ...`) driven by `sakha-loop`.

## Working style that worked

- Sakha fixes its own bugs: feed it the exact error/evidence as a `sakha run` prompt rather than hand-editing.
- Split big tasks into passes (backend → frontend → QA gaps) for lite models; strong models (`gemini-3-flash-preview`+) take full specs one-shot.
- For UI work: author/require a DESIGN.md via the `design-md` skill first, then implement from tokens.
- Prompt quality is a real dial: a maximal, numbered, constraint-explicit spec extracted ~2× the output from the same model.
