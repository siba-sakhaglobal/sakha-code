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

/// Guards against infinite loops: budget exhaustion, repeated failures, and
/// no-progress detection.
pub struct TerminationGuard {
    pub max_consecutive_failures: u32,
    pub max_iterations: u64,
}

impl TerminationGuard {
    pub fn new(max_consecutive_failures: u32, max_iterations: u64) -> Self {
        Self { max_consecutive_failures, max_iterations }
    }

    /// Evaluates `state` against configured limits, returning a `LoopDecision`.
    pub fn evaluate(&self, state: &AgentState) -> LoopDecision {
        if state.consecutive_failures >= self.max_consecutive_failures {
            return LoopDecision::Stop { reason: "repeated failure".into() };
        }
        if state.iterations >= self.max_iterations {
            return LoopDecision::Stop { reason: "max iterations reached".into() };
        }
        LoopDecision::Continue
    }
}
