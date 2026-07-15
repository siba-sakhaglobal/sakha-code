# Module 08: Files, Git, and Worktrees

## Responsibility

Enable safe codebase edits, diffing, version control, and isolated parallel work.

## Rust Crate

`crates/sakha-tools` plus `crates/sakha-vcs`

## Main Structs

- `WorkspaceFs`
- `PathPolicy`
- `FileSnapshot`
- `Patch`
- `PatchHunk`
- `DiffSummary`
- `GitRepository`
- `WorktreeManager`
- `WorktreeLease`
- `BranchPlan`
- `CommitPlan`

## Features

- Workspace root detection.
- Path traversal prevention.
- Read file with token-aware slicing.
- Write file with backup snapshot.
- Apply patch with conflict detection.
- Diff summarization.
- Git status/diff/log.
- Create branch.
- Create worktree per agent/loop.
- Cleanup worktrees.
- Commit with generated message.
- PR preparation metadata.

## Worktree Rules

- Long-running loop gets dedicated worktree.
- Sub-agent gets dedicated worktree if write access is enabled.
- Worktree lease records owner, goal, created_at, last_activity.
- Worktree cleanup requires no uncommitted changes or explicit force.

## Patch Safety

- Patch must match expected context.
- Patch engine stores before/after snapshots.
- Binary edits require separate flow.
- Generated files can be excluded by policy.
- Writes outside workspace require explicit policy grant.

## Implementation Tasks

1. Implement `WorkspaceFs`.
2. Implement path policy.
3. Implement file snapshot storage.
4. Implement patch parser/applier.
5. Implement diff summary.
6. Implement git wrapper.
7. Implement worktree manager.
8. Implement branch/commit planner.
9. Implement conflict reporting.
10. Implement cleanup job.

## Tests

- Path traversal blocked.
- Patch conflict detected.
- Snapshot restore works.
- Worktree lease acquired/released.
- Parallel worktrees do not collide.
- Git dirty state detected before destructive action.

