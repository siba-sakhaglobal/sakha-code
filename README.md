# Sakha Coding Agent

A clean-room coding assistant for local and remote software engineering automation, built in Rust with provider-neutral LLM routing, compression-aware context, and safe tool automation.

## What Is Sakha?

Sakha is a production-grade coding agent platform designed for:

- **Interactive coding assistance** — real-time chat with context-aware tool use
- **Long-running autonomous loops** — multi-iteration engineering workflows with budgets and checkpoints
- **Multi-agent collaboration** — spawnable subagents with isolated state
- **Provider-neutral routing** — any OpenAI-compatible endpoint (Ollama, vLLM, cloud providers, etc.)
- **Context compression** — Headroom-compatible compression for tools, logs, files, and history
- **Web research** — source-grounded planning with result caching and verification
- **Safe automation** — permissioned file, shell, git, browser, MCP, and CI tools with audit trails

## Features

- **CLI + TUI + Web surfaces** for session management, memory inspection, and agent observation
- **SQLite-backed session store** with durable conversation history and handoff state
- **Tool safety framework** — every shell command, file write, and API call is logged and replayable
- **Loop engine with budgets** — max iterations, cost, time, and verification gates
- **Provider gateway** — OpenAI-compatible abstraction with streaming and tool-use support
- **MCP (Model Context Protocol) integration** — extend capabilities with external tools
- **Local-first trust model** — workspace state remains source of truth; all logs inspectable

## Workspace Structure

| Crate | Purpose |
|-------|---------|
| **sakha-cli** | Command-line entry point; `sakha run`, `sakha chat`, `sakha session`, `sakha config`, etc. |
| **sakha-core** | Domain types: `SessionId`, `Turn`, `ToolCall`, etc.; foundation for all other crates |
| **sakha-provider** | OpenAI-compatible provider client; streaming, tool use, model capability negotiation |
| **sakha-context** | Context planning, budgeting, attribution, and prompt assembly (`ContextPlanner`, `PromptAssembler`) |
| **sakha-compression** | Headroom-compatible context compression for tool outputs, logs, and conversation history |
| **sakha-memory** | SQLite session store; turn records, goal history, memory search, and handoff state |
| **sakha-tools** | Tool registry and implementations (file I/O, shell, git, ripgrep, test runners) |
| **sakha-security** | Permissioning, audit trails, safe shell/file operations, sandboxing policy enforcement |
| **sakha-research** | Web search, source tracking, caching, and verification for grounded planning |
| **sakha-agent** | Core agent loop; receives requests, calls provider, processes tool results, manages state |
| **sakha-loop** | Long-running loop engine with iteration budgets, max costs, max time, and verification gates |
| **sakha-mcp** | Model Context Protocol server/client integration for extensible tool use |
| **sakha-observability** | Structured logging, tracing, and JSON telemetry for agent decisions and tool results |
| **sakha-daemon** | Background HTTP server; scheduled agents, session management, WebSocket streaming |
| **sakha-tui** | Terminal UI for session inspection, memory search, and agent observation |

## Build Instructions

**Requirements:** Rust 1.89+ (stable channel)

```bash
# Clone the repository
git clone https://github.com/sakha-agent/sakha.git
cd sakha

# Build all crates
cargo build --workspace --release

# Run tests
cargo test --workspace

# Build the CLI binary only
cargo build -p sakha-cli --release
# Binary at: target/release/sakha (Windows: sakha.exe)
```

The workspace uses Cargo's default resolver v2. All internal crates are managed via workspace dependencies in the root `Cargo.toml`.

## Quickstart

### 1. Install the CLI

```bash
# After building:
cargo install --path crates/sakha-cli

# Or copy the binary:
# Windows: target/release/sakha.exe
# Linux/macOS: target/release/sakha
```

Verify installation:
```bash
sakha --version
sakha --help
```

### 2. Configure a Provider

The fastest way to get connected is `sakha login`, which authenticates
against a [built-in provider preset](./docs/providers.md) and stores the key
in your OS credential manager (never in the config file):

```bash
# OpenRouter: opens your browser for an OAuth PKCE login
sakha login openrouter

# Any other provider: opens the provider's key page, then prompts you to paste the key
sakha login gemini

# Fully non-interactive (e.g. CI/scripts)
sakha login openai --api-key sk-...
```

Browse the full catalog (id, base URL, default model, auth method) with:

```bash
sakha providers catalog
```

