//! `TerminationGuard` / `LoopDecision`: decides whether the agent loop
//! continues. See spec "Termination Criteria".

use serde::{Deserialize, Serialize};

use crate::state::AgentState;

/// The decision returned by `AgentPolicy::should_continue` /
/// `TerminationGuard::evaluate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "decision")]
pub enum LoopDecision {
    Continue,
    Stop { reason: String },
    Blocked { reason: String, next_action: String },
}

/// Enumerates why a loop stopped, per spec "Termination Criteria".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    FinalAnswer,
    GoalAcceptanceCriteriaMet,
    VerificationPassed,
    BudgetExhausted,
    HumanApprovalNeeded,
    RepeatedFailure,
    NoProgress,
    UnsafeActionBlocked,
    ExternalDependencyUnavailable,
}

/// Default number of consecutive no-op iterations (identical
/// `AgentState::progress_signature()`) tolerated before the no-progress
/// detector stops the loop. See spec Termination Criteria "No-progress
/// detector": the model can keep calling the model/tools without ever
/// changing observable state (e.g. repeating a read-only tool call), which
/// `consecutive_failures` and `iterations` alone don't catch since each
/// individual call may succeed.
pub const DEFAULT_MAX_STALE_ITERATIONS: u32 = 4;

/// Guards against infinite loops: budget exhaustion, repeated failures, and
/// no-progress detection.
pub struct TerminationGuard {
    pub max_consecutive_failures: u32,
    pub max_iterations: u64,
    /// Consecutive iterations with an unchanged progress signature allowed
    /// before stopping with `StopReason::NoProgress`.
    pub max_stale_iterations: u32,
}

impl TerminationGuard {
    pub fn new(max_consecutive_failures: u32, max_iterations: u64) -> Self {
        Self {
            max_consecutive_failures,
            max_iterations,
            max_stale_iterations: DEFAULT_MAX_STALE_ITERATIONS,
        }
    }

    pub fn with_max_stale_iterations(mut self, max_stale_iterations: u32) -> Self {
        self.max_stale_iterations = max_stale_iterations;
        self
    }

    /// Evaluates `state` against configured limits, returning a `LoopDecision`.
    /// Does not itself mutate `state`; callers update
    /// `state.last_progress_signature` / `state.stale_iterations` via
    /// `record_progress` after each iteration so this stays a pure read.
    pub fn evaluate(&self, state: &AgentState) -> LoopDecision {
        if state.consecutive_failures >= self.max_consecutive_failures {
            return LoopDecision::Stop { reason: "repeated failure".into() };
        }
        if state.iterations >= self.max_iterations {
            return LoopDecision::Stop { reason: "max iterations reached".into() };
        }
        if state.stale_iterations >= self.max_stale_iterations {
            return LoopDecision::Stop { reason: "no progress".into() };
        }
        LoopDecision::Continue
    }

    /// Updates `state`'s progress-tracking fields for the iteration that just
    /// completed: recomputes the progress signature and either resets
    /// `stale_iterations` (state changed) or increments it (state identical
    /// to the previous check). Call this once per loop iteration, after any
    /// tool execution / model response has been folded into `state`.
    pub fn record_progress(&self, state: &mut AgentState) {
        let signature = state.progress_signature();
        if state.last_progress_signature == Some(signature) {
            state.stale_iterations += 1;
        } else {
            state.stale_iterations = 0;
        }
        state.last_progress_signature = Some(signature);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec requirement (`03-agent-loop-engine.md` Termination Criteria
    /// "No-progress detector"): iterations that change nothing observable
    /// (no new touched files/commands/plan progress/response text) must
    /// eventually stop the loop, even though no individual iteration failed
    /// and the hard iteration cap hasn't been hit.
    #[test]
    fn no_progress_detector_stops_loop_after_repeated_identical_state() {
        let guard = TerminationGuard::new(100, 1000).with_max_stale_iterations(3);
        let mut state = AgentState::new(sakha_core::SessionId::new());

        // First record_progress() only establishes the baseline signature
        // (stale_iterations stays 0); each subsequent call with an unchanged
        // signature increments it by one.
        guard.record_progress(&mut state);
        for _ in 0..3 {
            assert_eq!(guard.evaluate(&state), LoopDecision::Continue);
            guard.record_progress(&mut state);
        }

        // Three consecutive identical-signature iterations recorded after
        // the baseline; evaluate() must now stop with NoProgress rather than
        // continuing indefinitely.
        assert_eq!(guard.evaluate(&state), LoopDecision::Stop { reason: "no progress".into() });
    }

    #[test]
    fn progress_resets_stale_counter() {
        let guard = TerminationGuard::new(100, 1000).with_max_stale_iterations(2);
        let mut state = AgentState::new(sakha_core::SessionId::new());

        guard.record_progress(&mut state); // iteration 1: no change yet (baseline)
        assert_eq!(guard.evaluate(&state), LoopDecision::Continue);

        // Real work happens: a file gets touched.
        state.touched_files.push("a.txt".into());
        guard.record_progress(&mut state);
        assert_eq!(state.stale_iterations, 0);
        assert_eq!(guard.evaluate(&state), LoopDecision::Continue);
    }
}
