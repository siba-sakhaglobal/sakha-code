# Clean-Room and Migration Strategy

## Goal

Create Sakha Coding Agent as a new implementation based on requirements, behavior, interfaces, and tests.

Do not copy source code from the existing TypeScript project. Use the existing repository only to identify features and migration requirements.

## Clean-Room Method

### Allowed Inputs

- High-level feature list.
- Public API contracts.
- User-observable behavior.
- Test cases written from behavior.
- Public documentation for dependencies.
- This planning workspace.

### Disallowed Inputs

- Copying implementation bodies.
- Copying proprietary prompts.
- Copying exact user-facing copy where not necessary.
- Preserving product-specific branding.
- Preserving legal-risky provenance in new generated code.

## Migration Phases

1. **Inventory**: list all features and decide keep/rewrite/drop/quarantine.
2. **Spec**: write behavior specs and acceptance tests.
3. **Skeleton**: create Rust workspace, crates, schemas, and local daemon.
4. **Vertical slices**: build one end-to-end path at a time.
5. **Parity matrix**: map old feature category to new module or rejected feature.
6. **Clean-room review**: ensure code is written from specs and docs.
7. **Security review**: threat model every tool and automation loop.
8. **Release gates**: evals, audit logs, packaging, docs, license review.

## Feature Disposition Tags

- `KEEP`: provider-neutral and required.
- `REWRITE`: required but must be implemented from scratch.
- `ADAPT`: use public protocol/dependency but new implementation.
- `DROP`: not needed.
- `QUARANTINE`: unsafe/legal/provider-specific until redesigned.

## Initial Disposition

| Feature Area | Disposition | Notes |
|---|---|---|
| Core agent loop | REWRITE | Implement in Rust using new traits |
| Provider gateway | REWRITE | Provider-neutral from day one |
| File/shell/git tools | REWRITE | Same behavior category, new implementation |
| MCP | ADAPT | Use public MCP protocol and SDK/docs |
| Headroom | ADAPT | Integrate as external open-source dependency/service |
| Web UI | REWRITE | New branding and own interaction model |
| Desktop | NEW | Tauri recommended |
| OAuth/first-party account flows | QUARANTINE | Remove unless tied to owned infrastructure |
| Remote browser extension bridge | QUARANTINE | Redesign from owned protocol |
| Telemetry exporters | REWRITE | Local-first, opt-in only |
| GitHub Action templates | REWRITE | Use owned action or generic CI template |

## Output Artifacts

- `specs/features/README.md`: behavior spec index.
- `crates/*`: Rust implementation.
- `schemas/*.json`: stable JSON schemas.
- `evals/*.yaml`: agent eval definitions.
- `docs/security/`: threat models.
- `docs/adr/`: architecture decision records.
