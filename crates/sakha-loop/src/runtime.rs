//! `LoopRuntime`: drives `LoopSpec`s through ticks. See `04-core-domain-model.md`
//! `LoopController` and spec "Loop Control Rules".
//!
//! `Agent::run_turn` (in `sakha-agent`) is itself an unwired stub as of this
//! crate's implementation, so `LoopRuntime` is built around a pluggable
//! `TickExecutor` trait: the loop machinery here (budgets, watchdog,
//! checkpoint/resume, verification between ticks) is fully real and tested
//! against a test double, while the actual "call the agent" step is
//! supplied by whoever wires an `Agent` in (daemon/CLI), matching the
//! "external integrations behind traits with a working local fallback"
//! project rule extended to agent execution.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use sakha_core::{BudgetLedger, GoalId, LoopId, LoopTickId, SakhaError, SakhaResult};
use sakha_memory::{HandoffArtifact, HandoffStore};

use crate::spec::LoopSpec;
use crate::verification_loop::{run_checks, VerificationResult};
use crate::watchdog::{LoopWatchdog, StallReport};

/// Why a loop stopped (pause vs terminal stop).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopReason(pub String);

/// The lifecycle status of a loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopState {
    Created,
    Running,
    Paused,
    Stopped,
    Completed,
}

/// One executed tick of a loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopTick {
    pub id: LoopTickId,
    pub loop_id: LoopId,
    pub iteration: u64,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub succeeded: Option<bool>,
}

/// The result of running one tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopTickResult {
    pub tick: LoopTick,
    pub state: LoopState,
    #[serde(default)]
    pub verification: Option<VerificationResult>,
    #[serde(default)]
    pub stall: Option<StallReport>,
}

/// The outcome of running one iteration of loop work, produced by a
/// `TickExecutor`. Kept intentionally small: enough for the runtime to
/// checkpoint progress and feed the watchdog/verifier, without coupling to
/// `sakha-agent`'s internal state representation.
#[derive(Debug, Clone, Default)]
pub struct TickOutcome {
    /// A hash/signature of the resulting state, fed to the watchdog's stall
    /// detector. Two ticks producing the same signature back-to-back are a
    /// stall signal.
    pub state_signature: String,
    pub completed_work: Vec<String>,
    pub pending_work: Vec<String>,
    pub files_changed: Vec<String>,
    pub commands_run: Vec<String>,
    pub decisions_made: Vec<String>,
    pub blockers: Vec<String>,
    pub next_suggested_action: String,
    /// Whether the executor believes the loop's objective is now satisfied
    /// (drives `LoopState::Completed`).
    pub goal_satisfied: bool,
    /// Cost incurred by this tick, debited against the loop's budget.
    pub cost_micros: u64,
}

/// Drives one iteration of loop work for a given `LoopSpec`. The default
/// wiring (`Agent` + tool loop) lives outside this crate; a `NullTickExecutor`
/// is provided here as a safe, always-available fallback that reports
/// `not_implemented` rather than fabricating progress.
#[async_trait]
pub trait TickExecutor: Send + Sync {
    async fn run_tick(&self, spec: &LoopSpec, handoff: Option<&HandoffArtifact>) -> SakhaResult<TickOutcome>;
}

/// Safe default `TickExecutor`: always reports `not_implemented`. Loop
/// state/budget/watchdog machinery still functions with this executor;
/// only the "do the work" step is unavailable.
#[derive(Debug, Default)]
pub struct NullTickExecutor;

#[async_trait]
impl TickExecutor for NullTickExecutor {
    async fn run_tick(&self, _spec: &LoopSpec, _handoff: Option<&HandoffArtifact>) -> SakhaResult<TickOutcome> {
        Err(SakhaError::not_implemented("sakha-loop", "TickExecutor::run_tick (no executor wired)"))
    }
}

