//! `ToolCallInterpreter`: parses streamed tool call events into validated
//! calls, and `ToolCallState` tracking per-call progress.

use serde::{Deserialize, Serialize};

use sakha_core::{SakhaResult, ToolCallId};
use sakha_provider::{ModelEvent, ToolCallDelta};
use sakha_tools::{ToolCall, ToolCallStatus, ToolName};

/// Tracks the lifecycle of one tool call as it's parsed, executed, and
/// completed within a turn.
#[derive(Debug, Clone)]
pub struct ToolCallState {
    pub id: ToolCallId,
    pub tool_name: ToolName,
    pub status: ToolCallStatus,
}

/// Parses `ModelEvent`s into `ToolCallDelta`s, assembles complete calls, and
/// validates them before dispatch. See `04-core-domain-model.md`
/// `ToolCallInterpreter`.
pub trait ToolCallInterpreter: Send + Sync {
    fn parse(&self, event: &ModelEvent) -> Option<ToolCallDelta>;

    fn assemble(&mut self, delta: ToolCallDelta);

    fn finish(&mut self) -> SakhaResult<Vec<ToolCall>>;
}

/// Default interpreter built on `sakha_provider::ToolCallAssembler`.
#[derive(Default)]
pub struct DefaultToolCallInterpreter {
    assembler: sakha_provider::ToolCallAssembler,
}

impl ToolCallInterpreter for DefaultToolCallInterpreter {
    fn parse(&self, event: &ModelEvent) -> Option<ToolCallDelta> {
        match event {
            ModelEvent::ToolCallDelta(delta) => Some(delta.clone()),
            _ => None,
        }
    }

    fn assemble(&mut self, delta: ToolCallDelta) {
        self.assembler.push(delta);
    }

    fn finish(&mut self) -> SakhaResult<Vec<ToolCall>> {
        let assembler = std::mem::take(&mut self.assembler);
        let assembled = assembler.finish();
        Ok(assembled
            .into_iter()
            .filter_map(|call| {
                let input_json: serde_json::Value = serde_json::from_str(&call.arguments_json).ok()?;
                Some(ToolCall {
                    id: ToolCallId::new(),
                    tool_name: ToolName::new(call.name),
                    input_json,
                })
            })
            .collect())
    }
}

/// Serializable snapshot of tool-call progress, e.g. for UI/audit streaming.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallProgress {
    pub tool_call_id: ToolCallId,
    pub tool_name: String,
    pub status: String,
}
