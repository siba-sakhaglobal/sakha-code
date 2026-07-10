# Getting Started with Sakha

Sakha is an agentic coding assistant that runs locally via CLI. This guide covers installation, first run with the mock provider, connecting to a real LLM provider (OpenAI-compatible), basic permissions, session resume, and starting a long-running loop.

## Installation

### Prerequisites
- Rust 1.70+ (check `rust-toolchain.toml` in the repo)
- Cargo (comes with Rust)

### Build from Source

Clone the repository and build the CLI binary:

```bash
git clone https://github.com/sakha-agent/sakha.git
cd sakha
cargo build --release
```

The binary will be at `target/release/sakha` (or `.exe` on Windows).

### Add to PATH (Optional)

For convenience, move the binary to a directory on your `PATH`:

```bash
# Linux/macOS
sudo cp target/release/sakha /usr/local/bin/

# Windows (PowerShell, as Administrator)
Copy-Item -Path target\release\sakha.exe -Destination $env:ProgramFiles\sakha\
# Then add $env:ProgramFiles\sakha to your PATH
```

Or run it directly:

```bash
./target/release/sakha --version
```

## First Run: Mock Provider

By default, Sakha uses the **mock provider**, which requires no API key and returns deterministic test responses. This is perfect for your first test.

### Run a Single Command

```bash
sakha run "hello world"
```

Output:
```
Mock provider response for: hello world
```

Exit code `0` = success. The mock provider is always available offline.

### Start an Interactive Chat Session

```bash
sakha chat
```

Prompt:
```
sakha chat: session 550e8400-e29b-41d4-a716-446655440000 (type 'exit' to leave)
>
```

Type a message and press Enter:

```
> what is rust?
Mock provider response for: what is rust?
> exit
```

Sessions are stored in memory by default (lost when the process exits). To persist sessions across restarts, configure a database path (see [Configuration](./configuration.md)).

## Connecting to a Real Provider

Sakha supports OpenAI-compatible LLM providers (OpenAI, Ollama, local LM Studio, xAI Grok, or any compatible service).

### Step 1: Set Up Your Provider

Choose one:

#### OpenAI

1. Create an account at https://platform.openai.com
2. Generate an API key in Account Settings → API Keys
3. Store it safely (never commit to git)

#### Ollama (Local)

1. Download from https://ollama.ai
2. Run: `ollama serve`
3. In another terminal: `ollama pull mistral` (or another model)
4. Default base URL: `http://localhost:11434/v1`
5. No API key needed

#### LM Studio (Local)

1. Download from https://lmstudio.ai
2. Load a model and start the local server (default: `http://localhost:1234/v1`)
3. No API key needed

### Step 2: Configure Sakha

The config file is at `~/.sakha/config.toml`. Create it or edit if it exists.

#### Option A: Using the CLI

```bash
# Set the provider selection
sakha config set provider.selection open_ai_compatible

# Set the base URL (replace with your provider's)
sakha config set provider.base_url "https://api.openai.com/v1"

# Set the model name
sakha config set provider.model "gpt-4o-mini"

# Set the environment variable that holds your API key
sakha config set provider.api_key_env "OPENAI_API_KEY"
```

#### Option B: Editing the Config File Directly

Create or edit `~/.sakha/config.toml`:

```toml
[provider]
selection = "open_ai_compatible"
base_url = "https://api.openai.com/v1"
model = "gpt-4o-mini"
api_key_env = "OPENAI_API_KEY"
```

For Ollama:
```toml
[provider]
selection = "open_ai_compatible"
base_url = "http://localhost:11434/v1"
model = "mistral"
```

### Step 3: Set Your API Key

Store your API key in an environment variable (never in the config file).

```bash
# Linux/macOS
export OPENAI_API_KEY="sk-..."

# Windows (PowerShell)
$env:OPENAI_API_KEY = "sk-..."

# Windows (Command Prompt)
set OPENAI_API_KEY=sk-...
```

Or persist it in your shell profile (`~/.bashrc`, `~/.zshrc`, or PowerShell profile).

### Step 4: Test the Connection

```bash
# Check provider health
sakha providers list
```