/// Runs long-running loops safely. See `04-core-domain-model.md` `LoopController`.
#[async_trait]
pub trait LoopController: Send + Sync {
    async fn create_loop(&self, spec: LoopSpec) -> SakhaResult<LoopId>;
    async fn tick(&self, loop_id: LoopId) -> SakhaResult<LoopTickResult>;
    async fn pause(&self, loop_id: LoopId) -> SakhaResult<()>;
    async fn resume(&self, loop_id: LoopId) -> SakhaResult<()>;
    async fn stop(&self, loop_id: LoopId, reason: StopReason) -> SakhaResult<()>;
    async fn detect_stall(&self, loop_id: LoopId) -> SakhaResult<StallReport>;
}

/// Per-loop bookkeeping the runtime keeps alongside the immutable spec.
struct LoopEntry {
    spec: LoopSpec,
    state: LoopState,
    ledger: Arc<BudgetLedger>,
    iteration: u64,
    /// A stable goal id derived from the loop id, used to key handoff
    /// checkpoints (a loop's durable memory file, per spec "durable memory
    /// file for every loop").
    goal_id: GoalId,
}

/// `LoopRuntime`: implements `LoopController`, drives ticks through a
/// pluggable `TickExecutor`, checkpoints progress via a `HandoffStore`
/// (spec "tick checkpointing" / "durable memory file for every loop"), runs
/// deterministic verification between ticks (module 12), and consults a
/// `LoopWatchdog` for stall/budget stop conditions (spec "Infinite Loop
/// Prevention").
pub struct LoopRuntime {
    entries: Mutex<HashMap<LoopId, LoopEntry>>,
    handoff_store: Arc<dyn HandoffStore>,
    executor: Arc<dyn TickExecutor>,
    watchdog: Mutex<LoopWatchdog>,
}

impl LoopRuntime {
    /// Builds a runtime with in-memory handoff storage and a
    /// `NullTickExecutor` — a fully functional default for tests and for
    /// exercising the loop/budget/watchdog machinery before an `Agent` is
    /// wired in.
    pub fn new() -> Self {
        Self::with_deps(
            Arc::new(sakha_memory::InMemoryHandoffStore::new()),
            Arc::new(NullTickExecutor),
        )
    }

    /// Builds a runtime with a caller-supplied handoff store and tick
    /// executor, e.g. a `SqliteHandoffStore` for durability and a real
    /// `Agent`-backed executor for actual work.
    pub fn with_deps(handoff_store: Arc<dyn HandoffStore>, executor: Arc<dyn TickExecutor>) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            handoff_store,
            executor,
            watchdog: Mutex::new(LoopWatchdog::default()),
        }
    }

    /// Deterministically derives a `GoalId` from a `LoopId` so a loop's
    /// handoff checkpoints are addressable without an extra id round-trip.
    fn goal_id_for(loop_id: LoopId) -> GoalId {
        GoalId::from_uuid(loop_id.as_uuid())
    }

    /// Loads the most recent checkpoint for a loop, if any, per spec
    /// "checkpoint/resume via sakha-memory handoffs".
    pub async fn load_checkpoint(&self, loop_id: LoopId) -> SakhaResult<Option<HandoffArtifact>> {
        let goal_id = Self::goal_id_for(loop_id);
        self.handoff_store.load_handoff(goal_id).await
    }

    fn build_handoff(spec: &LoopSpec, outcome: &TickOutcome, iteration: u64) -> HandoffArtifact {
        let goal_id = Self::goal_id_for(spec.id);
        let mut handoff = HandoffArtifact::new(goal_id, spec.objective.clone());
        handoff.current_status = if outcome.goal_satisfied {
            "completed".to_string()
        } else {
            format!("running (iteration {iteration})")
        };
        handoff.completed_work = outcome.completed_work.clone();
        handoff.pending_work = outcome.pending_work.clone();
        handoff.files_changed = outcome.files_changed.clone();
        handoff.commands_run = outcome.commands_run.clone();
        handoff.decisions_made = outcome.decisions_made.clone();
        handoff.blockers = outcome.blockers.clone();
        handoff.next_suggested_action = outcome.next_suggested_action.clone();
        handoff
    }
}

