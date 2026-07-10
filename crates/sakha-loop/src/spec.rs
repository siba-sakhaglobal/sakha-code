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

/// Whether a loop's ticks are safe to re-run without changing the outcome
/// beyond the first successful run. See spec "Loop Control Rules" -> "every
/// loop must be idempotent or explicitly marked non-idempotent": a loop that
/// performs external side effects (sending an email, pushing to a remote,
/// filing an issue) must say so explicitly rather than silently defaulting
/// to "safe to retry".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdempotencyMode {
    /// Re-running a tick (e.g. after a crash/retry) is safe: it produces the
    /// same end state, or is naturally deduplicated.
    Idempotent,
    /// Re-running a tick may cause duplicate external side effects; the
    /// runtime must not silently retry/replay a tick in this mode without a
    /// dedup mechanism.
    NonIdempotent,
}

/// A single side-effect permission a loop is allowed to exercise while
/// ticking, e.g. `"shell.run_safe"`, `"git.remote_push"`, matching
/// `sakha_security`'s permission rule namespaces. See spec "Loop Control
/// Rules" -> "every loop must define side-effect permissions".
pub type SideEffectPermission = String;

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
    /// Side-effect permissions this loop is allowed to exercise (e.g.
    /// `["shell.run_safe", "file.write"]`); empty means the loop may not
    /// perform any side effects, only read-only work. Spec "every loop must
    /// define side-effect permissions".
    #[serde(default)]
    pub side_effect_permissions: Vec<SideEffectPermission>,
    /// Whether this loop's ticks are idempotent. Spec "every loop must be
    /// idempotent or explicitly marked non-idempotent"; defaults to
    /// `NonIdempotent` so a spec that doesn't think about this is treated
    /// conservatively (subject to side-effect dedup) rather than assumed safe.
    #[serde(default = "default_idempotency_mode")]
    pub idempotency_mode: IdempotencyMode,
}

fn default_idempotency_mode() -> IdempotencyMode {
    IdempotencyMode::NonIdempotent
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
            side_effect_permissions: Vec::new(),
            idempotency_mode: IdempotencyMode::NonIdempotent,
        }
    }

    /// Declares this loop idempotent (safe to re-run a tick without
    /// duplicating side effects).
    pub fn idempotent(mut self) -> Self {
        self.idempotency_mode = IdempotencyMode::Idempotent;
        self
    }

    /// Grants a side-effect permission to this loop.
    pub fn with_side_effect_permission(mut self, permission: impl Into<SideEffectPermission>) -> Self {
        self.side_effect_permissions.push(permission.into());
        self
    }

    /// Whether this loop is permitted to exercise `permission`.
    pub fn allows_side_effect(&self, permission: &str) -> bool {
        self.side_effect_permissions.iter().any(|p| p == permission)
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

    #[test]
    fn new_spec_defaults_to_non_idempotent_with_no_side_effect_permissions() {
        let spec = LoopSpec::new("goal", LoopKind::Agent, Trigger::Manual);
        assert_eq!(spec.idempotency_mode, IdempotencyMode::NonIdempotent);
        assert!(spec.side_effect_permissions.is_empty());
        assert!(!spec.allows_side_effect("shell.run_safe"));
    }

    #[test]
    fn with_side_effect_permission_grants_named_permission_only() {
        let spec = LoopSpec::new("goal", LoopKind::Agent, Trigger::Manual)
            .with_side_effect_permission("shell.run_safe");
        assert!(spec.allows_side_effect("shell.run_safe"));
        assert!(!spec.allows_side_effect("git.remote_push"));
    }

    #[test]
    fn idempotent_builder_sets_idempotency_mode() {
        let spec = LoopSpec::new("goal", LoopKind::Agent, Trigger::Manual).idempotent();
        assert_eq!(spec.idempotency_mode, IdempotencyMode::Idempotent);
    }

    #[test]
    fn loop_spec_missing_idempotency_mode_in_json_defaults_to_non_idempotent() {
        // Simulates an older/hand-written spec payload that predates the
        // idempotency field, per spec "must be idempotent or explicitly
        // marked non-idempotent": the conservative default is non-idempotent.
        let json = r#"{
            "id": "00000000-0000-0000-0000-000000000000",
            "trigger": {"kind": "manual"},
            "objective": "goal",
            "loop_kind": "agent",
            "schedule": null,
            "event_source": null,
            "max_iterations": 50,
            "max_wall_time_secs": 3600,
            "max_cost_micros": 0,
            "verifier": {"check_names": [], "checks": [], "rubric": {"criteria": []}},
            "handoff_policy": "require_human_approval"
        }"#;
        let spec: LoopSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.idempotency_mode, IdempotencyMode::NonIdempotent);
        assert!(spec.side_effect_permissions.is_empty());
    }
}
