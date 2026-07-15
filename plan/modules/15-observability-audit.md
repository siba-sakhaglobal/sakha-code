# Module 15: Observability and Audit

## Responsibility

Make agent behavior inspectable: logs, traces, metrics, audit events, cost accounting, compression stats, and loop diagnostics.

## Rust Crate

`crates/sakha-observability`

## Main Structs

- `AuditLogger`
- `TraceSpan`
- `MetricRecorder`
- `CostLedger`
- `TokenLedger`
- `CompressionLedger`
- `ToolAuditRecord`
- `LoopDiagnostic`
- `SessionReport`

## Event Categories

- Session.
- Model request.
- Tool call.
- Permission decision.
- File write.
- Command execution.
- Compression.
- Retrieval.
- Verification.
- Loop tick.
- Error.
- Budget.

## Metrics

- Tokens in/out.
- Tokens saved by compression.
- Compression retrieval rate.
- Provider latency.
- Tool latency.
- Tool failure rate.
- Loop iteration count.
- Cost per goal.
- Verification pass rate.
- Handoff quality score.

## Audit Requirements

- Immutable append-only audit log.
- Redacted but traceable secret events.
- Link every file write to tool call and turn.
- Link every model request to context items.
- Link every compressed item to raw artifact.
- Export session report.

## Implementation Tasks

1. Implement structured tracing.
2. Implement audit event schema.
3. Implement cost ledger.
4. Implement compression ledger.
5. Implement loop diagnostics.
6. Implement report generator.
7. Implement UI metrics API.
8. Implement log redaction.
9. Implement optional OpenTelemetry exporter.
10. Implement offline audit export.

## Tests

- Audit event written for tool call.
- Secret redacted from logs.
- Cost ledger totals across session.
- Compression savings calculated.
- Session report includes commands and changed files.

