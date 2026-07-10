//! `LoopSpec` and trigger types. See `04-core-domain-model.md` `LoopSpec`
//! and `modules/04-loop-engineering.md`.

use serde::{Deserialize, Serialize};

use sakha_core::{Budget, LoopId};

/// A cron-like schedule expression, kept as an opaque string until a real
/// cron parser is wired in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronSpec(pub String);

/// Where an event-driven loop's trigger events come from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum EventSource {
    GithubIssue,
    GithubPullRequest,
    GithubCi,
    Webhook { path: String },
    FilesystemWatch { path: String },
    QueueMessage { queue_name: String },
    ManualGoal,
    SearchMonitor { query: String },
}

/// What kind of loop this spec describes. See spec "Loop Layers".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopKind {
    Agent,
    Verification,
    Event,
    Research,
    Feedback,
    Replay,
}

/// What causes a loop tick to be scheduled.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Trigger {
    Schedule(CronSpec),
    Event(EventSource),
    Manual,
}

/// Who/what verifies the outcome of a loop tick.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerifierSpec {
    pub check_names: Vec<String>,
    /// Deterministic command-based checks (exit-code verification) run
    /// between ticks. See `modules/12-verification-evals.md` "Command
    /// verifier".
    pub checks: Vec<crate::verification_loop::CheckSpec>,
    /// Rubric criteria the tick's output must satisfy (module 12 "Rubric
    /// verifier").
    pub rubric: crate::verification_loop::Rubric,
}

/// Whether/how a loop's side effects may be applied automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffPolicy {
    AutoApply,
    RequireHumanApproval,
    DryRunOnly,
}

/// Mirrors `04-core-domain-model.md` `LoopSpec`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopSpec {
    pub id: LoopId,
    pub trigger: Trigger,
    pub objective: String,
    pub loop_kind: LoopKind,
    pub schedule: Option<CronSpec>,
    pub event_source: Option<EventSource>,
    pub max_iterations: u32,
    pub max_wall_time_secs: u64,
    pub max_cost_micros: u64,
    pub verifier: VerifierSpec,
    pub handoff_policy: HandoffPolicy,
}

impl LoopSpec {
    pub fn new(objective: impl Into<String>, loop_kind: LoopKind, trigger: Trigger) -> Self {
        Self {
            id: LoopId::new(),
            trigger,
            objective: objective.into(),
            loop_kind,
            schedule: None,
            event_source: None,
            max_iterations: 50,
            max_wall_time_secs: 3600,
            max_cost_micros: 0,
            verifier: VerifierSpec::default(),
            handoff_policy: HandoffPolicy::RequireHumanApproval,
        }
    }

    pub fn budget(&self) -> Budget {
        Budget {
            max_loop_iterations: Some(self.max_iterations as u64),
            max_cost_micros: if self.max_cost_micros > 0 { Some(self.max_cost_micros) } else { None },
            max_wall_time: Some(std::time::Duration::from_secs(self.max_wall_time_secs)),
            ..Budget::unlimited()
        }
    }

    pub fn with_schedule(mut self, cron: CronSpec) -> Self {
        self.schedule = Some(cron);
        self
    }

    pub fn with_event_source(mut self, source: EventSource) -> Self {
        self.event_source = Some(source);
        self
    }

    pub fn with_verifier(mut self, verifier: VerifierSpec) -> Self {
        self.verifier = verifier;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_maps_zero_cost_to_unlimited() {
        let spec = LoopSpec::new("goal", LoopKind::Agent, Trigger::Manual);
        let budget = spec.budget();
        assert_eq!(budget.max_cost_micros, None);
        assert_eq!(budget.max_loop_iterations, Some(50));
    }

    #[test]
    fn budget_maps_positive_cost_through() {
        let mut spec = LoopSpec::new("goal", LoopKind::Agent, Trigger::Manual);
        spec.max_cost_micros = 500;
        assert_eq!(spec.budget().max_cost_micros, Some(500));
    }

    #[test]
    fn spec_round_trips_through_json() {
        let spec = LoopSpec::new("goal", LoopKind::Research, Trigger::Manual);
        let json = serde_json::to_string(&spec).unwrap();
        let back: LoopSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.objective, spec.objective);
        assert_eq!(back.loop_kind, spec.loop_kind);
    }
}
