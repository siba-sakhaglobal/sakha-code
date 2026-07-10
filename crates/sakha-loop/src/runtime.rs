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

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use sakha_core::{Budget, BudgetDimension, BudgetLedger, GoalId, LoopId, LoopTickId, SakhaError, SakhaResult};
use sakha_memory::{HandoffArtifact, HandoffStore};

use crate::spec::{HandoffPolicy, LoopSpec};
use crate::verification_loop::{run_checks, VerificationResult};
use crate::watchdog::{LoopWatchdog, StallReport};

/// Per-loop budget wrapper named per `04-core-domain-model.md` / module 04
/// "Main Structs" -> `LoopBudget`. Thin, purpose-specific wrapper around
/// `sakha_core::{Budget, BudgetLedger}` (rather than a parallel
/// reimplementation) that also owns the loop's wall-clock start time, since
/// wall-clock enforcement needs a stable "loop created at" instant that
/// outlives any single tick.
pub struct LoopBudget {
    ledger: Arc<BudgetLedger>,
    /// When this loop's budget clock started; used to compute elapsed
    /// wall-clock time to debit against `BudgetDimension::WallTime` on every
    /// tick, per spec "every loop must define max wall-clock time".
    started_at: Instant,
}

impl LoopBudget {
    pub fn new(limits: Budget) -> Self {
        Self { ledger: Arc::new(BudgetLedger::new(limits)), started_at: Instant::now() }
    }

    pub fn ledger(&self) -> &Arc<BudgetLedger> {
        &self.ledger
    }

    /// Milliseconds elapsed since this loop's budget clock started.
    pub fn elapsed_millis(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    /// Debits the ledger's `WallTime` dimension up to the elapsed wall-clock
    /// time since loop creation, so `LoopWatchdog::check_budget` can enforce
    /// `max_wall_time_secs` across multiple ticks (not just within one).
    /// Only debits the delta since the last recorded usage so repeated calls
    /// within the same tick don't double-count.
    pub fn debit_elapsed_wall_time(&self) -> SakhaResult<()> {
        let elapsed = self.elapsed_millis();
        let already_debited = self.ledger.used(BudgetDimension::WallTime);
        let delta = elapsed.saturating_sub(already_debited);
        if delta > 0 {
            self.ledger.debit(BudgetDimension::WallTime, delta)?;
        }
        Ok(())
    }
}

/// One record of feedback (user/CI/runtime result) that should influence a
/// loop's memory or future policy, per module 04 "Loop Layers" -> "Feedback
/// loop: user/CI/runtime results update memory and policies" and
/// `04-core-domain-model.md` "Main Structs" -> `FeedbackRecord`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackRecord {
    pub loop_id: LoopId,
    pub tick_id: Option<LoopTickId>,
    pub source: FeedbackSource,
    /// Free-text feedback content (a CI failure message, a user correction,
    /// a runtime error), stored verbatim so it can be folded into the next
    /// tick's prompt/handoff without re-deriving it.
    pub content: String,
    /// Whether this feedback indicates the prior tick's work was accepted
    /// (`true`) or needs revision (`false`).
    pub accepted: bool,
    pub recorded_at: chrono::DateTime<chrono::Utc>,
}

/// Where a `FeedbackRecord` originated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackSource {
    User,
    Ci,
    Runtime,
    Verifier,
}

impl FeedbackRecord {
    pub fn new(loop_id: LoopId, source: FeedbackSource, content: impl Into<String>, accepted: bool) -> Self {
        Self {
            loop_id,
            tick_id: None,
            source,
            content: content.into(),
            accepted,
            recorded_at: sakha_core::time::now_utc(),
        }
    }

    pub fn with_tick_id(mut self, tick_id: LoopTickId) -> Self {
        self.tick_id = Some(tick_id);
        self
    }
}

/// A pending or resolved human approval for a loop tick's side effects, per
/// spec "Required Capabilities" -> "Human approval gates" and
/// `HandoffPolicy::RequireHumanApproval`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
}

