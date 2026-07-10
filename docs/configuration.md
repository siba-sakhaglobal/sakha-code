# Configuration Reference

Sakha's configuration is stored in `~/.sakha/config.toml` (TOML format). This document describes every section and key that actually exists in the codebase.

## File Location

- **Linux/macOS**: `~/.sakha/config.toml`
- **Windows**: `%USERPROFILE%\.sakha\config.toml`
- **Override**: Set `SAKHA_HOME` environment variable to use a custom directory (useful for tests/CI)

The file is created automatically with defaults on first run. It does not exist until you first run a Sakha command.

## Environment Variable Expansion

The config supports `${VAR}` and `$VAR` expansion at load time:

```toml
api_key_env = "${API_KEY_VAR}"  # Expands to the value of $API_KEY_VAR
base_url = "${PROVIDER_URL}"     # Expands to the value of $PROVIDER_URL
```

Unresolved variables are left as-is (not an error), so partially-configured environments still parse.

## Top-Level Sections

### `[provider]` – LLM Provider Configuration

Configures which LLM backend Sakha talks to and how to connect.

#### `provider.selection`

**Type**: `"mock"` or `"open_ai_compatible"`  
**Default**: `"mock"`  
**Required**: No

Which provider backend to use:

- `"mock"` – MockProviderClient: no network, deterministic canned output. Always available offline. Use for initial testing and CI smoke tests.
- `"open_ai_compatible"` – OpenAiCompatibleClient: talks to any provider speaking the OpenAI API format (OpenAI, Ollama, LM Studio, xAI, local gateways, etc.).

```toml
[provider]
selection = "open_ai_compatible"
```

#### `provider.base_url`

**Type**: String  
**Default**: `"http://localhost:11434/v1"`  
**Required**: No (unless `selection = "open_ai_compatible"`)

The base URL for the provider's API endpoint. Ignored when `selection = "mock"`.

```toml
[provider]
base_url = "https://api.openai.com/v1"          # OpenAI
base_url = "http://localhost:11434/v1"          # Ollama (default)
base_url = "http://localhost:1234/v1"           # LM Studio
base_url = "https://your-custom-provider/v1"    # Custom provider
```

The client will POST requests to `{base_url}/chat/completions` for inference.

#### `provider.model`

**Type**: String  
**Default**: `"mock-model"`  
**Required**: No

The model name to request from the provider. For OpenAI-compatible providers, this is passed verbatim in the `model` field of the chat completion request.

```toml
[provider]
model = "gpt-4o-mini"              # OpenAI
model = "mistral"                  # Ollama
model = "local-model"              # LM Studio or local providers
```

Can be overridden per-command: `sakha run "prompt" --model gpt-4o`

#### `provider.api_key_env`

**Type**: String or null  
**Default**: `null`  
**Required**: No (unless your provider requires authentication)

The name of an environment variable holding the API key. The raw key itself is **never stored in the config file**.

```toml
[provider]
api_key_env = "OPENAI_API_KEY"     # Reads from $OPENAI_API_KEY
api_key_env = "MY_CUSTOM_KEY_VAR"  # Reads from $MY_CUSTOM_KEY_VAR
```

Set the environment variable before running Sakha:

```bash
export OPENAI_API_KEY="sk-..."
sakha run "prompt"
```

For providers that don't require auth (Ollama, LM Studio), leave this `null` or omit it.

#### `provider.preset`

**Type**: String or null
**Default**: `null`
**Required**: No

The [provider catalog](./providers.md) preset id (e.g. `"openai"`,
`"openrouter"`, `"ollama"`) this configuration was derived from, set
automatically by `sakha login <preset>` and `sakha providers use <preset>`.
Used only as a fallback key-lookup hint: when `provider.api_key_env` names an
environment variable that is **not** set in the process, Sakha checks the OS
credential manager for a secret stored under this preset id (via `sakha
login`) and loads it into that env var for the current process only — it is
never written back to the config file.

```toml
[provider]
selection = "open_ai_compatible"
base_url = "https://openrouter.ai/api/v1"
model = "openrouter/auto"
api_key_env = "OPENROUTER_API_KEY"
preset = "openrouter"
```

You do not need to set this by hand; `sakha login`/`sakha providers use`
manage it. See [Provider Catalog & Login](./providers.md) for the full
picture (OAuth vs. pasted-key vs. `--api-key`, and where keys actually live).

### `.env` Autoload

Before loading `~/.sakha/config.toml` or dispatching any command, Sakha
tries to load two `.env` files, in order:

1. `./.env` (current working directory)
2. `{SAKHA_HOME or ~}/.sakha/.env`

Both loads are **non-overriding**: a variable already present in the process
environment always wins over the same variable in a `.env` file. Missing
files are ignored silently — most setups won't have either.

