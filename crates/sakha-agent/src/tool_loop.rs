//! `ToolCallInterpreter`: parses streamed tool call events into validated
//! calls, and `ToolCallState` tracking per-call progress.

use serde::{Deserialize, Serialize};

use sakha_core::{SakhaResult, ToolCallId};
use sakha_provider::{ModelEvent, ToolCallDelta};
use sakha_tools::{ToolCall, ToolCallStatus, ToolName, ToolRegistry};

/// Tracks the lifecycle of one tool call as it's parsed, executed, and
/// completed within a turn.
#[derive(Debug, Clone)]
pub struct ToolCallState {
    pub id: ToolCallId,
    pub tool_name: ToolName,
    pub status: ToolCallStatus,
}

/// A `ToolCall` whose input has already been checked against its tool's
/// input schema (`Tool::validate`), per `modules/03-agent-loop-engine.md`
/// `ToolCallInterpreter::validate(call) -> ValidatedToolCall`. Distinct from
/// `sakha_tools::ValidatedInput` (which wraps just the input JSON): this
/// carries the full call identity through validation so the loop can route
/// it straight to the executor without re-deriving `tool_name`/`id`.
#[derive(Debug, Clone)]
pub struct ValidatedToolCall {
    pub id: ToolCallId,
    pub tool_name: ToolName,
    pub input_json: serde_json::Value,
}

/// Parses `ModelEvent`s into `ToolCallDelta`s, assembles complete calls, and
/// validates them before dispatch. See `04-core-domain-model.md`
/// `ToolCallInterpreter`.
pub trait ToolCallInterpreter: Send + Sync {
    fn parse(&self, event: &ModelEvent) -> Option<ToolCallDelta>;

    fn assemble(&mut self, delta: ToolCallDelta);

    fn finish(&mut self) -> SakhaResult<Vec<ToolCall>>;

    /// Validates one assembled `ToolCall` against `registry`: the tool must
    /// be known, and its input must pass `Tool::validate`. Returns `Err`
    /// (never panics) for an unknown tool or malformed input, per spec
    /// "Model emits invalid JSON tool input" failure mode — callers use this
    /// to reject bad calls (and route them into the loop's self-correction
    /// path) before they ever reach permission checks or execution.
    fn validate(&self, call: &ToolCall, registry: &ToolRegistry) -> SakhaResult<ValidatedToolCall> {
        let tool = registry.get_or_err(&call.tool_name)?;
        let validated_input = tool.validate(call.input_json.clone())?;
        Ok(ValidatedToolCall {
            id: call.id,
            tool_name: call.tool_name.clone(),
            input_json: validated_input.0,
        })
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn registry_with_echo() -> ToolRegistry {
        use async_trait::async_trait;
        use sakha_tools::{
            IdempotencyPolicy, Tool, ToolContext, ToolInputSchema, ToolOutputSchema, ToolPermissionSpec, ToolPlan,
            ToolResult, ToolSpec, ToolSummary, ValidatedInput,
        };

        struct EchoTool;

        #[async_trait]
        impl Tool for EchoTool {
            fn spec(&self) -> ToolSpec {
                ToolSpec {
                    name: ToolName::new("echo"),
                    description: "echoes input".into(),
                    input_schema: ToolInputSchema(serde_json::json!({
                        "type": "object",
                        "properties": {"value": {"type": "string"}},
                        "required": ["value"]
                    })),
                    output_schema: ToolOutputSchema(serde_json::json!({"type": "object"})),
                    permission_spec: ToolPermissionSpec::default(),
                    idempotency_policy: IdempotencyPolicy::NotIdempotent,
                }
            }

            fn validate(&self, input: serde_json::Value) -> SakhaResult<ValidatedInput> {
                if input.get("value").and_then(|v| v.as_str()).is_none() {
                    return Err(sakha_core::SakhaError::invalid_input("test", "missing value"));
                }
                Ok(ValidatedInput(input))
            }

            async fn plan(&self, _input: &ValidatedInput, _context: &ToolContext) -> SakhaResult<ToolPlan> {
                Ok(ToolPlan::default())
            }

            async fn execute(&self, input: &ValidatedInput, _context: &ToolContext) -> SakhaResult<ToolResult> {
                Ok(ToolResult::success(input.0.clone()))
            }

            fn summarize(&self, result: &ToolResult) -> ToolSummary {
                ToolSummary { text: result.output_json.to_string(), truncated: false }
            }
        }

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool));
        registry
    }

    #[test]
    fn validate_rejects_unknown_tool() {
        let interpreter = DefaultToolCallInterpreter::default();
        let registry = ToolRegistry::new();
        let call = ToolCall {
            id: ToolCallId::new(),
            tool_name: ToolName::new("does.not.exist"),
            input_json: serde_json::json!({}),
        };
        let result = interpreter.validate(&call, &registry);
        assert!(result.is_err());
    }

    #[test]
    fn validate_rejects_input_failing_tool_schema() {
        let interpreter = DefaultToolCallInterpreter::default();
        let registry = registry_with_echo();
        let call = ToolCall {
            id: ToolCallId::new(),
            tool_name: ToolName::new("echo"),
            input_json: serde_json::json!({}), // missing required "value"
        };
        let result = interpreter.validate(&call, &registry);
        assert!(result.is_err());
    }

    #[test]
    fn validate_accepts_known_tool_with_valid_input() {
        let interpreter = DefaultToolCallInterpreter::default();
        let registry = registry_with_echo();
        let call = ToolCall {
            id: ToolCallId::new(),
            tool_name: ToolName::new("echo"),
            input_json: serde_json::json!({"value": "hi"}),
        };
        let validated = interpreter.validate(&call, &registry).unwrap();
        assert_eq!(validated.tool_name, ToolName::new("echo"));
        assert_eq!(validated.input_json["value"], "hi");
    }
}
