# Module 17: Config, Secrets, and Policy

## Responsibility

Load settings, provider profiles, policies, secrets, and workspace defaults consistently.

## Rust Crate

`crates/sakha-config`

## Main Structs

- `ConfigLoader`
- `ConfigLayer`
- `Settings`
- `ProviderProfile`
- `WorkspacePolicy`
- `SecretStoreConfig`
- `EnvAlias`
- `ConfigMigration`

## Config Layers

1. Built-in defaults.
2. System config.
3. User config.
4. Workspace config.
5. Project config.
6. Environment variables.
7. CLI flags.

## Key Files

- `~/.sakha/config.toml`
- `~/.sakha/providers.toml`
- `~/.sakha/policies.toml`
- `.sakha/settings.toml`
- `.sakha/agents/*.toml`
- `.sakha/loops/*.toml`
- `.sakha/skills/`

## Backward Compatibility

During migration, read legacy aliases but write new names:

- `CLAUDE_CODE_USE_OPENAI_COMPATIBLE` -> `SAKHA_USE_OPENAI_COMPATIBLE`
- `OPENAI_COMPATIBLE_*` remains valid provider-neutral fallback.
- `CLAUDE_CONFIG_DIR` -> `SAKHA_CONFIG_DIR`

## Implementation Tasks

1. Define config schema.
2. Implement TOML loader.
3. Implement layer merge.
4. Implement env aliases.
5. Implement validation.
6. Implement config migration.
7. Implement secret store abstraction.
8. Implement provider profile resolver.
9. Implement policy resolver.
10. Implement `sakha config doctor`.

## Tests

- Layer precedence.
- Env alias compatibility.
- Invalid config gives clear error.
- Secrets never serialized into config dump.
- Migration writes new file format.
