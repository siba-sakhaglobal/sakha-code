# Module 16: Subagents and Teams

## Responsibility

Run multiple specialized agents safely with isolation, coordination, and review.

## Rust Crate

`crates/sakha-team`

## Main Structs

- `AgentProfile`
- `AgentTeam`
- `SubAgent`
- `TaskAssignment`
- `WorktreeLease`
- `ReviewRequest`
- `ReviewResult`
- `CoordinationBoard`
- `AgentMessage`

## Agent Profiles

- Planner.
- Implementer.
- Reviewer.
- Researcher.
- Tester.
- Security auditor.
- Refactor specialist.
- Documentation writer.
- Release engineer.

## Coordination Model

- Coordinator owns goal and plan.
- Sub-agents receive scoped tasks.
- Each writing sub-agent uses isolated worktree.
- Reviewer verifies diffs before merge.
- Coordinator merges accepted work.
- All agents write handoffs.

## Communication Rules

- Messages are persisted.
- Artifacts referenced by ID.
- No agent sees secrets unless explicitly allowed.
- Sub-agent context is minimized and compressed.
- Coordinator sees summaries and diffs, not full logs by default.

## Implementation Tasks

1. Define agent profile schema.
2. Implement task assignment.
3. Implement sub-agent runner.
4. Integrate worktree leases.
5. Implement review workflow.
6. Implement team board.
7. Implement message bus.
8. Implement merge coordination.
9. Implement conflict resolver.
10. Implement sub-agent evals.

## Tests

- Sub-agent receives scoped context.
- Worktree isolation prevents conflicts.
- Reviewer rejects failing diff.
- Coordinator merges accepted diff.
- Sub-agent failure writes handoff.

