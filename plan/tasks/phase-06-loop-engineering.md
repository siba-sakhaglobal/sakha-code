# Phase 06: Loop Engineering

## Goal

Implement long-running autonomous work loops, including web search loops.

## Tasks

### 06.1 Goal Mode

- Create goal.
- Persist plan.
- Tick goal until done/blocked/budgeted.
- Resume after restart.

### 06.2 Scheduler

- Cron trigger.
- Interval trigger.
- Manual trigger.
- Queue leases.

### 06.3 Event Triggers

- GitHub/CI webhook adapter stub.
- Filesystem watcher.
- Web research monitor.

### 06.4 Web Research Loop

- Query planning.
- Search.
- Fetch pages.
- Score sources.
- Compress pages.
- Build evidence pack.
- Verify claims.

### 06.5 Verification Loop

- Run tests/lint/build.
- Feed failures back to agent.
- Retry within budget.
- Stop after repeated failure.

### 06.6 Loop Watchdog

- Detect repeated action.
- Detect context growth.
- Detect side-effect repetition.
- Detect no progress.
- Block with explanation.

### 06.7 Replay Skills

- Record successful workflow.
- Extract parameters.
- Dry-run replay.
- Promote to loop skill.

## Definition of Done

- Scheduled goal runs.
- Web research loop produces evidence pack.
- Verification loop fixes a failing test or blocks.
- Watchdog catches infinite loop fixture.
- Replay skill reduces model calls for repeated task.