```bash
# ./.env or ~/.sakha/.env
OPENAI_API_KEY=sk-...
OPENROUTER_API_KEY=sk-or-...
```

This is a convenience layered on top of the normal `provider.api_key_env`
resolution (env var → keyring fallback described above) — it does not change
config file syntax, and secrets are still never written to
`~/.sakha/config.toml`.

### `db_path` – Session Database

**Type**: String or null  
**Default**: `null`  
**Required**: No

> **⚠️ Placement matters**: `db_path` is a *top-level* key. In TOML, any key written below a `[section]` header belongs to that section — so `db_path` placed after `[provider]` becomes `provider.db_path` and is silently ignored. Put `db_path` at the very top of the file, before any `[section]`.

File path to an SQLite database for persistent session, turn, memory, and research storage.

- When set: sessions created by `sakha chat`, `sakha loop`, etc. persist across CLI invocations. `sakha session list`, `sakha session resume` work as expected.
- When `null` or omitted: in-memory store only. Sessions are lost when the process exits.

```toml
db_path = "/home/user/.sakha/sessions.sqlite3"
db_path = "/var/sakha/workspace-db.sqlite3"
db_path = "$HOME/.sakha/sessions.sqlite3"  # With env-var expansion
```

Create the directory if it doesn't exist:

```bash
mkdir -p ~/.sakha
```

The file is created automatically when first accessed. To inspect or manage sessions:

```bash
sqlite3 ~/.sakha/sessions.sqlite3
sqlite> .tables
sqlite> SELECT id, status, created_at FROM sessions LIMIT 5;
```

### Extra Custom Keys

Any additional keys you add are preserved under a catch-all `extra` table and can be read/written via `sakha config get` and `sakha config set`:

```toml
[provider]
selection = "open_ai_compatible"
model = "gpt-4o-mini"

[custom_settings]
my_workspace_path = "/path/to/workspace"
max_retries = 3
enable_debug_logging = true
```

Retrieve them:

```bash
sakha config get custom_settings.my_workspace_path
# Output: /path/to/workspace

sakha config get custom_settings.max_retries
# Output: 3
```

Set them:

```bash
sakha config set custom_settings.max_retries 5
sakha config set custom_settings.debug true
```

These are stored exactly as provided (with type inference: booleans, integers, floats parsed; strings otherwise).

## Complete Example Configs

### Minimal Config (Mock Provider, In-Memory Sessions)

```toml
[provider]
selection = "mock"
```

- Always works offline
- Sessions lost on exit
- Good for testing and CI

### Ollama (Local LLM)

```toml
db_path = "/home/user/.sakha/sessions.sqlite3"

[provider]
selection = "open_ai_compatible"
base_url = "http://localhost:11434/v1"
model = "mistral"
```

Prerequisites:
- Ollama running locally: `ollama serve`
- Model pulled: `ollama pull mistral`

### OpenAI

```toml
db_path = "/home/user/.sakha/sessions.sqlite3"

[provider]
selection = "open_ai_compatible"
base_url = "https://api.openai.com/v1"
model = "gpt-4o-mini"
api_key_env = "OPENAI_API_KEY"
```

Before running, set the API key:

```bash
export OPENAI_API_KEY="sk-..."
```

### LM Studio (Local)

```toml
db_path = "/home/user/.sakha/sessions.sqlite3"

[provider]
selection = "open_ai_compatible"
base_url = "http://localhost:1234/v1"
model = "local-model"
```

Prerequisites:
- LM Studio running locally with a model loaded

### Development/Testing (Mock + Persistent Sessions)

```toml
db_path = "/tmp/sakha-test-sessions.sqlite3"

[provider]
selection = "mock"
model = "mock-model"
```

Good for:
- Testing session management without network calls
- Offline CI pipelines
- Learning Sakha workflows

## Command-Line Config Management

### Get a Config Value

```bash
sakha config get provider.model
# Output: gpt-4o-mini

sakha config get db_path
# Output: /home/user/.sakha/sessions.sqlite3

sakha config get provider.selection --output json
# Output: {"key":"provider.selection","value":"open_ai_compatible"}
```

If the key doesn't exist, exit code is 3; otherwise 0.

### Set a Config Value

```bash
sakha config set provider.model "gpt-4o"
sakha config set provider.base_url "http://localhost:11434/v1"
sakha config set db_path "/home/user/.sakha/sessions.sqlite3"
sakha config set custom_settings.workspace_root "/path/to/work"
```

- Values are parsed as TOML scalars (bool, int, float) when possible; otherwise stored as strings.
- Parent tables are created as needed.
- Exit code 0 on success.

Verify with `get`:

```bash
sakha config get provider.model
# Output: gpt-4o
```

## Schema (Derived from Rust Structs)

This section describes the actual structure used internally.

### `ProviderSelection` Enum