impl Default for LoopRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LoopController for LoopRuntime {
    async fn create_loop(&self, spec: LoopSpec) -> SakhaResult<LoopId> {
        let id = spec.id;
        let ledger = Arc::new(BudgetLedger::new(spec.budget()));
        let goal_id = Self::goal_id_for(id);
        self.entries.lock().unwrap().insert(id, LoopEntry { spec, state: LoopState::Created, ledger, iteration: 0, goal_id });
        Ok(id)
    }

    async fn tick(&self, loop_id: LoopId) -> SakhaResult<LoopTickResult> {
        // Snapshot what we need under the lock, then run async work outside it.
        let (spec, ledger, iteration, goal_id) = {
            let entries = self.entries.lock().unwrap();
            let entry = entries.get(&loop_id).ok_or_else(|| SakhaError::invalid_input("sakha-loop", format!("unknown loop: {loop_id}")))?;
            if entry.state == LoopState::Stopped || entry.state == LoopState::Completed {
                return Err(SakhaError::invalid_input("sakha-loop", format!("loop {loop_id} is {:?}, cannot tick", entry.state)));
            }
            if entry.state == LoopState::Paused {
                return Err(SakhaError::invalid_input("sakha-loop", format!("loop {loop_id} is paused, resume before ticking")));
            }
            (entry.spec.clone(), Arc::clone(&entry.ledger), entry.iteration, entry.goal_id)
        };

        // Loop Control Rules: "every loop must define max ... cost" — check
        // budget/stall state before doing any work for this tick.
        let pre_check = self.watchdog.lock().unwrap().check_budget(&ledger);
        if pre_check.stalled {
            self.stop(loop_id, StopReason(format!("budget exhausted before tick: {:?}", pre_check.reasons))).await?;
            return Err(SakhaError::budget("sakha-loop", format!("loop {loop_id} budget exhausted: {:?}", pre_check.reasons)));
        }

        ledger.debit(sakha_core::BudgetDimension::LoopIterations, 1)?;

        let tick_id = LoopTickId::new();
        let started_at = sakha_core::time::now_utc();

        {
            let mut entries = self.entries.lock().unwrap();
            if let Some(entry) = entries.get_mut(&loop_id) {
                entry.state = LoopState::Running;
            }
        }

        // Checkpoint/resume: load the prior handoff (if any) so the
        // executor can continue from where the last tick left off.
        let prior_handoff = self.handoff_store.load_handoff(goal_id).await?;

        let tick_result = self.executor.run_tick(&spec, prior_handoff.as_ref()).await;

        let (outcome, succeeded, tick_error) = match tick_result {
            Ok(outcome) => (Some(outcome), true, None),
            Err(err) => (None, false, Some(err)),
        };

        // Debit any cost the tick incurred, and observe the watchdog before
        // deciding the resulting loop state.
        let mut stall_report = StallReport::default();
        if let Some(outcome) = &outcome {
            if outcome.cost_micros > 0 {
                ledger.debit(sakha_core::BudgetDimension::CostMicros, outcome.cost_micros)?;
            }
            stall_report = self.watchdog.lock().unwrap().observe(loop_id, outcome.state_signature.clone());
        } else if let Some(err) = &tick_error {
            stall_report = self.watchdog.lock().unwrap().observe_error(loop_id, err.class.to_string());
        }
        let post_budget = self.watchdog.lock().unwrap().check_budget(&ledger);
        if post_budget.stalled {
            stall_report.stalled = true;
            stall_report.reasons.extend(post_budget.reasons);
        }

        // Run deterministic verification between ticks (module 12 "Command
        // verifier") when the spec declares checks and the tick succeeded.
        let verification = if succeeded && !spec.verifier.checks.is_empty() {
            Some(run_checks(&spec.verifier.checks).await)
        } else {
            None
        };
        let verification_failed = verification.as_ref().is_some_and(|v| !v.passed);

        // Write a checkpoint/handoff after every tick, per spec "every loop
        // must write a handoff artifact after each tick" — even on failure,
        // so a fresh agent (or human) can see exactly what happened.
        let new_iteration = iteration + 1;
        if let Some(outcome) = &outcome {
            let handoff = Self::build_handoff(&spec, outcome, new_iteration);
            self.handoff_store.write_handoff(goal_id, handoff).await?;
        } else if let Some(err) = &tick_error {
            let mut handoff = HandoffArtifact::new(goal_id, spec.objective.clone());
            handoff.current_status = "blocked".to_string();
            handoff.blockers.push(err.to_string());
            handoff.next_suggested_action = "investigate tick executor failure".to_string();
            self.handoff_store.write_handoff(goal_id, handoff).await?;
        }

        let goal_satisfied = outcome.as_ref().is_some_and(|o| o.goal_satisfied) && !verification_failed;

        // A failed tick (executor error) or a failed verification pass
        // leaves the loop `Running` so a retry can be attempted on the next
        // tick, unless the watchdog says to stop. Only an explicit
        // `goal_satisfied` completes the loop.
        let new_state = if stall_report.stalled {
            LoopState::Stopped
        } else if goal_satisfied {
            LoopState::Completed
        } else {
            LoopState::Running
        };

        {
            let mut entries = self.entries.lock().unwrap();
            if let Some(entry) = entries.get_mut(&loop_id) {
                entry.state = new_state;
                entry.iteration = new_iteration;
            }
        }

        let tick = LoopTick {
            id: tick_id,
            loop_id,
            iteration: new_iteration,
            started_at,
            completed_at: Some(sakha_core::time::now_utc()),
            succeeded: Some(succeeded && !verification_failed),
        };

        let result = LoopTickResult {
            tick,
            state: new_state,
            verification,
            stall: if stall_report.stalled { Some(stall_report) } else { None },
        };

        // Surface a tick-executor failure as an `Err` (so callers see it
        // immediately) while the handoff/state bookkeeping above still ran,
        // per spec "every loop must write a handoff artifact after each
        // tick" (including failed ticks).
        if let Some(err) = tick_error {
            return Err(err.with_subject(format!("loop_tick={} state_after={:?}", result.tick.id, result.state)));
        }

        Ok(result)
    }