If you'd rather manage the key yourself (env var or `.env` file — see
[.env Autoload](./docs/configuration.md#env-autoload)), point config at a
preset without logging in:

```bash
sakha providers use openai
export OPENAI_API_KEY=sk-...
```

Or hand-edit `~/.sakha/config.toml` directly:

**Local (Ollama) Example:**
```toml
db_path = "~/.sakha/sakha.db"

[provider]
selection = "openai_compatible"
base_url = "http://localhost:11434/v1"
model = "llama2:latest"
api_key_env = "OLLAMA_API_KEY"  # Leave empty if not required

# Optional: session store (use in-memory if omitted)
```

**OpenAI-Compatible Cloud Example:**
```toml
db_path = "~/.sakha/sakha.db"

[provider]
selection = "openai_compatible"
base_url = "https://api.openai.com/v1"
model = "gpt-4-turbo"
api_key_env = "OPENAI_API_KEY"
```

**Testing/Offline (Default):**
```toml
[provider]
selection = "mock"
model = "mock-model"
# No api_key_env needed; returns canned deterministic responses.
```

### 3. Run Commands

**Non-interactive run:**
```bash
sakha run "implement a retry policy for the http client"
```

Streams the response to stdout. Exit codes: `0` (success), `1` (model error), `2` (config error).

**Interactive chat:**
```bash
sakha chat
# Reads prompts from stdin (one per line). Type `exit` or `quit` to leave.
# Persists to `db_path` if configured; resume with `sakha session list` + `sakha session resume <id>`.
```

**Resume a session:**
```bash
sakha session list --output json
sakha session resume <session-id>
```

**List providers and tools:**
```bash
sakha providers list
sakha tools list
```

**Inspect configuration:**
```bash
sakha config get provider.model
sakha config set provider.model "gpt-4"
```

**Local daemon:**
```bash
sakha daemon run
# Starts HTTP server for background agents and WebSocket streaming.
# Default: http://localhost:9999
```

## Configuration Details

**Config file location:** `~/.sakha/config.toml` (or `$SAKHA_HOME/.sakha/config.toml` for testing/CI)

**Provider section:**
- `selection`: `"mock"` (offline, deterministic) or `"openai_compatible"` (remote/local endpoints)
- `base_url`: OpenAI-compatible endpoint URL (e.g., `http://localhost:11434/v1` for Ollama)
- `model`: Model name as recognized by the provider (e.g., `llama2:latest`, `gpt-4-turbo`)
- `api_key_env`: Name of environment variable holding the API key (e.g., `OPENAI_API_KEY`, `OLLAMA_API_KEY`). Omit for providers that don't require authentication.
- `preset`: Catalog preset id set automatically by `sakha login`/`sakha providers use` (e.g. `"openai"`). See [Provider Catalog & Login](./docs/providers.md).

**Optional:**
- `db_path`: Path to SQLite database for durable session storage. If omitted, sessions are in-memory (non-durable).

**Environment variable expansion:** Config values support `${VAR}` and `$VAR` syntax; unresolved references are left as-is.

## Example Workflow

```bash
# 1. Create a config for a local LLM (Ollama)
mkdir -p ~/.sakha
cat > ~/.sakha/config.toml << 'EOF'
[provider]
selection = "openai_compatible"
base_url = "http://localhost:11434/v1"
model = "llama2"

db_path = "~/.sakha/sakha.db"
EOF

# 2. (Ensure Ollama is running: ollama serve)

# 3. Run a quick task
sakha run "What are the top 3 patterns for error handling in Rust?"

# 4. Start an interactive session
sakha chat
# > add a retry decorator to this function
# > use exponential backoff
# > exit

# 5. List and resume sessions
sakha session list
sakha session resume <session-id>
```

## Full Specifications

Complete design, architecture, and implementation specs live in the [Sakha Coding Agent planning folder](https://github.com/sakha-agent/sakha/tree/main/docs):

- `00-executive-summary.md` — Core principles, non-goals, and first production slice
- `06-context-memory-state.md` — Context planning, budgeting, and session durability
- `13-ui-cli-tui-web-desktop.md` — CLI commands, TUI layouts, and web API
- `17-config-secrets-policy.md` — Configuration, secrets management, and security model
- `21-prompt-system.md` — Prompt assembly, compression, and context routing
- `modules/` — Deep dives on providers, tools, MCP, loops, and verification
- `crates/crate-work-breakdown.md` — Detailed crate responsibilities and public APIs
- [`docs/providers.md`](./docs/providers.md) — Provider catalog, `sakha login`/`sakha logout`, and the key-storage security model
- [`docs/configuration.md`](./docs/configuration.md) — Full `~/.sakha/config.toml` schema, including `.env` autoload

## License

MIT

## Contributing

Contributions follow clean-room principles: rebuild from specs and tests, not cloned source. See [CONTRIBUTING.md](./CONTRIBUTING.md) (if present) or open an issue/PR.
