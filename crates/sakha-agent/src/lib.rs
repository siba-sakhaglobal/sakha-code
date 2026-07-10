//! sakha-agent: agent loop, message assembler, planning state, tool call interpreter, termination detector.
//!
//! Public API skeleton — see spec `modules/03-agent-loop-engine.md` and
//! `crates/crate-work-breakdown.md`.

pub mod agent;
pub mod handoff;
pub mod messages;
pub mod policy;
pub mod state;
pub mod termination;
pub mod tool_loop;

pub use agent::{Agent, AgentDeps, LoopType};
pub use handoff::AgentHandoff;
pub use messages::{MessageGraph, TurnContext};
pub use policy::{AgentPolicy, DefaultAgentPolicy, PolicyAction, PromptBlock};
pub use state::{AgentBudget, AgentConfig, AgentPhase, AgentState, PlanState, Scratchpad};
pub use termination::{LoopDecision, StopReason, TerminationGuard};
pub use tool_loop::{DefaultToolCallInterpreter, ToolCallInterpreter, ToolCallProgress, ToolCallState};

pub fn crate_name() -> &'static str {
    "sakha-agent"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn termination_guard_stops_on_repeated_failure() {
        let guard = TerminationGuard::new(3, 100);
        let mut state = AgentState::new(sakha_core::SessionId::new());
        state.consecutive_failures = 3;
        let decision = guard.evaluate(&state);
        assert_eq!(decision, LoopDecision::Stop { reason: "repeated failure".into() });
    }

    #[test]
    fn termination_guard_stops_on_budget_style_iteration_cap() {
        let guard = TerminationGuard::new(10, 5);
        let mut state = AgentState::new(sakha_core::SessionId::new());
        state.iterations = 5;
        let decision = guard.evaluate(&state);
        assert_eq!(decision, LoopDecision::Stop { reason: "max iterations reached".into() });
    }

    #[test]
    fn default_policy_continues_until_completed() {
        let policy = DefaultAgentPolicy::new("coding-agent");
        let mut state = AgentState::new(sakha_core::SessionId::new());
        assert_eq!(policy.should_continue(&state), LoopDecision::Continue);
        state.phase = AgentPhase::Completed;
        assert_eq!(policy.should_continue(&state), LoopDecision::Stop { reason: "completed".into() });
    }

    #[test]
    fn handoff_includes_plan_status_and_next_action() {
        let mut state = AgentState::new(sakha_core::SessionId::new());
        state.plan.steps = vec!["write tests".into(), "implement feature".into()];
        state.plan.completed_steps = vec![0];
        let handoff = AgentHandoff::build(&state, "ship feature x", "implement feature");
        assert_eq!(handoff.completed_work, vec!["write tests".to_string()]);
        assert_eq!(handoff.pending_work, vec!["implement feature".to_string()]);
        assert_eq!(handoff.next_suggested_action, "implement feature");
    }

    #[tokio::test]
    async fn tool_call_interpreter_assembles_complete_call() {
        use sakha_provider::ToolCallDelta;

        let mut interpreter = DefaultToolCallInterpreter::default();
        interpreter.assemble(ToolCallDelta { extra_content: None,
            index: 0,
            id: Some("call_1".into()),
            name: Some("file.read".into()),
            arguments_fragment: "{\"path\":\"a.txt\"}".into(),
        });
        let calls = interpreter.finish().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name.0, "file.read");
    }
}