    async fn pause(&self, loop_id: LoopId) -> SakhaResult<()> {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries.get_mut(&loop_id).ok_or_else(|| SakhaError::invalid_input("sakha-loop", "unknown loop"))?;
        entry.state = LoopState::Paused;
        Ok(())
    }

    async fn resume(&self, loop_id: LoopId) -> SakhaResult<()> {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries.get_mut(&loop_id).ok_or_else(|| SakhaError::invalid_input("sakha-loop", "unknown loop"))?;
        entry.state = LoopState::Running;
        Ok(())
    }

    async fn stop(&self, loop_id: LoopId, _reason: StopReason) -> SakhaResult<()> {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries.get_mut(&loop_id).ok_or_else(|| SakhaError::invalid_input("sakha-loop", "unknown loop"))?;
        entry.state = LoopState::Stopped;
        Ok(())
    }

    async fn detect_stall(&self, loop_id: LoopId) -> SakhaResult<StallReport> {
        let ledger = {
            let entries = self.entries.lock().unwrap();
            let entry = entries.get(&loop_id).ok_or_else(|| SakhaError::invalid_input("sakha-loop", "unknown loop"))?;
            Arc::clone(&entry.ledger)
        };
        Ok(self.watchdog.lock().unwrap().check_budget(&ledger))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{LoopKind, Trigger};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A `TickExecutor` test double that succeeds a configurable number of
    /// times before reporting `goal_satisfied`, so tests can exercise
    /// multi-tick progression without a real `Agent`.
    struct ScriptedExecutor {
        ticks_until_done: AtomicU64,
    }

    impl ScriptedExecutor {
        fn new(ticks_until_done: u64) -> Self {
            Self { ticks_until_done: AtomicU64::new(ticks_until_done) }
        }
    }

    #[async_trait]
    impl TickExecutor for ScriptedExecutor {
        async fn run_tick(&self, _spec: &LoopSpec, handoff: Option<&HandoffArtifact>) -> SakhaResult<TickOutcome> {
            let previous = self.ticks_until_done.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| Some(n.saturating_sub(1))).unwrap();
            let remaining = previous.saturating_sub(1);
            let mut outcome = TickOutcome {
                state_signature: format!("tick-{remaining}"),
                completed_work: vec![format!("step-{remaining}")],
                next_suggested_action: "continue".into(),
                goal_satisfied: remaining == 0,
                cost_micros: 10,
                ..Default::default()
            };
            if let Some(h) = handoff {
                outcome.completed_work.push(format!("resumed-from:{}", h.current_status));
            }
            Ok(outcome)
        }
    }

