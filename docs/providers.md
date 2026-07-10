# Provider Catalog & Login

Sakha ships a static catalog of well-known LLM providers/gateways and a
`sakha login` command that authenticates against one and wires up
`~/.sakha/config.toml` automatically. This document covers the catalog, the
three ways to obtain a key, and how keys are stored.

## The Catalog

`sakha providers catalog` lists every built-in preset:

```bash
sakha providers catalog
sakha providers catalog --output json
```

| id | Kind | Base URL | Default model | Auth |
|----|------|----------|----------------|------|
| `openai` | Direct | `https://api.openai.com/v1` | `gpt-5-mini` | api-key |
| `anthropic` | Direct | `https://api.anthropic.com/v1` | `claude-sonnet-5` | api-key |
| `gemini` | Direct | `https://generativelanguage.googleapis.com/v1beta/openai` | `gemini-3.1-flash-lite` | api-key |
| `xai` | Direct | `https://api.x.ai/v1` | `grok-4-fast` | api-key |
| `mistral` | Direct | `https://api.mistral.ai/v1` | `mistral-small-latest` | api-key |
| `deepseek` | Direct | `https://api.deepseek.com/v1` | `deepseek-chat` | api-key |
| `groq` | Direct | `https://api.groq.com/openai/v1` | `llama-3.3-70b-versatile` | api-key |
| `cerebras` | Direct | `https://api.cerebras.ai/v1` | `llama-3.3-70b` | api-key |
| `together` | Aggregator | `https://api.together.xyz/v1` | `meta-llama/Llama-3.3-70B-Instruct-Turbo` | api-key |
| `fireworks` | Aggregator | `https://api.fireworks.ai/inference/v1` | `accounts/fireworks/models/llama-v3p3-70b-instruct` | api-key |
| `deepinfra` | Aggregator | `https://api.deepinfra.com/v1/openai` | `meta-llama/Llama-3.3-70B-Instruct` | api-key |
| `moonshot` | Direct | `https://api.moonshot.ai/v1` | `kimi-k2-turbo-preview` | api-key |
| `openrouter` | Aggregator | `https://openrouter.ai/api/v1` | `openrouter/auto` | **oauth** |
| `vercel-ai-gateway` | Gateway | `https://ai-gateway.vercel.sh/v1` | `openai/gpt-5-mini` | api-key |
| `portkey` | Gateway | `https://api.portkey.ai/v1` | `gpt-5-mini` | api-key |
| `requesty` | Aggregator | `https://router.requesty.ai/v1` | `openai/gpt-5-mini` | api-key |
| `litellm` | Gateway | `http://localhost:4000` (override with `--base-url`) | — | api-key |
| `ollama` | Local | `http://localhost:11434/v1` | `llama3.3` | none |
| `lmstudio` | Local | `http://localhost:1234/v1` | `local-model` | none |

Each preset also names a canonical environment variable (e.g.
`OPENAI_API_KEY`, `OPENROUTER_API_KEY`) that `provider.api_key_env` gets set
to once you log in or `use` it.

The `KEY?` column in `sakha providers catalog` reports whether a key is
*currently available* for that preset — either the env var is already set in
your shell, or you've previously run `sakha login` for it.

## Three Ways to Get a Key Into Sakha

### 1. `sakha login <preset>` — OAuth (OpenRouter only)

```bash
sakha login openrouter
```

Opens your browser to OpenRouter's authorization page using the PKCE flow
(no client secret involved): Sakha generates a code verifier/challenge pair,
starts a one-shot local HTTP listener on `127.0.0.1`, and waits (up to 5
minutes) for the redirect carrying the authorization code. It then exchanges
that code for an API key and stores it.

### 2. `sakha login <preset>` — pasted key (everyone else)

```bash
sakha login openai
sakha login gemini
```

Opens the provider's key-management console in your browser (when known) and
prompts you to paste the key back, with input hidden (via `rpassword`).

### 3. `sakha login <preset> --api-key <KEY>` — fully non-interactive

```bash
sakha login openai --api-key sk-...
```

Skips the browser and any prompt entirely — useful for scripts and CI. Works
for every preset, including OpenRouter (bypasses OAuth).

After any of the above, Sakha validates the key with `GET {base_url}/models`
(15s timeout) unless you pass `--skip-validate`. A failed validation is only
ever a **warning** — some providers don't expose `/models` even for valid
keys — the key is stored regardless.

### Configuring without logging in

If you already export the key yourself (env var, `.env` file, secrets
manager), you don't need `sakha login` at all — just point config at the
preset:

```bash
sakha providers use openai
export OPENAI_API_KEY=sk-...
```

`sakha providers use <preset>` writes `provider.selection`, `base_url`,
`model`, and `api_key_env` from the catalog entry, without touching the
keyring. It warns (non-fatally) if no key is found anywhere for the preset.

## Logging Out

```bash
sakha logout openrouter
```

Removes the stored key from the OS credential manager. It does not change
`~/.sakha/config.toml`.

## Security Model

- **Raw API keys are never written to `~/.sakha/config.toml`.** The config
  file only ever stores an environment variable *name*
  (`provider.api_key_env`) and, when obtained via `sakha login`/`sakha
  providers use`, the catalog preset id (`provider.preset`).
- Keys obtained via `sakha login` are stored in the **OS credential manager**
  (Windows Credential Manager, macOS Keychain, or the Secret Service API on
  Linux) under the service name `sakha`, one entry per preset
  (`provider:<preset-id>`).
- Keys can also come from a plain environment variable — set directly, or
  loaded from a `.env` file (see [Configuration](./configuration.md#env-autoload)).
- **Precedence when Sakha builds a provider client:** if the environment
  variable named by `provider.api_key_env` is already set in the process,
  that value is used and the keyring is never consulted. Only when it's unset
  does Sakha look up the keyring entry for `provider.preset` and load it into
  that env var for the current process. This means an explicit `export
  OPENAI_API_KEY=...` always overrides a stored `sakha login` key.
- Sakha never logs or prints a raw key. Anywhere a key needs to be
  acknowledged in output, it's masked to `\u{2022}\u{2022}\u{2022}` plus at
  most the last 4 characters.

## See Also

- [Configuration Reference](./configuration.md) — full `~/.sakha/config.toml` schema, including `.env` autoload
- `sakha login --help`
- `sakha providers catalog --help`
- `sakha providers use --help`
