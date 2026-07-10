//! sakha-loop: loop specs, scheduler, event triggers, work queue, stall detection, verification loop.
//!
//! Public API skeleton — see spec `modules/04-loop-engineering.md` and
//! `crates/crate-work-breakdown.md`.

pub mod event_queue;
pub mod replay;
pub mod runtime;
pub mod scheduler;
pub mod spec;
pub mod verification_loop;
pub mod watchdog;

pub use event_queue::{InMemoryLoopQueue, LeasedMessage, LoopQueue, QueueMessage};
pub use replay::{LoopSkill, LoopSkillRecorder, ReplayEngine, ReplayStep, ReplayTrace};
pub use runtime::{
    ApprovalStatus, ApprovalStore, FeedbackRecord, FeedbackSource, InMemoryApprovalStore, LoopBudget, LoopController,
    LoopRuntime, LoopState, LoopSummary, LoopTick, LoopTickResult, NullTickExecutor, StopReason, TickExecutor, TickOutcome,
};
pub use scheduler::{ImmediateScheduler, IntervalCronScheduler, ParsedSchedule, Scheduler};
pub use spec::{
    CronSpec, EventSource, HandoffPolicy, IdempotencyMode, LoopKind, LoopSpec, SideEffectPermission, Trigger,
    VerifierSpec,
};
pub use verification_loop::{
    run_check, run_checks, CheckResult, CheckSpec, CommandVerifier, Grade, NullVerifier, Rubric, VerificationResult,
    Verifier,
};
pub use watchdog::{LoopWatchdog, StallReport};

pub fn crate_name() -> &'static str {
    "sakha-loop"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn scheduled_loop_can_be_created_and_ticked_state_tracked() {
        let runtime = LoopRuntime::new();
        let spec = LoopSpec::new("nightly build check", LoopKind::Verification, Trigger::Manual);
        let loop_id = runtime.create_loop(spec).await.unwrap();
        // tick is not yet implemented (agent wiring pending), but must error
        // cleanly rather than panic.
        assert!(runtime.tick(loop_id).await.is_err());
    }

    #[tokio::test]
    async fn event_loop_deduplicates_event_by_dedup_key() {
        let queue = InMemoryLoopQueue::new();
        let loop_id = sakha_core::LoopId::new();
        queue.enqueue(QueueMessage { loop_id, dedup_key: "issue-42".into(), payload: serde_json::json!({}) }).await.unwrap();
        queue.enqueue(QueueMessage { loop_id, dedup_key: "issue-42".into(), payload: serde_json::json!({}) }).await.unwrap();
        let first = queue.lease_next().await.unwrap();
        assert!(first.is_some());
        let second = queue.lease_next().await.unwrap();
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn loop_pauses_and_resumes() {
        let runtime = LoopRuntime::new();
        let spec = LoopSpec::new("watch ci", LoopKind::Event, Trigger::Manual);
        let loop_id = runtime.create_loop(spec).await.unwrap();
        runtime.pause(loop_id).await.unwrap();
        runtime.resume(loop_id).await.unwrap();
    }

    #[test]
    fn budget_stops_loop_via_loop_spec_budget_mapping() {
        let mut spec = LoopSpec::new("budgeted loop", LoopKind::Agent, Trigger::Manual);
        spec.max_iterations = 5;
        let budget = spec.budget();
        assert_eq!(budget.max_loop_iterations, Some(5));
    }

    #[tokio::test]
    async fn replay_skill_dry_run_executes_without_model_call() {
        let engine = ReplayEngine::new();
        let skill = LoopSkill { name: "noop".into(), trace: ReplayTrace::default(), variables: vec![] };
        let result = engine.dry_run(&skill).await.unwrap();
        assert!(result);
    }

    #[test]
    fn watchdog_flags_repeated_state_hash_as_stall() {
        let mut watchdog = LoopWatchdog::new(2);
        let loop_id = sakha_core::LoopId::new();
        watchdog.observe(loop_id, "hash-a".into());
        watchdog.observe(loop_id, "hash-a".into());
        let report = watchdog.observe(loop_id, "hash-a".into());
        assert!(report.stalled);
    }
}