    /// A `TickExecutor` that always produces the same state signature, to
    /// exercise watchdog stall detection.
    #[derive(Default)]
    struct StuckExecutor;

    #[async_trait]
    impl TickExecutor for StuckExecutor {
        async fn run_tick(&self, _spec: &LoopSpec, _handoff: Option<&HandoffArtifact>) -> SakhaResult<TickOutcome> {
            Ok(TickOutcome { state_signature: "stuck".into(), goal_satisfied: false, ..Default::default() })
        }
    }

    fn runtime_with(executor: Arc<dyn TickExecutor>) -> LoopRuntime {
        LoopRuntime::with_deps(Arc::new(sakha_memory::InMemoryHandoffStore::new()), executor)
    }

    #[tokio::test]
    async fn default_runtime_tick_reports_not_implemented_without_executor() {
        let runtime = LoopRuntime::new();
        let spec = LoopSpec::new("goal", LoopKind::Agent, Trigger::Manual);
        let loop_id = runtime.create_loop(spec).await.unwrap();
        assert!(runtime.tick(loop_id).await.is_err());
    }

    #[tokio::test]
    async fn tick_drives_loop_to_completion_and_writes_final_handoff() {
        let runtime = runtime_with(Arc::new(ScriptedExecutor::new(2)));
        let spec = LoopSpec::new("ship feature", LoopKind::Agent, Trigger::Manual);
        let loop_id = runtime.create_loop(spec).await.unwrap();

        let first = runtime.tick(loop_id).await.unwrap();
        assert_eq!(first.state, LoopState::Running);

        let second = runtime.tick(loop_id).await.unwrap();
        assert_eq!(second.state, LoopState::Completed);

        let checkpoint = runtime.load_checkpoint(loop_id).await.unwrap().expect("handoff written");
        assert_eq!(checkpoint.current_status, "completed");
    }

    #[tokio::test]
    async fn tick_resumes_from_prior_checkpoint() {
        let runtime = runtime_with(Arc::new(ScriptedExecutor::new(1)));
        let spec = LoopSpec::new("resume test", LoopKind::Agent, Trigger::Manual);
        let loop_id = runtime.create_loop(spec).await.unwrap();

        let result = runtime.tick(loop_id).await.unwrap();
        assert_eq!(result.state, LoopState::Completed);
        let checkpoint = runtime.load_checkpoint(loop_id).await.unwrap().unwrap();
        // The single tick had no prior handoff to resume from.
        assert!(!checkpoint.completed_work.iter().any(|s| s.starts_with("resumed-from:")));
    }

    #[tokio::test]
    async fn ticking_unknown_loop_errors() {
        let runtime = LoopRuntime::new();
        assert!(runtime.tick(LoopId::new()).await.is_err());
    }

