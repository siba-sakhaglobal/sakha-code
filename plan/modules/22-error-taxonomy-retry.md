# Module 22: Error Taxonomy, Retry, and Rate-Limit Policy

## Purpose

One cross-cutting error model so every layer (provider, tools, loop, daemon) classifies failures the same way and retry/abort decisions are policy, not scattered `match` arms.

## Error Taxonomy

| Class | Examples | Default handling |
|---|---|---|
| `Transient` | network reset, 429, 500/502/503, provider overload | retry with backoff |
| `Budget` | token/cost/time budget exhausted | stop loop, write handoff |
| `Permission` | denied tool call, sandbox block | surface to user, never retry silently |
| `InvalidInput` | bad tool args, schema mismatch | return to model once for self-correction, then fail turn |
| `Integrity` | checkpoint corrupt, migration failure, patch conflict | halt, require human/repair path |
| `Fatal` | config invalid, auth invalid | abort with actionable message |

All errors carry: class, source module, retryable flag, provider/tool id, and an audit event.

## Retry Policy

- Exponential backoff with full jitter: base 500ms, cap 60s, max 5 attempts for `Transient`.
- Respect `Retry-After` headers when present (providers, web fetch).
- Retry budget is part of the turn `Budget` — retries debit the same ledger, so loops cannot burn unbounded retries.
- Idempotency: only auto-retry tool calls marked idempotent in their `ToolSpec`; non-idempotent side effects require model re-issue.

## Rate Limiting

- Per-provider token-bucket (requests/min and tokens/min) configured in the provider profile.
- 429 responses feed back into the bucket (adaptive cooldown).
- Loop scheduler must consult provider limiter before dispatching ticks.

## Interfaces

```rust
enum ErrorClass { Transient, Budget, Permission, InvalidInput, Integrity, Fatal }
struct SakhaError { class: ErrorClass, retryable: bool, source: ModuleId, msg, cause }
trait RetryPolicy { fn next_delay(&self, attempt: u32, err: &SakhaError) -> Option<Duration>; }
```

## Test Requirements

- Backoff sequence with jitter bounds.
- Retry stops at budget exhaustion even below max attempts.
- Non-idempotent tool never auto-retried.
- 429 with `Retry-After` honored exactly.

## Crate Mapping

Taxonomy lives in `sakha-core` (`error.rs`); retry/limiter helpers in `sakha-core` (`retry.rs`) reused by `sakha-provider` and `sakha-loop`.
