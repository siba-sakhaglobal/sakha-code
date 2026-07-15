# Phase 08: Security and Sandbox

## Goal

Harden tool execution and secret handling.

## Tasks

### 08.1 Policy Engine

- Define policy schema.
- Merge policy layers.
- Add allow/deny lists.
- Add approval modes.

### 08.2 Secret Store

- Secret references.
- OS keychain integration.
- Env fallback.
- Redaction.

### 08.3 Sandbox

- Linux sandbox prototype.
- macOS sandbox design.
- Windows job object design.
- Network policy.
- Filesystem policy.

### 08.4 Risk Classifier

- Shell command risk.
- File operation risk.
- Git operation risk.
- MCP connector risk.

### 08.5 Audit

- Permission audit.
- Secret access audit.
- Sandbox violation audit.
- Export report.

## Definition of Done

- Denied tool cannot run.
- Secret does not appear in model context.
- Shell command risk requires approval.
- Sandbox blocks outside-workspace write.
- Audit report includes all approvals.