/// Stores and resolves human approval requests for loop ticks. Ticking a
/// loop whose `HandoffPolicy` is `RequireHumanApproval` consults this store
/// before applying side effects; `DryRunOnly`/`AutoApply` loops never need
/// one. See spec "Human approval gates".
#[async_trait]
pub trait ApprovalStore: Send + Sync {
    /// Records that iteration `iteration` of `loop_id` is waiting on human
    /// approval, and returns the current status (an already-resolved status
    /// from a prior call is returned as-is rather than being reset to
    /// `Pending`).
    async fn request(&self, loop_id: LoopId, iteration: u64) -> SakhaResult<ApprovalStatus>;
    /// Resolves a pending approval. Callable by a human/operator (CLI,
    /// daemon API) independently of the loop tick that requested it.
    async fn resolve(&self, loop_id: LoopId, iteration: u64, approved: bool) -> SakhaResult<()>;
    /// Current status for `loop_id`'s `iteration`, if a request exists.
    async fn status(&self, loop_id: LoopId, iteration: u64) -> SakhaResult<Option<ApprovalStatus>>;
}

/// In-memory `ApprovalStore`: a safe default that starts every request as
/// `Pending` until a caller (human/operator) explicitly resolves it. Never
/// auto-approves, so `RequireHumanApproval` loops are genuinely gated even
/// with no real approval UI wired in yet.
#[derive(Debug, Default)]
pub struct InMemoryApprovalStore {
    decisions: Mutex<HashMap<(LoopId, u64), ApprovalStatus>>,
}

impl InMemoryApprovalStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ApprovalStore for InMemoryApprovalStore {
    async fn request(&self, loop_id: LoopId, iteration: u64) -> SakhaResult<ApprovalStatus> {
        let mut decisions = self.decisions.lock().unwrap_or_else(|p| p.into_inner());
        Ok(*decisions.entry((loop_id, iteration)).or_insert(ApprovalStatus::Pending))
    }

    async fn resolve(&self, loop_id: LoopId, iteration: u64, approved: bool) -> SakhaResult<()> {
        let mut decisions = self.decisions.lock().unwrap_or_else(|p| p.into_inner());
        decisions.insert((loop_id, iteration), if approved { ApprovalStatus::Approved } else { ApprovalStatus::Rejected });
        Ok(())
    }

    async fn status(&self, loop_id: LoopId, iteration: u64) -> SakhaResult<Option<ApprovalStatus>> {
        let decisions = self.decisions.lock().unwrap_or_else(|p| p.into_inner());
        Ok(decisions.get(&(loop_id, iteration)).copied())
    }
}

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
    /// Stable signatures of external side effects this tick performed or is
    /// about to perform (e.g. `"shell:git push origin main"`,
    /// `"http:POST https://api.example.com/issues"`), used by the runtime's
    /// side-effect dedup (spec "Infinite Loop Prevention" -> "External
    /// side-effect deduplication") to detect a same-effect re-execution
    /// across ticks for non-idempotent loops.
    pub side_effect_signatures: Vec<String>,
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
    /// Lists every loop this controller currently knows about (spec "CLI
    /// Commands": `sakha loop list`). For an in-process `LoopRuntime` this is
    /// every loop created via `create_loop` since the runtime was
    /// constructed; a daemon holding one long-lived `LoopRuntime` therefore
    /// sees every loop it manages, while a fresh per-invocation runtime (as
    /// today's CLI wiring uses) legitimately sees none yet.
    async fn list(&self) -> SakhaResult<Vec<LoopSummary>>;
}

/// A lightweight, serializable view of one loop's current state — enough for
/// `sakha loop list` without exposing `LoopRuntime`'s internal `LoopEntry`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopSummary {
    pub id: LoopId,
    pub objective: String,
    pub kind: crate::spec::LoopKind,
    pub state: LoopState,
    pub iteration: u64,
}

