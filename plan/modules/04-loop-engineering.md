# Module 04: Loop Engineering

## Responsibility

Turn manual prompting into designed loops: scheduled, event-driven, verification-driven, research-driven, feedback-driven, and deterministic replay loops.

## Rust Crate

`crates/sakha-loop`

## Main Structs

- `LoopSpec`
- `LoopRuntime`
- `LoopTick`
- `Trigger`
- `ScheduleTrigger`
- `EventTrigger`
- `WebhookTrigger`
- `LoopQueue`
- `LoopState`
- `LoopBudget`
- `LoopWatchdog`
- `LoopSkill`
- `ReplayTrace`
- `FeedbackRecord`

## Loop Layers

1. **Agent loop**: model calls tools until it can answer.
2. **Verification loop**: run checks, feed failures back, retry.
3. **Event loop**: schedule/webhook/queue triggers tasks.
4. **Research loop**: search, read, compress, synthesize, verify.
5. **Feedback loop**: user/CI/runtime results update memory and policies.
6. **Replay loop**: deterministic replay of previously learned workflows.

## Required Capabilities

- Cron/interval schedules.
- Event sources:
  - GitHub issue/PR/CI event.
  - Webhook.
  - Filesystem watcher.
  - Queue message.
  - Manual goal.
  - Search monitor.
- Worktree per loop tick.
- Loop queue with leases.
- Tick checkpointing.
- Durable memory file for every loop.
- Loop watchdog.
- Loop analytics.
- Human approval gates.

## Loop Control Rules

- Every loop must define max wall-clock time.
- Every loop must define max iterations.
- Every loop must define max provider cost.
- Every loop must define side-effect permissions.
- Every loop must write a handoff artifact after each tick.
- Every loop must be pausable.
- Every loop must be idempotent or explicitly marked non-idempotent.

## Infinite Loop Prevention

Implement:

- State hash comparison.
- Repeated tool-call detector.
- Repeated prompt detector.
- Cost growth guard.
- Context growth guard.
- Same-error recurrence detector.
- External side-effect deduplication.
- Max retry per failure category.

## Deterministic Replay Suggestion

Add `LoopSkillRecorder`:

- Records successful tool trajectory.
- Extracts variables from inputs/outputs.
- Produces `LoopSkill` template.
- Verifies replay in dry-run.
- Runs future periodic tasks without LLM unless unexpected state appears.

## Implementation Tasks

1. Define `LoopSpec` schema.
2. Implement schedule engine.
3. Implement event queue.
4. Implement worktree lease manager.
5. Implement loop watchdog.
6. Implement verification loop.
7. Implement loop memory/handoff writer.
8. Implement deterministic trace recorder.
9. Implement replay engine.
10. Implement loop dashboard API.

## Tests

- Scheduled loop runs once.
- Event loop deduplicates event.
- Loop pauses/resumes.
- Budget stops loop.
- Repeated failure blocks loop.
- Worktree isolation works.
- Replay skill executes without model call.