Output:
```json
[
  {
    "selection": "OpenAiCompatible",
    "model": "gpt-4o-mini",
    "base_url": "https://api.openai.com/v1",
    "health": "Healthy",
    "capabilities": {
      "supports_streaming": true,
      "supports_tools": true,
      ...
    }
  }
]
```

### Step 5: Run a Command

```bash
sakha run "write a python hello world script"
```

The real LLM provider will respond.

## Permissions Basics

Sakha has a permission system to gate sensitive operations (file writes, shell commands, network access, etc.).

### Default Behavior

By default, Sakha allows most operations in the current workspace. Some operations (like destructive shell commands) require approval or explicit policy.

### Common Permission Types

- `file.read` – Read files
- `file.write` – Write files
- `file.delete` – Delete files
- `shell.run_safe` – Run non-destructive shell commands
- `shell.run_arbitrary` – Run any shell command
- `network.access` – Fetch URLs
- `git.remote_push` – Push to git remotes
- `package.install` – Install dependencies
- `secret.read` – Access secrets/API keys

### Checking Permission Status

Use `sakha security check` (future command) to preview what permissions a tool would request.

For now, deny/allow decisions appear in logs when operations occur.

## Session Resume

Sessions persist when you configure a database path.

### Enable Session Persistence

Edit `~/.sakha/config.toml` and add:

```toml
db_path = "/path/to/sessions.sqlite3"
```

Or via CLI:

```bash
sakha config set db_path "$HOME/.sakha/sessions.sqlite3"
```

### List Sessions

```bash
sakha session list
```

Output:
```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "status": "completed",
    "created_at": "2024-07-10T15:30:00Z",
    "updated_at": "2024-07-10T15:32:00Z"
  }
]
```

### Show Session Details

```bash
sakha session show 550e8400-e29b-41d4-a716-446655440000
```

### Resume a Session

```bash
sakha chat --session-id 550e8400-e29b-41d4-a716-446655440000
```

The chat will reload the session's prior turn history and continue from where it left off.

## Starting a Loop

Loops are long-running, goal-driven execution modes where Sakha iteratively works toward an objective.

### Basic Loop

```bash
sakha loop start "implement user authentication module" --kind agent
```

Output:
```json
{
  "loop_id": "loop-abc123",
  "objective": "implement user authentication module",
  "max_iterations": 50
}
```

The loop runs up to 50 iterations (default) trying to achieve the objective.

### Loop Kinds

- `agent` (default) – General-purpose agentic loop
- `verification` – Verify a completed task
- `event` – Event-driven loop (awaits external input)
- `research` – Research-focused loop
- `feedback` – Feedback collection loop
- `replay` – Replay prior execution for debugging

### Control a Loop

```bash
# List running loops
sakha loop list

# Pause a loop
sakha loop pause loop-abc123

# Resume a paused loop
sakha loop resume loop-abc123

# Stop a loop
sakha loop stop loop-abc123
```

### Customize Loop Iterations

```bash
sakha loop start "objective" --max-iterations 100
```

## Next Steps

- Read [Configuration](./configuration.md) for a full config reference
- Use `sakha --help` to see all available commands
- Check `sakha <command> --help` for command-specific options
- Run `sakha doctor` to diagnose your installation

## Troubleshooting

### Config file not found
Sakha creates `~/.sakha/config.toml` automatically on first run with defaults. If missing, create the directory:

```bash
mkdir -p ~/.sakha
```

### Provider returns "connection refused"
- Verify the `base_url` in config matches your provider's actual address
- For Ollama/LM Studio, confirm the local server is running
- For OpenAI, check your internet connection

### Sessions not persisting
- Confirm `db_path` is set in config and the path is writable
- Check the directory exists: `mkdir -p $(dirname ~/.sakha/sessions.sqlite3)`

### API key not recognized
- Verify the environment variable name matches `provider.api_key_env` in config
- Ensure the variable is actually set: `echo $OPENAI_API_KEY` (or `$env:OPENAI_API_KEY` on Windows)
- API keys are never logged; check the provider's documentation

### Mock provider always returns canned responses
That's correct! The mock provider is deterministic for testing. Switch to a real provider in config to get dynamic responses.
