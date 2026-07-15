# Module 14: Security, Sandbox, and Permissions

## Responsibility

Constrain agent actions, protect secrets, enforce human approvals, sandbox tools, and provide auditable safety.

## Rust Crate

`crates/sakha-security`

## Main Structs

- `PermissionPolicy`
- `PermissionRequest`
- `PermissionDecision`
- `SandboxProfile`
- `SecretRef`
- `SecretStore`
- `Redactor`
- `RiskClassifier`
- `PolicySource`
- `SecurityAudit`

## Policy Sources

Priority order:

1. CLI flags.
2. Workspace local policy.
3. Project policy.
4. User policy.
5. Organization policy.
6. Built-in defaults.

## Permission Types

- File read.
- File write.
- File delete.
- Shell run.
- Network access.
- Git remote push.
- Browser automation.
- Secret read.
- MCP connector call.
- External side effect.
- Package install.

## Sandbox Profiles

- `read_only`
- `workspace_write`
- `network_disabled`
- `network_allowed`
- `build_test`
- `full_trust`
- `custom`

## Secret Rules

- Secrets are never inserted into model context.
- Tools receive secret handles, not raw values, where possible.
- Logs redact secrets.
- Tool output scanned for leaked secrets.
- Secret access is audited.

## Implementation Tasks

1. Define policy schema.
2. Implement policy merge.
3. Implement risk classifier.
4. Implement permission request flow.
5. Implement secret store abstraction.
6. Implement redactor.
7. Implement sandbox adapter trait.
8. Implement Linux sandbox adapter.
9. Implement macOS/Windows strategy docs.
10. Implement security audit exports.

## Tests

- Higher-priority policy overrides lower.
- Secret redaction catches known secret.
- Denied permission blocks tool.
- Sandbox prevents outside-workspace write.
- Network disabled blocks fetch tool.
- Audit log records every decision.

