//! `AgentPolicy`: pluggable per-role behavior. See `04-core-domain-model.md`
//! `AgentPolicy` (as part of the `Agent` trait family) and
//! `modules/03-agent-loop-engine.md` "Main Traits".

use sakha_provider::ToolDefinition;
use sakha_tools::{ToolName, ToolResult};

use crate::state::AgentState;
use crate::termination::LoopDecision;

/// A block of prompt text produced by a policy for the system prompt.
#[derive(Debug, Clone)]
pub struct PromptBlock(pub String);

/// What the agent loop should do after observing a tool result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyAction {
    Continue,
    Retry,
    Stop,
    EscalateToHuman,
}

/// Defines role-specific behavior: system prompt, allowed tools,
/// continuation decisions, and reactions to tool results.
pub trait AgentPolicy: Send + Sync {
    fn system_prompt(&self, ctx: &AgentState) -> PromptBlock;

    fn allowed_tools(&self, ctx: &AgentState) -> Vec<ToolName>;

    fn should_continue(&self, state: &AgentState) -> LoopDecision;

    fn on_tool_result(&self, result: &ToolResult) -> PolicyAction;
}

/// A minimal default policy: allows no tools by default, continues until
/// explicitly told to stop, and always continues after a tool result. Real
/// role-specific policies (interactive/goal/repair/etc) override this.
pub struct DefaultAgentPolicy {
    pub role_name: String,
    pub tools: Vec<ToolName>,
}

impl DefaultAgentPolicy {
    pub fn new(role_name: impl Into<String>) -> Self {
        Self { role_name: role_name.into(), tools: Vec::new() }
    }
}

impl AgentPolicy for DefaultAgentPolicy {
    fn system_prompt(&self, _ctx: &AgentState) -> PromptBlock {
        PromptBlock(format!("You are acting as: {}.", self.role_name))
    }

    fn allowed_tools(&self, _ctx: &AgentState) -> Vec<ToolName> {
        self.tools.clone()
    }

    fn should_continue(&self, state: &AgentState) -> LoopDecision {
        if state.phase == crate::state::AgentPhase::Completed {
            LoopDecision::Stop { reason: "completed".into() }
        } else {
            LoopDecision::Continue
        }
    }

    fn on_tool_result(&self, _result: &ToolResult) -> PolicyAction {
        PolicyAction::Continue
    }
}

/// Unused import guard: keeps `ToolDefinition` referenced for future policy
/// implementations that need to filter/shape tool definitions per role.
#[allow(dead_code)]
fn _uses_tool_definition(_: &ToolDefinition) {}