```rust
pub enum ProviderSelection {
    Mock,
    OpenAiCompatible,  // TOML: "open_ai_compatible"
}
```

Default: `Mock`

### `ProviderConfig` Struct

```rust
pub struct ProviderConfig {
    pub selection: ProviderSelection,           // Default: Mock
    pub base_url: String,                       // Default: "http://localhost:11434/v1"
    pub model: String,                          // Default: "mock-model"
    pub api_key_env: Option<String>,            // Default: None
}
```

### `SakhaConfig` Struct (Top-Level)

```rust
pub struct SakhaConfig {
    pub provider: ProviderConfig,
    pub db_path: Option<String>,                // Default: None (in-memory store)
    pub extra: toml::value::Table,              // Catch-all for custom keys
}
```

All fields except `provider.selection` are optional; missing fields use defaults.

## Troubleshooting

### Config file not found / Uses defaults

**Issue**: `~/.sakha/config.toml` doesn't exist, and defaults are being used.

**Solution**: Create the file manually or run any Sakha command, which will create it. Then edit:

```bash
mkdir -p ~/.sakha
cat > ~/.sakha/config.toml << 'EOF'
db_path = "/home/user/.sakha/sessions.sqlite3"

[provider]
selection = "open_ai_compatible"
base_url = "https://api.openai.com/v1"
model = "gpt-4o-mini"
api_key_env = "OPENAI_API_KEY"
EOF
```

### Sessions not persisting

**Issue**: `sakha session list` is empty after restart, even though I ran `sakha chat` earlier.

**Solution**: Set `db_path` in config. Without it, sessions are in-memory only:

```bash
sakha config set db_path "$HOME/.sakha/sessions.sqlite3"
```

Then run a chat session and try listing again:

```bash
sakha chat
# (Type some messages, then 'exit')
sakha session list
```

### Provider returns 404 or connection refused

**Issue**: `sakha providers list` shows "Unavailable" or connection errors.

**Solution**:
1. Verify `base_url` matches your provider:
   ```bash
   sakha config get provider.base_url
   ```
2. For local providers (Ollama, LM Studio), confirm they're running:
   ```bash
   curl -I http://localhost:11434/v1/models  # Ollama
   curl -I http://localhost:1234/v1/models   # LM Studio
   ```
3. For OpenAI, check internet connectivity.

### API key not found / 401 Unauthorized

**Issue**: Provider returns 401 or "unauthorized".

**Solution**:
1. Verify `provider.api_key_env` is set to the correct environment variable name:
   ```bash
   sakha config get provider.api_key_env
   ```
2. Verify the environment variable is actually set and not empty:
   ```bash
   echo $OPENAI_API_KEY  # Linux/macOS
   echo $env:OPENAI_API_KEY  # PowerShell
   ```
3. API keys are never logged. Check your provider's API key management page to ensure it's valid.

### Config value shows as string instead of type

**Issue**: `sakha config get custom_settings.enabled` returns `"true"` (string) instead of boolean.

**Solution**: Re-set it without quotes to let the parser infer type:

```bash
sakha config set custom_settings.enabled true
sakha config get custom_settings.enabled
# Now returns: true (boolean)
```

Or use the config file directly to control type.

### Unrecognized provider.selection

**Issue**: Error like `"unknown variant mock_mode"`.

**Solution**: Use the correct enum values:

- `"mock"` (not `"mock_mode"`, `"Mock"`, etc.)
- `"open_ai_compatible"` (not `"openai"`, `"OpenAiCompatible"`, etc.)

Case matters (TOML snake_case).

## Migration Guide

### From Mock to Real Provider

1. Choose a provider (OpenAI, Ollama, LM Studio, etc.)
2. Obtain credentials (API key or local URL)
3. Update config:
   ```bash
   sakha config set provider.selection "open_ai_compatible"
   sakha config set provider.base_url "https://api.openai.com/v1"
   sakha config set provider.model "gpt-4o-mini"
   sakha config set provider.api_key_env "OPENAI_API_KEY"
   ```
4. Set environment variable:
   ```bash
   export OPENAI_API_KEY="sk-..."
   ```
5. Test:
   ```bash
   sakha providers list
   sakha run "test prompt"
   ```

### From In-Memory to Persistent Sessions

1. Configure database:
   ```bash
   sakha config set db_path "$HOME/.sakha/sessions.sqlite3"
   ```
2. Create the directory:
   ```bash
   mkdir -p ~/.sakha
   ```
3. Test:
   ```bash
   sakha chat
   # (exit)
   sakha session list
   ```

Prior in-memory sessions are not migrated (they were ephemeral).

## See Also

- [Getting Started](./getting-started.md) – Tutorial and first-run guide
- `sakha config --help` – Command-line help
- `sakha config get <key> --output json` – Machine-readable output