/// Per-loop bookkeeping the runtime keeps alongside the immutable spec.
struct LoopEntry {
    spec: LoopSpec,
    state: LoopState,
    budget: Arc<LoopBudget>,
    iteration: u64,
    /// A stable goal id derived from the loop id, used to key handoff
    /// checkpoints (a loop's durable memory file, per spec "durable memory
    /// file for every loop").
    goal_id: GoalId,
}

/// A loop's paused/running status plus iteration count, persisted through
/// the `HandoffStore` (via `HandoffArtifact::current_status`/`pending_work`)
/// so that a paused loop survives a process restart. Spec "every loop must
/// be pausable" combined with "durable memory file for every loop": pause is
/// only a real pause if the runtime can be restarted and still know the loop
/// was paused, rather than silently resuming (or losing the loop) because
/// state only lived in an in-memory map.
const PAUSED_STATUS_MARKER: &str = "__sakha_loop_paused__";

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
    approvals: Arc<dyn ApprovalStore>,
    /// External side-effect signatures already observed, across all loops,
    /// keyed by loop id. Spec "Infinite Loop Prevention" -> "External
    /// side-effect deduplication": a non-idempotent loop that reports the
    /// same side-effect signature on a later tick (e.g. a retried tick
    /// re-running `shell:git push origin main`) has that effect flagged
    /// rather than silently re-applied.
    side_effects_seen: Mutex<HashMap<LoopId, HashSet<String>>>,
    /// Feedback recorded for loops (spec "Feedback loop"), most recent last.
    feedback: Mutex<Vec<FeedbackRecord>>,
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
    /// `Agent`-backed executor for actual work. Uses an `InMemoryApprovalStore`
    /// for human approval gates; use `with_approval_store` to supply a
    /// durable one.
    pub fn with_deps(handoff_store: Arc<dyn HandoffStore>, executor: Arc<dyn TickExecutor>) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            handoff_store,
            executor,
            watchdog: Mutex::new(LoopWatchdog::default()),
            approvals: Arc::new(InMemoryApprovalStore::new()),
            side_effects_seen: Mutex::new(HashMap::new()),
            feedback: Mutex::new(Vec::new()),
        }
    }

    /// Overrides the approval store used for `HandoffPolicy::RequireHumanApproval`
    /// gating, e.g. to back it with durable storage.
    pub fn with_approval_store(mut self, approvals: Arc<dyn ApprovalStore>) -> Self {
        self.approvals = approvals;
        self
    }

    /// Records a `FeedbackRecord`, per spec "Feedback loop": user/CI/runtime
    /// results update memory and policies. Feedback is appended to an
    /// in-process log the next tick's executor/handoff-builder can consult;
    /// durable persistence of feedback (beyond a process lifetime) is the
    /// caller's responsibility via `sakha-memory`, matching how `TickExecutor`
    /// wiring is left to the caller.
    pub fn record_feedback(&self, feedback: FeedbackRecord) {
        self.feedback.lock().unwrap_or_else(|p| p.into_inner()).push(feedback);
    }

    /// Returns all feedback recorded for `loop_id`, oldest first.
    pub fn feedback_for(&self, loop_id: LoopId) -> Vec<FeedbackRecord> {
        self.feedback
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|f| f.loop_id == loop_id)
            .cloned()
            .collect()
    }

    /// Approval store accessor, so callers (CLI/daemon) can resolve pending
    /// human approval gates for `RequireHumanApproval` loops.
    pub fn approvals(&self) -> &Arc<dyn ApprovalStore> {
        &self.approvals
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

    /// Persists a loop's paused/running status durably via its handoff
    /// checkpoint, so `pause()` survives a process restart (spec "every loop
    /// must be pausable" + "durable memory file for every loop"). Loads the
    /// existing handoff (if any) and rewrites only the status marker,
    /// preserving everything else the last tick recorded.
    async fn persist_pause_state(&self, goal_id: GoalId, objective: &str, paused: bool) -> SakhaResult<()> {
        let mut handoff = self
            .handoff_store
            .load_handoff(goal_id)
            .await?
            .unwrap_or_else(|| HandoffArtifact::new(goal_id, objective.to_string()));
        if paused {
            if !handoff.current_status.starts_with(PAUSED_STATUS_MARKER) {
                handoff.current_status = format!("{PAUSED_STATUS_MARKER}{}", handoff.current_status);
            }
        } else if let Some(stripped) = handoff.current_status.strip_prefix(PAUSED_STATUS_MARKER) {
            handoff.current_status = stripped.to_string();
        }
        self.handoff_store.write_handoff(goal_id, handoff).await
    }

    /// Restores in-memory `LoopState` for a loop from its durable handoff
    /// checkpoint, per spec "every loop must be pausable": called on
    /// `create_loop` (recreation after a restart uses the same `LoopId`, so
    /// a prior paused checkpoint is honored) to recover paused state without
    /// requiring a separate persistence path from the handoff artifact.
    async fn recover_state_from_checkpoint(&self, goal_id: GoalId) -> SakhaResult<LoopState> {
        let handoff = self.handoff_store.load_handoff(goal_id).await?;
        Ok(match handoff {
            Some(h) if h.current_status.starts_with(PAUSED_STATUS_MARKER) => LoopState::Paused,
            _ => LoopState::Created,
        })
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
        let budget = Arc::new(LoopBudget::new(spec.budget()));
        let goal_id = Self::goal_id_for(id);
        // Recover paused state from a durable checkpoint, per spec "every
        // loop must be pausable": re-creating a `LoopSpec` with the same id
        // (e.g. after a process restart) must not silently drop a pause.
        let state = self.recover_state_from_checkpoint(goal_id).await?;
        self.entries.lock().unwrap().insert(id, LoopEntry { spec, state, budget, iteration: 0, goal_id });
        Ok(id)
    }

    async fn tick(&self, loop_id: LoopId) -> SakhaResult<LoopTickResult> {
        // Snapshot what we need under the lock, then run async work outside it.
        let (spec, budget, iteration, goal_id) = {
            let entries = self.entries.lock().unwrap();
            let entry = entries.get(&loop_id).ok_or_else(|| SakhaError::invalid_input("sakha-loop", format!("unknown loop: {loop_id}")))?;
            if entry.state == LoopState::Stopped || entry.state == LoopState::Completed {
                return Err(SakhaError::invalid_input("sakha-loop", format!("loop {loop_id} is {:?}, cannot tick", entry.state)));
            }
            if entry.state == LoopState::Paused {
                return Err(SakhaError::invalid_input("sakha-loop", format!("loop {loop_id} is paused, resume before ticking")));
            }
            (entry.spec.clone(), Arc::clone(&entry.budget), entry.iteration, entry.goal_id)
        };
        let ledger = Arc::clone(budget.ledger());

        // Wall-clock enforcement (spec "every loop must define max
        // wall-clock time"): debit elapsed time since loop creation before
        // checking/running this tick, so a loop that has simply been open
        // too long (regardless of iteration/cost) is caught even if each
        // individual tick is cheap.
        budget.debit_elapsed_wall_time().ok();

        // Loop Control Rules: "every loop must define max ... cost" — check
        // budget/stall state before doing any work for this tick.
        let pre_check = self.watchdog.lock().unwrap().check_budget(&ledger);
        if pre_check.stalled {
            self.stop(loop_id, StopReason(format!("budget exhausted before tick: {:?}", pre_check.reasons))).await?;
            return Err(SakhaError::budget("sakha-loop", format!("loop {loop_id} budget exhausted: {:?}", pre_check.reasons)));
        }

        // Human approval gate (spec "Required Capabilities" -> "Human
        // approval gates"): a `RequireHumanApproval` loop must not tick
        // (i.e. apply further side effects) until a human has approved the
        // current iteration. `DryRunOnly`/`AutoApply` loops proceed without
        // gating.
        if spec.handoff_policy == HandoffPolicy::RequireHumanApproval {
            let status = self.approvals.request(loop_id, iteration).await?;
            match status {
                ApprovalStatus::Pending => {
                    return Err(SakhaError::permission(
                        "sakha-loop",
                        format!("loop {loop_id} iteration {iteration} awaits human approval before ticking"),
                    ));
                }
                ApprovalStatus::Rejected => {
                    self.stop(loop_id, StopReason(format!("iteration {iteration} rejected by approver"))).await?;
                    return Err(SakhaError::permission(
                        "sakha-loop",
                        format!("loop {loop_id} iteration {iteration} was rejected by an approver"),
                    ));
                }
                ApprovalStatus::Approved => {}
            }
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

        let (mut outcome, succeeded, tick_error) = match tick_result {
            Ok(outcome) => (Some(outcome), true, None),
            Err(err) => (None, false, Some(err)),
        };

        // External side-effect deduplication (spec "Infinite Loop
        // Prevention" -> "External side-effect deduplication"): a
        // non-idempotent loop that reports the same side-effect signature it
        // already reported on a prior tick is a signal that a retry/replay
        // is about to re-apply an effect that already happened (e.g.
        // re-pushing to a remote, re-filing an issue). Flag it as a stall
        // rather than silently repeating the effect.
        let mut dedup_reasons = Vec::new();
        if let Some(outcome) = &outcome {
            if spec.idempotency_mode == crate::spec::IdempotencyMode::NonIdempotent
                && !outcome.side_effect_signatures.is_empty()
            {
                let mut seen = self.side_effects_seen.lock().unwrap_or_else(|p| p.into_inner());
                let loop_seen = seen.entry(loop_id).or_default();
                for sig in &outcome.side_effect_signatures {
                    if !loop_seen.insert(sig.clone()) {
                        dedup_reasons.push(format!("duplicate external side effect suppressed: {sig}"));
                    }
                }
            }
        }
        if !dedup_reasons.is_empty() {
            if let Some(outcome) = outcome.as_mut() {
                outcome.blockers.extend(dedup_reasons.clone());
            }
        }

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
        budget.debit_elapsed_wall_time().ok();
        let post_budget = self.watchdog.lock().unwrap().check_budget(&ledger);
        if post_budget.stalled {
            stall_report.stalled = true;
            stall_report.reasons.extend(post_budget.reasons);
        }
        if !dedup_reasons.is_empty() {
            stall_report.stalled = true;
            stall_report.reasons.extend(dedup_reasons);
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
        let (goal_id, objective) = {
            let mut entries = self.entries.lock().unwrap();
            let entry = entries.get_mut(&loop_id).ok_or_else(|| SakhaError::invalid_input("sakha-loop", "unknown loop"))?;
            entry.state = LoopState::Paused;
            (entry.goal_id, entry.spec.objective.clone())
        };
        // Persist the pause durably (spec "every loop must be pausable" +
        // "durable memory file for every loop") so it survives a restart,
        // not just an in-memory state flip.
        self.persist_pause_state(goal_id, &objective, true).await
    }

    async fn resume(&self, loop_id: LoopId) -> SakhaResult<()> {
        let (goal_id, objective) = {
            let mut entries = self.entries.lock().unwrap();
            let entry = entries.get_mut(&loop_id).ok_or_else(|| SakhaError::invalid_input("sakha-loop", "unknown loop"))?;
            entry.state = LoopState::Running;
            (entry.goal_id, entry.spec.objective.clone())
        };
        self.persist_pause_state(goal_id, &objective, false).await
    }

    async fn stop(&self, loop_id: LoopId, _reason: StopReason) -> SakhaResult<()> {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries.get_mut(&loop_id).ok_or_else(|| SakhaError::invalid_input("sakha-loop", "unknown loop"))?;
        entry.state = LoopState::Stopped;
        Ok(())
    }

    async fn detect_stall(&self, loop_id: LoopId) -> SakhaResult<StallReport> {
        let budget = {
            let entries = self.entries.lock().unwrap();
            let entry = entries.get(&loop_id).ok_or_else(|| SakhaError::invalid_input("sakha-loop", "unknown loop"))?;
            Arc::clone(&entry.budget)
        };
        budget.debit_elapsed_wall_time().ok();
        Ok(self.watchdog.lock().unwrap().check_budget(budget.ledger()))
    }

    async fn list(&self) -> SakhaResult<Vec<LoopSummary>> {
        let entries = self.entries.lock().unwrap();
        Ok(entries
            .values()
            .map(|entry| LoopSummary {
                id: entry.spec.id,
                objective: entry.spec.objective.clone(),
                kind: entry.spec.loop_kind,
                state: entry.state,
                iteration: entry.iteration,
            })
            .collect())
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

    /// Builds a `LoopSpec` with `HandoffPolicy::AutoApply`, for tests that
    /// exercise tick mechanics unrelated to the human-approval gate (which
    /// has its own dedicated tests below). `LoopSpec::new`'s default of
    /// `RequireHumanApproval` is intentionally conservative for production
    /// use; tests that don't care about approval opt out of the gate
    /// explicitly rather than relying on a permissive default.
    fn auto_approved_spec(objective: &str, loop_kind: LoopKind, trigger: Trigger) -> LoopSpec {
        let mut spec = LoopSpec::new(objective, loop_kind, trigger);
        spec.handoff_policy = HandoffPolicy::AutoApply;
        spec
    }

    #[tokio::test]
    async fn default_runtime_tick_reports_not_implemented_without_executor() {
        let runtime = LoopRuntime::new();
        let spec = auto_approved_spec("goal", LoopKind::Agent, Trigger::Manual);
        let loop_id = runtime.create_loop(spec).await.unwrap();
        assert!(runtime.tick(loop_id).await.is_err());
    }

    #[tokio::test]
    async fn tick_drives_loop_to_completion_and_writes_final_handoff() {
        let runtime = runtime_with(Arc::new(ScriptedExecutor::new(2)));
        let spec = auto_approved_spec("ship feature", LoopKind::Agent, Trigger::Manual);
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
        let spec = auto_approved_spec("resume test", LoopKind::Agent, Trigger::Manual);
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
        let spec = auto_approved_spec("goal", LoopKind::Agent, Trigger::Manual);
        let loop_id = runtime.create_loop(spec).await.unwrap();
        runtime.pause(loop_id).await.unwrap();
        assert!(runtime.tick(loop_id).await.is_err());
    }

    #[tokio::test]
    async fn watchdog_stops_loop_on_repeated_state_signature() {
        let runtime = runtime_with(Arc::new(StuckExecutor));
        let mut spec = auto_approved_spec("stuck goal", LoopKind::Agent, Trigger::Manual);
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
        let mut spec = auto_approved_spec("budget-capped goal", LoopKind::Agent, Trigger::Manual);
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
        let spec = auto_approved_spec("goal", LoopKind::Agent, Trigger::Manual);
        let loop_id = runtime.create_loop(spec).await.unwrap();
        runtime.pause(loop_id).await.unwrap();
        runtime.resume(loop_id).await.unwrap();
        assert!(runtime.tick(loop_id).await.is_ok());
    }

    #[tokio::test]
    async fn verification_checks_run_between_ticks_and_gate_completion() {
        use crate::verification_loop::CheckSpec;
        let runtime = runtime_with(Arc::new(ScriptedExecutor::new(1)));
        let mut spec = auto_approved_spec("verified goal", LoopKind::Verification, Trigger::Manual);
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
        let mut spec = auto_approved_spec("goal", LoopKind::Agent, Trigger::Manual);
        spec.max_iterations = 1;
        let loop_id = runtime.create_loop(spec).await.unwrap();
        runtime.tick(loop_id).await.unwrap();
        let report = runtime.detect_stall(loop_id).await.unwrap();
        assert!(report.stalled);
    }

    /// `list()` enumerates every loop created on this runtime (spec "CLI
    /// Commands": `sakha loop list`), reflecting state/iteration changes as
    /// the loop progresses, not just the state at creation time.
    #[tokio::test]
    async fn list_enumerates_created_loops_with_current_state() {
        let runtime = LoopRuntime::new();
        assert!(runtime.list().await.unwrap().is_empty());

        let spec = LoopSpec::new("nightly build check", LoopKind::Verification, Trigger::Manual);
        let loop_id = runtime.create_loop(spec).await.unwrap();

        let summaries = runtime.list().await.unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, loop_id);
        assert_eq!(summaries[0].objective, "nightly build check");
        assert_eq!(summaries[0].kind, LoopKind::Verification);
        assert_eq!(summaries[0].state, LoopState::Created);

        runtime.pause(loop_id).await.unwrap();
        let summaries = runtime.list().await.unwrap();
        assert_eq!(summaries[0].state, LoopState::Paused);
    }

    // --- New coverage: LoopBudget wall-clock, FeedbackRecord, side-effect
    // dedup, human approval gates, and durable pause state. ---

    #[tokio::test]
    async fn wall_clock_budget_exhaustion_stops_loop_across_ticks() {
        let runtime = runtime_with(Arc::new(ScriptedExecutor::new(1000)));
        let mut spec = auto_approved_spec("slow goal", LoopKind::Agent, Trigger::Manual);
        spec.max_iterations = 1000;
        spec.max_wall_time_secs = 0; // Any elapsed time exhausts the budget immediately.
        let loop_id = runtime.create_loop(spec).await.unwrap();

        // The very first tick debits >0ms of elapsed wall time against a
        // 0-second budget, so it must fail rather than run forever.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let result = runtime.tick(loop_id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn tick_by_default_requires_human_approval_before_running() {
        // LoopSpec::new defaults to RequireHumanApproval, per spec "Human
        // approval gates": ticking must be blocked until approved.
        let runtime = runtime_with(Arc::new(ScriptedExecutor::new(1)));
        let spec = LoopSpec::new("needs approval", LoopKind::Agent, Trigger::Manual);
        let loop_id = runtime.create_loop(spec).await.unwrap();

        let blocked = runtime.tick(loop_id).await;
        assert!(blocked.is_err());

        runtime.approvals().resolve(loop_id, 0, true).await.unwrap();
        let approved = runtime.tick(loop_id).await;
        assert!(approved.is_ok());
    }

    #[tokio::test]
    async fn rejected_approval_stops_the_loop() {
        let runtime = runtime_with(Arc::new(ScriptedExecutor::new(1)));
        let spec = LoopSpec::new("needs approval", LoopKind::Agent, Trigger::Manual);
        let loop_id = runtime.create_loop(spec).await.unwrap();

        runtime.approvals().resolve(loop_id, 0, false).await.unwrap();
        let result = runtime.tick(loop_id).await;
        assert!(result.is_err());
        // A rejected approval terminally stops the loop rather than leaving
        // it retriable.
        assert!(runtime.tick(loop_id).await.is_err());
    }

    #[tokio::test]
    async fn duplicate_side_effect_signature_is_flagged_as_stall_for_non_idempotent_loop() {
        struct SideEffectExecutor;
        #[async_trait]
        impl TickExecutor for SideEffectExecutor {
            async fn run_tick(&self, _spec: &LoopSpec, _handoff: Option<&HandoffArtifact>) -> SakhaResult<TickOutcome> {
                Ok(TickOutcome {
                    state_signature: uuid::Uuid::new_v4().to_string(),
                    goal_satisfied: false,
                    side_effect_signatures: vec!["shell:git push origin main".to_string()],
                    ..Default::default()
                })
            }
        }

        let runtime = runtime_with(Arc::new(SideEffectExecutor));
        let mut spec = auto_approved_spec("push once", LoopKind::Agent, Trigger::Manual);
        spec.max_iterations = 10;
        let loop_id = runtime.create_loop(spec).await.unwrap();

        let first = runtime.tick(loop_id).await.unwrap();
        assert_eq!(first.state, LoopState::Running);

        // Second tick reports the exact same side-effect signature again;
        // for a non-idempotent (default) loop this must be flagged, not
        // silently re-applied.
        let second = runtime.tick(loop_id).await.unwrap();
        assert_eq!(second.state, LoopState::Stopped);
        assert!(second.stall.is_some());
        assert!(second.stall.unwrap().reasons.iter().any(|r| r.contains("duplicate external side effect")));
    }

    #[tokio::test]
    async fn idempotent_loop_does_not_flag_repeated_side_effect_signature() {
        struct SideEffectExecutor;
        #[async_trait]
        impl TickExecutor for SideEffectExecutor {
            async fn run_tick(&self, _spec: &LoopSpec, _handoff: Option<&HandoffArtifact>) -> SakhaResult<TickOutcome> {
                Ok(TickOutcome {
                    state_signature: uuid::Uuid::new_v4().to_string(),
                    goal_satisfied: false,
                    side_effect_signatures: vec!["file:write README.md".to_string()],
                    ..Default::default()
                })
            }
        }

        let runtime = runtime_with(Arc::new(SideEffectExecutor));
        let mut spec = auto_approved_spec("idempotent write", LoopKind::Agent, Trigger::Manual).idempotent();
        spec.max_iterations = 10;
        let loop_id = runtime.create_loop(spec).await.unwrap();

        runtime.tick(loop_id).await.unwrap();
        let second = runtime.tick(loop_id).await.unwrap();
        assert_eq!(second.state, LoopState::Running);
        assert!(second.stall.is_none());
    }

    #[tokio::test]
    async fn pause_persists_durably_and_is_recovered_by_a_fresh_runtime() {
        // Spec "every loop must be pausable" + "durable memory file for
        // every loop": a paused loop must stay paused across a simulated
        // process restart (a fresh `LoopRuntime` sharing the same
        // `HandoffStore`), not just within the same in-memory runtime.
        let handoff_store: Arc<dyn HandoffStore> = Arc::new(sakha_memory::InMemoryHandoffStore::new());
        let executor: Arc<dyn TickExecutor> = Arc::new(ScriptedExecutor::new(5));

        let runtime_a = LoopRuntime::with_deps(Arc::clone(&handoff_store), Arc::clone(&executor));
        let spec = auto_approved_spec("long running", LoopKind::Agent, Trigger::Manual);
        let loop_id = spec.id;
        runtime_a.create_loop(spec.clone()).await.unwrap();
        runtime_a.pause(loop_id).await.unwrap();

        // Simulate a restart: a brand-new runtime, same handoff store, same
        // spec id re-registered.
        let runtime_b = LoopRuntime::with_deps(Arc::clone(&handoff_store), Arc::clone(&executor));
        runtime_b.create_loop(spec).await.unwrap();
        let result = runtime_b.tick(loop_id).await;
        assert!(result.is_err(), "loop must still be paused after simulated restart");
    }

    #[tokio::test]
    async fn feedback_records_are_retrievable_per_loop() {
        let runtime = LoopRuntime::new();
        let loop_id = LoopId::new();
        runtime.record_feedback(FeedbackRecord::new(loop_id, FeedbackSource::Ci, "build failed", false));
        runtime.record_feedback(FeedbackRecord::new(loop_id, FeedbackSource::User, "looks good", true));
        runtime.record_feedback(FeedbackRecord::new(LoopId::new(), FeedbackSource::Runtime, "unrelated", true));

        let feedback = runtime.feedback_for(loop_id);
        assert_eq!(feedback.len(), 2);
        assert_eq!(feedback[0].content, "build failed");
        assert!(!feedback[0].accepted);
    }

    #[test]
    fn loop_budget_debits_wall_time_incrementally_without_double_counting() {
        let budget = LoopBudget::new(Budget { max_wall_time: Some(std::time::Duration::from_secs(10)), ..Budget::unlimited() });
        budget.debit_elapsed_wall_time().unwrap();
        let first_used = budget.ledger().used(BudgetDimension::WallTime);
        // A second call immediately after should only add the small delta
        // elapsed since, not double-debit the already-recorded amount.
        budget.debit_elapsed_wall_time().unwrap();
        let second_used = budget.ledger().used(BudgetDimension::WallTime);
        assert!(second_used >= first_used);
        assert!(second_used < 10_000);
    }
}
