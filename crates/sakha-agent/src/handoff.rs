//! `AgentHandoff`: builds a `sakha_memory::HandoffArtifact` from
//! `AgentState` when a session pauses, blocks, or completes.

use sakha_memory::HandoffArtifact;

use crate::state::AgentState;

/// Builds handoff artifacts from agent state so a fresh agent can resume
/// the work later. See spec "Handoff includes plan, status, touched files,
/// next steps".
pub struct AgentHandoff;

impl AgentHandoff {
    pub fn build(state: &AgentState, objective: impl Into<String>, next_action: impl Into<String>) -> HandoffArtifact {
        let goal_id = state.goal_id.unwrap_or_else(sakha_core::GoalId::nil);
        let mut handoff = HandoffArtifact::new(goal_id, objective);
        handoff.current_status = format!("{:?}", state.phase);
        handoff.completed_work = state
            .plan
            .completed_steps
            .iter()
            .filter_map(|&i| state.plan.steps.get(i).cloned())
            .collect();
        handoff.pending_work = state
            .plan
            .steps
            .iter()
            .enumerate()
            .filter(|(i, _)| !state.plan.completed_steps.contains(i))
            .map(|(_, s)| s.clone())
            .collect();
        handoff.next_suggested_action = next_action.into();
        handoff.files_changed = state.touched_files.clone();
        handoff.commands_run = state.commands_run.clone();
        handoff.decisions_made = state.decisions_made.clone();
        if let Some(err) = &state.last_error {
            handoff.blockers.push(err.clone());
        }
        handoff
    }
}
