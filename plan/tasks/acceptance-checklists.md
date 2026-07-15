# Acceptance Checklists

## Core Runtime

- [ ] Session persists.
- [ ] Session resumes.
- [ ] Cancellation propagates.
- [ ] Budget stops work.
- [ ] Audit events persist.

## Provider Gateway

- [ ] OpenAI-compatible streaming works.
- [ ] Tool calls normalize.
- [ ] JSON schema mode works or disables by capability.
- [ ] Usage records persist.
- [ ] Provider errors are readable.

## Agent Loop

- [ ] Agent can answer without tools.
- [ ] Agent can call tools.
- [ ] Agent can edit files.
- [ ] Agent stops on completion.
- [ ] Agent blocks on repeated failure.

## Tool Runtime

- [ ] Permission checked before side effect.
- [ ] File patch applies safely.
- [ ] Shell timeout works.
- [ ] Output compressed when large.
- [ ] Audit links tool to turn.

## Headroom

- [ ] Compression enabled by policy.
- [ ] Raw artifact retrievable.
- [ ] Compression stats visible.
- [ ] Failure policy respected.
- [ ] Eval catches over-compression.

## Loop Engineering

- [ ] Scheduled loop runs.
- [ ] Event loop deduplicates.
- [ ] Handoff written each tick.
- [ ] Watchdog catches repeated action.
- [ ] Replay skill dry-runs.

## Web Research

- [ ] Search query planner works.
- [ ] Fetch/extract works.
- [ ] Sources scored.
- [ ] Evidence pack stored.
- [ ] Final answer cites sources.

## Security

- [ ] Secrets redacted.
- [ ] Sandbox blocks disallowed writes.
- [ ] Network policy enforced.
- [ ] Dangerous command requires approval.
- [ ] Audit export complete.

## UI

- [ ] CLI run mode works.
- [ ] TUI chat works.
- [ ] Web timeline works.
- [ ] Permission UI works.
- [ ] Compression stats visible.

## Release

- [ ] All evals pass threshold.
- [ ] Install docs complete.
- [ ] Docker smoke test passes.
- [ ] Desktop smoke test passes.
- [ ] License/compliance review complete.