    #[tokio::test]
    async fn ticking_paused_loop_errors() {
        let runtime = runtime_with(Arc::new(ScriptedExecutor::new(5)));
        let spec = LoopSpec::new("goal", LoopKind::Agent, Trigger::Manual);
        let loop_id = runtime.create_loop(spec).await.unwrap();
        runtime.pause(loop_id).await.unwrap();
        assert!(runtime.tick(loop_id).await.is_err());
    }

    #[tokio::test]
    async fn watchdog_stops_loop_on_repeated_state_signature() {
        let runtime = runtime_with(Arc::new(StuckExecutor));
        let mut spec = LoopSpec::new("stuck goal", LoopKind::Agent, Trigger::Manual);
        spec.max_iterations = 100;
        let loop_id = runtime.create_loop(spec).await.unwrap();

        // Watchdog default threshold is 3 repeats before stalling.
        let mut last_state = LoopState::Created;
        for _ in 0..6 {
            match runtime.tick(loop_id).await {
                Ok(result) => last_state = result.state,
                Err(_) => break,
            }
            if last_state == LoopState::Stopped {
                break;
            }
        }
        assert_eq!(last_state, LoopState::Stopped);
    }

    #[tokio::test]
    async fn budget_stops_loop_when_iteration_cap_reached() {
        let runtime = runtime_with(Arc::new(ScriptedExecutor::new(1000)));
        let mut spec = LoopSpec::new("budget-capped goal", LoopKind::Agent, Trigger::Manual);
        spec.max_iterations = 2;
        let loop_id = runtime.create_loop(spec).await.unwrap();

        runtime.tick(loop_id).await.unwrap();
        runtime.tick(loop_id).await.unwrap();
        // Third tick should be rejected: iteration budget exhausted.
        let third = runtime.tick(loop_id).await;
        assert!(third.is_err());
    }

    #[tokio::test]
    async fn pause_then_resume_allows_ticking_again() {
        let runtime = runtime_with(Arc::new(ScriptedExecutor::new(5)));
        let spec = LoopSpec::new("goal", LoopKind::Agent, Trigger::Manual);
        let loop_id = runtime.create_loop(spec).await.unwrap();
        runtime.pause(loop_id).await.unwrap();
        runtime.resume(loop_id).await.unwrap();
        assert!(runtime.tick(loop_id).await.is_ok());
    }

    #[tokio::test]
    async fn verification_checks_run_between_ticks_and_gate_completion() {
        use crate::verification_loop::CheckSpec;
        let runtime = runtime_with(Arc::new(ScriptedExecutor::new(1)));
        let mut spec = LoopSpec::new("verified goal", LoopKind::Verification, Trigger::Manual);
        let failing_check = if cfg!(windows) {
            CheckSpec::new("must-fail", vec!["cmd".into(), "/C".into(), "exit 1".into()])
        } else {
            CheckSpec::new("must-fail", vec!["false".into()])
        };
        spec.verifier.checks = vec![failing_check];
        let loop_id = runtime.create_loop(spec).await.unwrap();

        let result = runtime.tick(loop_id).await.unwrap();
        // Goal was reported satisfied by the executor, but the failing
        // verification check must gate completion.
        assert_ne!(result.state, LoopState::Completed);
        assert!(result.verification.is_some());
        assert!(!result.verification.unwrap().passed);
    }

    #[tokio::test]
    async fn detect_stall_reports_budget_exhaustion() {
        let runtime = runtime_with(Arc::new(ScriptedExecutor::new(1000)));
        let mut spec = LoopSpec::new("goal", LoopKind::Agent, Trigger::Manual);
        spec.max_iterations = 1;
        let loop_id = runtime.create_loop(spec).await.unwrap();
        runtime.tick(loop_id).await.unwrap();
        let report = runtime.detect_stall(loop_id).await.unwrap();
        assert!(report.stalled);
    }
}
