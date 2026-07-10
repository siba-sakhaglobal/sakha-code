//! `AgentState`: the state machine driving one agent session. See spec
//! `modules/03-agent-loop-engine.md` "Implementation Tasks" step 1.

use serde::{Deserialize, Serialize};

use sakha_core::{Budget, BudgetLedger, GoalId, SessionId};

/// Discrete phases of an agent session's lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPhase {
    Idle,
    Planning,
    AwaitingModel,
    ExecutingTools,
    AwaitingPermission,
    Verifying,
    Completed,
    Blocked,
}

/// Free-form working notes the agent keeps between turns (not sent to the
/// model directly; summarized into context by the planner).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Scratchpad {
    pub notes: Vec<String>,
}

impl Scratchpad {
    pub fn push(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
    }
}

/// Tracks plan progress across turns. See `04-core-domain-model.md` `Goal.plan`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlanState {
    pub steps: Vec<String>,
    pub completed_steps: Vec<usize>,
}

/// The full mutable state of a running agent session.
#[derive(Debug, Clone)]
pub struct AgentState {
    pub session_id: SessionId,
    pub goal_id: Option<GoalId>,
    pub phase: AgentPhase,
    pub scratchpad: Scratchpad,
    pub plan: PlanState,
    pub consecutive_failures: u32,
    pub iterations: u64,
    /// Paths touched by file-mutating tool calls this session, for handoff.
    pub touched_files: Vec<String>,
    /// Shell/git commands executed this session, for handoff.
    pub commands_run: Vec<String>,
    /// Decisions worth recording (e.g. "used self-correction round"), for handoff.
    pub decisions_made: Vec<String>,
    /// Set when a tool call round required self-correction (invalid tool call
    /// recovery); cleared once a valid round completes. Used to ensure only
    /// one self-correction round is ever granted per turn.
    pub used_self_correction: bool,
    /// Last assistant text produced by the model (for a "final answer" turn).
    pub last_response_text: Option<String>,
    /// Last error, if the loop stopped abnormally (budget/permission/etc).
    pub last_error: Option<String>,
    /// Progress signature (see `progress_signature()`) observed the last time
    /// `TerminationGuard::evaluate` ran, used by the no-progress detector to
    /// notice when consecutive iterations leave no observable trace of work.
    pub last_progress_signature: Option<u64>,
    /// Number of consecutive `TerminationGuard::evaluate` calls whose
    /// `progress_signature()` matched `last_progress_signature`, i.e. state
    /// that looks identical to the previous check. Reset to 0 whenever the
    /// signature changes.
    pub stale_iterations: u32,
}

impl AgentState {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            goal_id: None,
            phase: AgentPhase::Idle,
            scratchpad: Scratchpad::default(),
            plan: PlanState::default(),
            consecutive_failures: 0,
            iterations: 0,
            touched_files: Vec::new(),
            commands_run: Vec::new(),
            decisions_made: Vec::new(),
            used_self_correction: false,
            last_response_text: None,
            last_error: None,
            last_progress_signature: None,
            stale_iterations: 0,
        }
    }

    /// A cheap fingerprint of "observable work done so far": counts of
    /// touched files, commands run, completed plan steps, decisions logged,
    /// and the last response text. Two turns with an identical signature
    /// produced no new observable effect, which is the "no-progress" signal
    /// from spec `03-agent-loop-engine.md` Termination Criteria. Not a
    /// cryptographic hash — collisions are acceptable since this only gates a
    /// heuristic stop condition, and a false "no progress" stop is safe
    /// (the loop halts rather than spinning forever).
    pub fn progress_signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.touched_files.len().hash(&mut hasher);
        self.commands_run.len().hash(&mut hasher);
        self.plan.completed_steps.len().hash(&mut hasher);
        self.decisions_made.len().hash(&mut hasher);
        self.last_response_text.hash(&mut hasher);
        hasher.finish()
    }
}

/// Ambient config for an `Agent`: budgets and behavior toggles.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub budget: Budget,
    pub max_consecutive_failures: u32,
    pub allow_parallel_tool_calls: bool,
    /// Hard ceiling on ReAct loop iterations within a single `run_turn` call,
    /// independent of the budget ledger's `max_loop_iterations` (which may be
    /// `None`/unlimited). Prevents a runaway turn even with an unlimited
    /// budget. See spec "Infinite-loop guard".
    pub max_turn_iterations: u64,
    /// Which model id to request from the provider.
    pub model: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            budget: Budget::unlimited(),
            max_consecutive_failures: 3,
            allow_parallel_tool_calls: false,
            max_turn_iterations: 25,
            model: "default".to_string(),
        }
    }
}

/// Convenience wrapper pairing config with a live budget ledger.
pub struct AgentBudget {
    pub config: AgentConfig,
    pub ledger: BudgetLedger,
}

impl AgentBudget {
    pub fn new(config: AgentConfig) -> Self {
        let ledger = BudgetLedger::new(config.budget);
        Self { config, ledger }
    }
}
