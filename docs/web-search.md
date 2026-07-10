# Web Search & Fetch

Sakha's agent loop can search the web and pull individual pages via two
tools: `web.search` and `web.fetch`. Both are backed by a prioritized pool of
pluggable providers with automatic quota/auth failover, so a single expired
or rate-limited key never stops research — the pool just moves on to the
next configured backend.

## The economical search-then-fetch flow

Full-page fetches cost far more tokens than a snippet. The tools are
designed so the model always searches first, then chooses:

1. **`web.search`** — cheap. Returns only `title` / `url` / `snippet` for up
   to 10 hits. Use this to discover candidate pages.
2. **`web.fetch`** — expensive. Returns the full page as clean, size-capped
   markdown. Only call this on the specific URL(s) chosen from `web.search`
   results — never fetch every hit.

Both tools' descriptions reinforce this to the model directly (`web.search`:
"Cheap — returns snippets only. Call web.fetch on the most relevant URL(s)
afterwards."; `web.fetch`: "Costs more tokens — only fetch URLs chosen from
web.search results.").

## How the search pool works

`SearchPool` (in `sakha-research`) holds backends in **priority order**.
Priority is manual — set by config (`[search].backends`) or the built-in
default order below — but **failover within that order is automatic**:

- The pool tries the first configured backend in the list.
- If a backend call fails with a quota/auth-shaped error (HTTP
  401/402/403/429) or a transport error, the pool records a **10-minute
  in-process cooldown** for that backend and immediately tries the next one.
- The first success wins. `web.search`'s output includes which `backend`
  actually served the request, so this is visible, not silent.
- `sakha search-backends` shows live status: which backends are configured,
  which are currently cooling down, and their last error kind.

This is "switch key automatically based on quota" — you still choose which
backends to enable and in what order; the pool just stops retrying a backend
that just told it to back off.

### Default backend priority

```
firecrawl -> brave -> tavily -> serper -> serpapi -> exa -> searxng
```

A backend is **configured** if its required environment variable is set.
Firecrawl is the only exception: it works keyless (rate-limited free tier),
so it's always considered configured and sits first in the default order.

## How fetch works

`web.fetch` / `FetchPool` tries progressively more capable backends for a
single URL, in order:

1. **Firecrawl scrape** (`POST /v2/scrape`, keyless-capable) — best-quality
   markdown extraction.
2. **Jina Reader** (`GET https://r.jina.ai/{url}`, keyless-capable,
   rate-limited) — solid fallback markdown extraction.
3. **Plain fetch** — a direct `reqwest` GET through Sakha's existing
   `html2text`-based extractor. Always available, no third-party dependency,
   no key required. This is the guaranteed-to-work floor.

The result is stripped of null bytes and truncated to `max_chars`
(`[truncated]` appended when truncation happens) so a single fetch can never
blow the context budget.

## Backends: environment variables and key portals

| Backend | Env var | Keyless tier? | Get a key |
|---|---|---|---|
| Firecrawl | `FIRECRAWL_API_KEY` | Yes (rate-limited) | https://www.firecrawl.dev/app/api-keys |
| Brave Search | `BRAVE_API_KEY` | No | https://api-dashboard.search.brave.com |
| Tavily | `TAVILY_API_KEY` | No | https://app.tavily.com |
| Serper | `SERPER_API_KEY` | No | https://serper.dev/api-key |
| SerpApi | `SERPAPI_API_KEY` | No | https://serpapi.com/manage-api-key |
| Exa | `EXA_API_KEY` | No | https://dashboard.exa.ai/api-keys |
| SearXNG | `SEARXNG_BASE_URL` | Self-hosted, no key | Point at your own instance's base URL |
| Jina Reader (fetch only) | `JINA_API_KEY` | Yes (rate-limited) | https://jina.ai/api-dashboard |

Set as many or as few as you like. With nothing configured, `web.search`
still works via Firecrawl's keyless tier; `web.fetch` still works via its
plain-fetch fallback. If every configured backend is unavailable (all
cooling down, or none configured and even keyless Firecrawl fails), the
tools return a clear `invalid_input` error explaining that no search backend
is available — never a panic, never a silent empty result.

Keys can live in real environment variables, or in `./.env` /
`~/.sakha/.env` — both are auto-loaded by the CLI before any command runs
(see [Configuration Reference](./configuration.md) ".env Autoload"). API
keys are never logged and never appear in error messages.

## CLI usage

```bash
# Search (cheap — snippets only)
sakha search "rust tokio tutorial" --limit 5
sakha search "rust tokio tutorial" --backend brave   # bypass priority/failover, target one backend
sakha search "rust tokio tutorial" --output json

# Fetch one URL as clean markdown
sakha fetch https://tokio.rs/tokio/tutorial --max-chars 8000

# Inspect backend status: configured?, cooling down?, key portal
sakha search-backends
```

## Configuration

Optional `[search]` section in `~/.sakha/config.toml`:

```toml
[search]
backends = ["brave", "firecrawl", "tavily"]  # priority override; omit for the default order
max_results = 5
fetch_max_chars = 12000
```

All three fields are optional — an absent `[search]` section behaves
identically to the defaults shown above. See
[Configuration Reference](./configuration.md#search--web-search-tool-configuration)
for the full field reference.

## Agent tool reference

### `web.search`

- **Input**: `{ "query": string (required), "limit": integer 1-10, default 5 }`
- **Output**: `{ "backend": string, "hits": [{ "title", "url", "snippet" }] }`

### `web.fetch`

- **Input**: `{ "url": string (required), "max_chars": integer, default 12000 }`
- **Output**: `{ "url": string, "backend": string, "content_markdown": string }`

Both tools are registered by default in every `ToolRegistry` Sakha builds
(`sakha-cli::runtime::build_tool_registry`), so they're available to `sakha
run`/`sakha chat` out of the box.
