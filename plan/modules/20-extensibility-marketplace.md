# Module 20: Extensibility and Marketplace

## Responsibility

Let users add tools, skills, prompts, provider adapters, loop templates, and connectors safely.

## Rust Crate

`crates/sakha-extensions`

## Extension Types

- Skill.
- Prompt pack.
- Tool pack.
- MCP server pack.
- Provider adapter.
- Loop template.
- Verification pack.
- Compression plugin.
- UI panel plugin.

## Main Structs

- `ExtensionManifest`
- `ExtensionRegistry`
- `Skill`
- `PromptTemplate`
- `LoopTemplate`
- `ToolPack`
- `InstallPlan`
- `ExtensionTrust`
- `ExtensionSignature`

## Manifest Fields

- `id`
- `name`
- `version`
- `kind`
- `description`
- `publisher`
- `permissions`
- `entrypoints`
- `files`
- `config_schema`
- `dependencies`
- `signature`
- `license`

## Trust Levels

- Built-in.
- Local workspace.
- User-installed.
- Organization-approved.
- Untrusted.
- Blocked.

## Marketplace Rules

- No auto-install without approval.
- Permissions shown before install.
- Signature verification where supported.
- Sandboxed execution by default.
- Disable/remove must be simple.
- Extension data stored separately.

## Implementation Tasks

1. Define manifest schema.
2. Implement extension discovery.
3. Implement install plan.
4. Implement trust policy.
5. Implement skill loader.
6. Implement prompt template loader.
7. Implement loop template loader.
8. Implement MCP server pack loader.
9. Implement extension update flow.
10. Implement extension audit.

## Tests

- Valid manifest installs.
- Invalid manifest rejected.
- Permission display complete.
- Blocked extension cannot run.
- Skill is discoverable.
- Loop template creates valid `LoopSpec`.

