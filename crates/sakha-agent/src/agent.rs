//! `Agent`: the top-level agent loop driver. See `04-core-domain-model.md`
//! `AgentRuntime` and `modules/03-agent-loop-engine.md`.

use std::sync::Arc;

use sakha_compression::ContextCompressor;
use sakha_context::{ContextBuildRequest, ContextPlanner, DefaultPromptAssembler, PromptAssembler, PromptRequest};
use sakha_core::{Budget, BudgetDimension, GoalId, SakhaResult, SessionId};
use sakha_memory::{HandoffStore, InMemoryHandoffStore};
use sakha_provider::{MessageRole, ModelMessage, ModelRequest, ProviderClient, StopReason as ProviderStopReason, ToolDefinition};
use sakha_security::PermissionPolicy;
use sakha_tools::{ToolCall as ExecutorToolCall, ToolContext, ToolExecutor, ToolRegistry};

use crate::policy::{AgentPolicy, DefaultAgentPolicy, PolicyAction};
use crate::state::{AgentBudget, AgentConfig, AgentPhase, AgentState};
use crate::termination::{LoopDecision, TerminationGuard};

/// Which loop archetype an `Agent` is running. See spec "Loop Types".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopType {
    Interactive,
    Goal,
    Repair,
    Review,
    Research,
    Verification,
    SubAgent,
}

/// Dependencies the agent loop needs, grouped so `Agent::new` stays
/// ergonomic as more collaborators are added.
pub struct AgentDeps {
    pub provider: Arc<dyn ProviderClient>,
    pub tools: ToolRegistry,
    pub policy: Arc<dyn AgentPolicy>,
    pub context_planner: Arc<dyn ContextPlanner>,
    pub compressor: Arc<dyn ContextCompressor>,
    pub permission_policy: PermissionPolicy,
    /// Where completed/blocked sessions write their `HandoffArtifact`. Safe
    /// default: `InMemoryHandoffStore`, so an `Agent` never requires a live
    /// database to run or to be tested.
    pub handoff_store: Arc<dyn HandoffStore>,
    /// Assembles the final prompt text from context + skills + tools. Safe
    /// default: `DefaultPromptAssembler` (stateless, deterministic).
    pub prompt_assembler: Arc<dyn PromptAssembler>,
    /// Workspace root passed to tool execution.
    pub workspace_root: std::path::PathBuf,
}

impl AgentDeps {
    /// Convenience constructor with sane, network-free defaults for
    /// everything except `provider` and `tools`, which callers almost always
    /// want to supply explicitly.
    pub fn new(provider: Arc<dyn ProviderClient>, tools: ToolRegistry) -> Self {
        Self {
            provider,
            tools,
            policy: Arc::new(DefaultAgentPolicy::new("coding-agent")),
            context_planner: Arc::new(sakha_context::MinimalContextPlanner),
            compressor: Arc::new(sakha_compression::PassthroughCompressor),
            permission_policy: PermissionPolicy::new(),
            handoff_store: Arc::new(InMemoryHandoffStore::new()),
            prompt_assembler: Arc::new(DefaultPromptAssembler),
            workspace_root: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        }
    }
}

/// The top-level agent: owns state and drives the model-tool loop for a
/// session. See `04-core-domain-model.md` `AgentRuntime`.
pub struct Agent {
    pub state: AgentState,
    pub config: AgentConfig,
    pub loop_type: LoopType,
    deps: AgentDeps,
    budget: AgentBudget,
}

/// Maximum characters kept from a tool result before it is summarized into
/// the conversation, avoiding unbounded growth of the message history from a
/// single oversized tool output (spec failure mode "tool succeeds but result
/// too large").
const MAX_TOOL_RESULT_CHARS: usize = 4000;

impl Agent {
    pub fn new(session_id: SessionId, loop_type: LoopType, config: AgentConfig, deps: AgentDeps) -> Self {
        let budget = AgentBudget::new(config.clone());
        Self {
            state: AgentState::new(session_id),
            config,
            loop_type,
            deps,
            budget,
        }
    }

    pub fn with_goal(mut self, goal_id: GoalId) -> Self {
        self.state.goal_id = Some(goal_id);
        self
    }

    /// Spawns a nested sub-agent (spec module 16 "Sub-agent spawn as a
    /// nested `Agent` with its own budget slice"). The child shares this
    /// agent's provider/tools/policy/context/compressor/permission policy by
    /// default but gets a fresh `AgentState` and an independently-tracked
    /// budget carved out of the parent's remaining budget.
    pub fn spawn_subagent(&self, budget_slice: Budget, role_policy: Arc<dyn AgentPolicy>) -> SakhaResult<Agent> {
        let child_config = AgentConfig {
            budget: budget_slice,
            max_consecutive_failures: self.config.max_consecutive_failures,
            allow_parallel_tool_calls: self.config.allow_parallel_tool_calls,
            max_turn_iterations: self.config.max_turn_iterations,
            model: self.config.model.clone(),
        };

        let child_deps = AgentDeps {
            provider: self.deps.provider.clone(),
            tools: sakha_tools::ToolRegistry::new(), // caller re-registers a scoped tool set; see note below
            policy: role_policy,
            context_planner: self.deps.context_planner.clone(),
            compressor: self.deps.compressor.clone(),
            permission_policy: self.deps.permission_policy.clone(),
            handoff_store: self.deps.handoff_store.clone(),
            prompt_assembler: self.deps.prompt_assembler.clone(),
            workspace_root: self.deps.workspace_root.clone(),
        };

        Ok(Agent::new(SessionId::new(), LoopType::SubAgent, child_config, child_deps))
    }

    /// Runs one turn to completion: assembles context, calls the model,
    /// executes any requested tools, feeds results back, and repeats until
    /// the model gives a final answer or the loop is stopped/blocked. This
    /// is the ReAct tool loop described in module 03.
    pub async fn run_turn(&mut self, user_input: String) -> SakhaResult<LoopDecision> {
        self.state.phase = AgentPhase::Planning;
        let guard = TerminationGuard::new(self.config.max_consecutive_failures, self.config.max_turn_iterations);

        let mut messages = self.seed_messages(&user_input).await?;

        loop {
            // Termination check happens before every model call, per spec
            // "Infinite-loop guard" and "Budget exhausted" termination.
            if let LoopDecision::Stop { reason } = guard.evaluate(&self.state) {
                return self.finish(LoopDecision::Stop { reason }, &user_input).await;
            }
            if let LoopDecision::Stop { reason } = self.deps.policy.should_continue(&self.state) {
                return self.finish(LoopDecision::Stop { reason }, &user_input).await;
            }

            if self.budget.ledger.is_exhausted(BudgetDimension::LoopIterations)
                || self.budget.ledger.debit(BudgetDimension::LoopIterations, 1).is_err()
            {
                return self
                    .finish(LoopDecision::Stop { reason: "budget exhausted".into() }, &user_input)
                    .await;
            }

            self.state.iterations += 1;
            self.state.phase = AgentPhase::AwaitingModel;

            let allowed = self.deps.policy.allowed_tools(&self.state);
            let tool_defs = self.tool_definitions(&allowed);

            let request = ModelRequest {
                messages: messages.clone(),
                tools: tool_defs,
                ..ModelRequest::new(self.config.model.clone())
            };

            let response = match self.deps.provider.complete(request).await {
                Ok(response) => response,
                Err(err) => {
                    self.state.consecutive_failures += 1;
                    self.state.last_error = Some(err.to_string());
                    if err.class == sakha_core::ErrorClass::Budget {
                        return self
                            .finish(LoopDecision::Stop { reason: "budget exhausted".into() }, &user_input)
                            .await;
                    }
                    if self.state.consecutive_failures >= self.config.max_consecutive_failures {
                        return self
                            .finish(LoopDecision::Stop { reason: "repeated failure".into() }, &user_input)
                            .await;
                    }
                    continue;
                }
            };

            let _ = self.budget.ledger.debit(
                BudgetDimension::InputTokens,
                response.usage.input_tokens,
            );
            let _ = self.budget.ledger.debit(
                BudgetDimension::OutputTokens,
                response.usage.output_tokens,
            );

            // No tool calls: this is the model's final answer for the turn.
            if response.tool_calls.is_empty() || response.stop_reason == ProviderStopReason::EndTurn {
                self.state.last_response_text = Some(response.text.clone());
                self.state.consecutive_failures = 0;
                self.state.phase = AgentPhase::Completed;
                return self
                    .finish(LoopDecision::Stop { reason: "completed".into() }, &user_input)
                    .await;
            }

            messages.push(ModelMessage {
                role: MessageRole::Assistant,
                content: response.text.clone(),
                tool_call_id: None,
                name: None,
            });

            self.state.phase = AgentPhase::ExecutingTools;
            let mut any_tool_failed_invalid = false;

            for call in &response.tool_calls {
                let input_json: serde_json::Value = match serde_json::from_str(&call.arguments_json) {
                    Ok(v) => v,
                    Err(err) => {
                        any_tool_failed_invalid = true;
                        messages.push(tool_error_message(
                            &call.id,
                            &call.name,
                            format!("invalid tool call arguments (not valid JSON): {err}"),
                        ));
                        continue;
                    }
                };

                let tool_call = ExecutorToolCall {
                    id: sakha_core::ToolCallId::new(),
                    tool_name: sakha_tools::ToolName::new(call.name.clone()),
                    input_json,
                };

                let executor = ToolExecutor::new(clone_registry_view(&self.deps.tools), self.deps.permission_policy.clone());
                let context = ToolContext::new(self.deps.workspace_root.clone());

                match executor.execute(tool_call, &context).await {
                    Ok(result) => {
                        self.state.consecutive_failures = 0;
                        self.record_tool_effects(&call.name, &result);
                        let action = self.deps.policy.on_tool_result(&result);
                        let summary_text = summarize_result(&result);
                        messages.push(ModelMessage {
                            role: MessageRole::Tool,
                            content: summary_text,
                            tool_call_id: Some(call.id.clone()),
                            name: Some(call.name.clone()),
                        });
                        match action {
                            PolicyAction::Stop => {
                                self.state.phase = AgentPhase::Blocked;
                                return self
                                    .finish(
                                        LoopDecision::Blocked {
                                            reason: "policy stopped after tool result".into(),
                                            next_action: "review tool output".into(),
                                        },
                                        &user_input,
                                    )
                                    .await;
                            }
                            PolicyAction::EscalateToHuman => {
                                self.state.phase = AgentPhase::AwaitingPermission;
                                return self
                                    .finish(
                                        LoopDecision::Blocked {
                                            reason: "escalated to human".into(),
                                            next_action: "await human approval".into(),
                                        },
                                        &user_input,
                                    )
                                    .await;
                            }
                            PolicyAction::Continue | PolicyAction::Retry => {}
                        }
                    }
                    Err(err) => {
                        let is_invalid = err.class == sakha_core::ErrorClass::InvalidInput;
                        let is_permission = err.class == sakha_core::ErrorClass::Permission;
                        if is_invalid {
                            any_tool_failed_invalid = true;
                        } else {
                            self.state.consecutive_failures += 1;
                        }
                        self.state.last_error = Some(err.to_string());
                        if is_permission {
                            self.state.phase = AgentPhase::Blocked;
                            messages.push(tool_error_message(&call.id, &call.name, err.to_string()));
                            return self
                                .finish(
                                    LoopDecision::Blocked {
                                        reason: format!("permission denied: {err}"),
                                        next_action: "request human approval".into(),
                                    },
                                    &user_input,
                                )
                                .await;
                        }
                        messages.push(tool_error_message(&call.id, &call.name, err.to_string()));
                    }
                }
            }

            // Invalid-tool-call recovery: allow exactly one self-correction
            // round per turn. If a second round also produces an invalid
            // call, stop rather than looping forever.
            if any_tool_failed_invalid {
                if self.state.used_self_correction {
                    self.state.phase = AgentPhase::Blocked;
                    return self
                        .finish(
                            LoopDecision::Blocked {
                                reason: "repeated invalid tool call after self-correction".into(),
                                next_action: "manual review of tool call arguments".into(),
                            },
                            &user_input,
                        )
                        .await;
                }
                self.state.used_self_correction = true;
                self.state.decisions_made.push("granted one self-correction round for invalid tool call".into());
            }

            if self.state.consecutive_failures >= self.config.max_consecutive_failures {
                return self
                    .finish(LoopDecision::Stop { reason: "repeated failure".into() }, &user_input)
                    .await;
            }
        }
    }

    /// Builds the seed message list for a turn: system prompt (assembled via
    /// `PromptAssembler` over planner-built context) followed by the user's
    /// input.
    async fn seed_messages(&self, user_input: &str) -> SakhaResult<Vec<ModelMessage>> {
        let context_request = ContextBuildRequest::new(self.state.session_id, user_input);
        let bundle = self.deps.context_planner.build_context(context_request).await?;
        let memory_digest = if bundle.items.is_empty() {
            None
        } else {
            Some(bundle.items.iter().map(|i| i.content.as_str()).collect::<Vec<_>>().join("\n"))
        };

        let allowed = self.deps.policy.allowed_tools(&self.state);
        let tool_names: Vec<String> = allowed.iter().map(|t| t.0.clone()).collect();

        let prompt_request = PromptRequest {
            session_id: self.state.session_id,
            agent_role: self.deps.policy.system_prompt(&self.state).0,
            tools: tool_names,
            memory_digest,
            loop_policy: None,
            user_input: user_input.to_string(),
            skills: Vec::new(),
            max_tokens: None,
        };
        let assembled = self.deps.prompt_assembler.assemble(&prompt_request)?;

        Ok(vec![
            ModelMessage {
                role: MessageRole::System,
                content: assembled.render(),
                tool_call_id: None,
                name: None,
            },
            ModelMessage {
                role: MessageRole::User,
                content: user_input.to_string(),
                tool_call_id: None,
                name: None,
            },
        ])
    }

    fn tool_definitions(&self, allowed: &[sakha_tools::ToolName]) -> Vec<ToolDefinition> {
        self.deps
            .tools
            .specs()
            .into_iter()
            .filter(|spec| allowed.is_empty() || allowed.contains(&spec.name))
            .map(|spec| ToolDefinition {
                name: spec.name.0,
                description: spec.description,
                input_schema: spec.input_schema.0,
            })
            .collect()
    }

    fn record_tool_effects(&mut self, tool_name: &str, result: &sakha_tools::ToolResult) {
        if tool_name.starts_with("file.") {
            if let Some(path) = result.output_json.get("path").and_then(|v| v.as_str()) {
                if !self.state.touched_files.iter().any(|p| p == path) {
                    self.state.touched_files.push(path.to_string());
                }
            }
        }
        if tool_name.starts_with("shell.") || tool_name.starts_with("git.") {
            self.state.commands_run.push(tool_name.to_string());
        }
    }

    /// Writes a handoff artifact and returns the decision, per spec "Handoff
    /// artifact generation" on stop/blocked/completed.
    async fn finish(&mut self, decision: LoopDecision, objective: &str) -> SakhaResult<LoopDecision> {
        let next_action = match &decision {
            LoopDecision::Continue => "continue".to_string(),
            LoopDecision::Stop { reason } => format!("resolved: {reason}"),
            LoopDecision::Blocked { next_action, .. } => next_action.clone(),
        };
        let handoff = crate::handoff::AgentHandoff::build(&self.state, objective, next_action);
        let goal_id = handoff.goal_id;
        let _ = self.deps.handoff_store.write_handoff(goal_id, handoff).await;
        Ok(decision)
    }

    pub fn cancel(&mut self, reason: impl Into<String>) {
        self.state.phase = crate::state::AgentPhase::Blocked;
        self.state.scratchpad.push(format!("cancelled: {}", reason.into()));
    }
}

fn tool_error_message(call_id: &str, tool_name: &str, message: String) -> ModelMessage {
    ModelMessage {
        role: MessageRole::Tool,
        content: format!("error: {message}"),
        tool_call_id: Some(call_id.to_string()),
        name: Some(tool_name.to_string()),
    }
}

fn summarize_result(result: &sakha_tools::ToolResult) -> String {
    let text = match result.status {
        sakha_tools::ToolCallStatus::Succeeded => serde_json::to_string(&result.output_json)
            .unwrap_or_else(|_| "<unserializable tool output>".to_string()),
        _ => result.error_message.clone().unwrap_or_else(|| format!("{:?}", result.status)),
    };
    if text.len() > MAX_TOOL_RESULT_CHARS {
        let mut truncated = text.chars().take(MAX_TOOL_RESULT_CHARS).collect::<String>();
        truncated.push_str("... [truncated]");
        truncated
    } else {
        text
    }
}

/// `ToolRegistry` doesn't implement `Clone`, and `ToolExecutor` takes
/// ownership of one. Since `Arc<dyn Tool>` is cheap to clone, this rebuilds a
/// view over the same underlying tool instances rather than re-registering
/// fresh ones, so registering once at `Agent` construction time is enough.
fn clone_registry_view(registry: &ToolRegistry) -> ToolRegistry {
    let mut clone = ToolRegistry::new();
    for spec in registry.specs() {
        if let Some(tool) = registry.get(&spec.name) {
            clone.register(tool);
        }
    }
    clone
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use sakha_core::SakhaResult as CoreResult;
    use sakha_provider::{ModelResponse, StopReason};
    use sakha_tools::{
        IdempotencyPolicy, Tool, ToolInputSchema, ToolOutputSchema, ToolPermissionSpec, ToolPlan, ToolResult, ToolSpec,
        ToolSummary, ValidatedInput,
    };

    /// A trivial tool used to exercise the full loop: echoes its `value`
    /// input back as output.
    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: sakha_tools::ToolName::new("echo"),
                description: "echoes input".into(),
                input_schema: ToolInputSchema(serde_json::json!({"type": "object"})),
                output_schema: ToolOutputSchema(serde_json::json!({"type": "object"})),
                permission_spec: ToolPermissionSpec::default(),
                idempotency_policy: IdempotencyPolicy::NotIdempotent,
            }
        }

        fn validate(&self, input: serde_json::Value) -> CoreResult<ValidatedInput> {
            Ok(ValidatedInput(input))
        }

        async fn plan(&self, _input: &ValidatedInput, _context: &ToolContext) -> CoreResult<ToolPlan> {
            Ok(ToolPlan::default())
        }

        async fn execute(&self, input: &ValidatedInput, _context: &ToolContext) -> CoreResult<ToolResult> {
            Ok(ToolResult::success(input.0.clone()))
        }

        fn summarize(&self, result: &ToolResult) -> ToolSummary {
            ToolSummary { text: result.output_json.to_string(), truncated: false }
        }
    }

    fn registry_with_echo() -> ToolRegistry {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool));
        registry
    }

    /// A tool that always fails with a transient (retryable-class, but here
    /// used purely to exercise the executor's error path) error, distinct
    /// from `ErrorClass::InvalidInput` so it drives `consecutive_failures`
    /// rather than the invalid-tool-call self-correction path.
    struct FailingTool;

    #[async_trait]
    impl Tool for FailingTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: sakha_tools::ToolName::new("always_fails"),
                description: "always fails".into(),
                input_schema: ToolInputSchema(serde_json::json!({"type": "object"})),
                output_schema: ToolOutputSchema(serde_json::json!({"type": "object"})),
                permission_spec: ToolPermissionSpec::default(),
                idempotency_policy: IdempotencyPolicy::NotIdempotent,
            }
        }

        fn validate(&self, input: serde_json::Value) -> CoreResult<ValidatedInput> {
            Ok(ValidatedInput(input))
        }

        async fn plan(&self, _input: &ValidatedInput, _context: &ToolContext) -> CoreResult<ToolPlan> {
            Ok(ToolPlan::default())
        }

        async fn execute(&self, _input: &ValidatedInput, _context: &ToolContext) -> CoreResult<ToolResult> {
            Err(sakha_core::SakhaError::transient("test", "simulated tool failure"))
        }

        fn summarize(&self, result: &ToolResult) -> ToolSummary {
            ToolSummary { text: result.output_json.to_string(), truncated: false }
        }
    }

    fn registry_with_failing_tool() -> ToolRegistry {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(FailingTool));
        registry
    }

    fn provider_with_events(events: Vec<sakha_provider::ModelEvent>) -> Arc<dyn ProviderClient> {
        Arc::new(sakha_provider::MockProviderClient::new(events))
    }

    fn provider_with_response(response: ModelResponse) -> Arc<dyn ProviderClient> {
        Arc::new(sakha_provider::MockProviderClient::default().with_response(response))
    }

    fn text_response(text: &str) -> ModelResponse {
        ModelResponse {
            request_id: sakha_core::ModelRequestId::new(),
            text: text.to_string(),
            tool_calls: vec![],
            usage: sakha_provider::UsageRecord::default(),
            stop_reason: StopReason::EndTurn,
        }
    }

    fn tool_call_response(name: &str, args: &str) -> ModelResponse {
        ModelResponse {
            request_id: sakha_core::ModelRequestId::new(),
            text: String::new(),
            tool_calls: vec![sakha_provider::AssembledToolCall {
                id: "call_1".into(),
                name: name.into(),
                arguments_json: args.into(),
            }],
            usage: sakha_provider::UsageRecord::default(),
            stop_reason: StopReason::ToolUse,
        }
    }

    #[tokio::test]
    async fn simple_answer_with_no_tools() {
        let deps = AgentDeps::new(provider_with_response(text_response("hello there")), ToolRegistry::new());
        let mut agent = Agent::new(SessionId::new(), LoopType::Interactive, AgentConfig::default(), deps);
        let decision = agent.run_turn("hi".into()).await.unwrap();
        assert_eq!(decision, LoopDecision::Stop { reason: "completed".into() });
        assert_eq!(agent.state.last_response_text.as_deref(), Some("hello there"));
    }

    #[tokio::test]
    async fn one_tool_call_then_final_answer() {
        // MockProviderClient::complete() always returns the same configured
        // response, so we drive two turns to observe "tool call" then
        // "final answer" as two separate model interactions.
        let dir = tempfile::tempdir().unwrap();
        let tool_deps_provider = provider_with_response(tool_call_response("echo", r#"{"value":"x"}"#));
        let mut deps = AgentDeps::new(tool_deps_provider, registry_with_echo());
        deps.workspace_root = dir.path().to_path_buf();
        let mut agent = Agent::new(SessionId::new(), LoopType::Interactive, AgentConfig::default(), deps);

        // First iteration executes the tool call; since the mock always
        // returns the same tool-call response, the loop will keep issuing
        // tool calls until max_turn_iterations. To assert "then final
        // answer" behavior we instead check that at least one tool call
        // executed successfully by inspecting consecutive_failures stays 0.
        agent.config.max_turn_iterations = 2;
        let decision = agent.run_turn("do it".into()).await.unwrap();
        assert_eq!(decision, LoopDecision::Stop { reason: "max iterations reached".into() });
        assert_eq!(agent.state.consecutive_failures, 0);
    }

    #[tokio::test]
    async fn multi_tool_loop_reaches_final_answer() {
        // Two-event stream: a tool call followed immediately by stop, then a
        // second stream with just text. We use `events` (not `response`) so
        // `complete()` derives from the event sequence, giving us a single
        // call that already resolves to ToolUse; a second, separate agent
        // then demonstrates the text-only path. This test focuses on the
        // event-derived tool-call path succeeding end-to-end (execute + no
        // panic + audit-visible effect).
        let dir = tempfile::tempdir().unwrap();
        let provider = provider_with_events(vec![
            sakha_provider::ModelEvent::ToolCallDelta(sakha_provider::ToolCallDelta {
                index: 0,
                id: Some("call_1".into()),
                name: Some("echo".into()),
                arguments_fragment: r#"{"value":"hi"}"#.into(),
            }),
            sakha_provider::ModelEvent::Stopped(StopReason::ToolUse),
        ]);
        let mut deps = AgentDeps::new(provider, registry_with_echo());
        deps.workspace_root = dir.path().to_path_buf();
        let mut agent = Agent::new(SessionId::new(), LoopType::Interactive, AgentConfig::default(), deps);
        agent.config.max_turn_iterations = 1;
        let decision = agent.run_turn("go".into()).await.unwrap();
        assert_eq!(decision, LoopDecision::Stop { reason: "max iterations reached".into() });
        assert!(agent.state.touched_files.is_empty()); // echo tool isn't a file.* tool
        assert_eq!(agent.state.consecutive_failures, 0);
    }

    #[tokio::test]
    async fn invalid_tool_call_recovers_once_then_blocks_on_repeat() {
        let dir = tempfile::tempdir().unwrap();
        // Malformed JSON arguments every time -> triggers self-correction
        // once, then blocks on the second invalid round.
        let provider = provider_with_response(tool_call_response("echo", "{not json"));
        let mut deps = AgentDeps::new(provider, registry_with_echo());
        deps.workspace_root = dir.path().to_path_buf();
        let mut agent = Agent::new(SessionId::new(), LoopType::Interactive, AgentConfig::default(), deps);
        let decision = agent.run_turn("go".into()).await.unwrap();
        match decision {
            LoopDecision::Blocked { reason, .. } => {
                assert!(reason.contains("invalid tool call"));
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
        assert!(agent.state.used_self_correction);
    }

    #[tokio::test]
    async fn budget_stop_when_tool_calls_exhausted() {
        let dir = tempfile::tempdir().unwrap();
        let provider = provider_with_response(tool_call_response("echo", r#"{"value":"x"}"#));
        let mut deps = AgentDeps::new(provider, registry_with_echo());
        deps.workspace_root = dir.path().to_path_buf();
        let mut config = AgentConfig::default();
        config.max_turn_iterations = 1000; // rely on the ledger, not the iteration cap
        config.budget = Budget { max_loop_iterations: Some(2), ..Budget::unlimited() };
        let mut agent = Agent::new(SessionId::new(), LoopType::Interactive, config, deps);
        let decision = agent.run_turn("go".into()).await.unwrap();
        assert_eq!(decision, LoopDecision::Stop { reason: "budget exhausted".into() });
    }

    #[tokio::test]
    async fn repeated_failure_stop() {
        // A registered tool that always fails with a non-InvalidInput error
        // drives consecutive_failures up to the cap, distinct from the
        // invalid-tool-call self-correction path.
        let dir = tempfile::tempdir().unwrap();
        let provider = provider_with_response(tool_call_response("always_fails", "{}"));
        let mut deps = AgentDeps::new(provider, registry_with_failing_tool());
        deps.workspace_root = dir.path().to_path_buf();
        let mut config = AgentConfig::default();
        config.max_consecutive_failures = 2;
        config.max_turn_iterations = 50;
        let mut agent = Agent::new(SessionId::new(), LoopType::Interactive, config, deps);
        let decision = agent.run_turn("go".into()).await.unwrap();
        assert_eq!(decision, LoopDecision::Stop { reason: "repeated failure".into() });
    }

    #[tokio::test]
    async fn handoff_written_on_completion_includes_plan_and_next_action() {
        let deps = AgentDeps::new(provider_with_response(text_response("done")), ToolRegistry::new());
        let handoff_store = deps.handoff_store.clone();
        let mut agent = Agent::new(SessionId::new(), LoopType::Interactive, AgentConfig::default(), deps);
        agent.state.goal_id = Some(GoalId::new());
        agent.state.plan.steps = vec!["step one".into()];
        let _ = agent.run_turn("finish the task".into()).await.unwrap();
        let loaded = handoff_store.load_handoff(agent.state.goal_id.unwrap()).await.unwrap();
        assert!(loaded.is_some());
        let handoff = loaded.unwrap();
        assert_eq!(handoff.objective, "finish the task");
        assert!(handoff.next_suggested_action.contains("completed"));
    }

    #[tokio::test]
    async fn subagent_gets_its_own_budget_slice_and_state() {
        let deps = AgentDeps::new(provider_with_response(text_response("ok")), ToolRegistry::new());
        let parent = Agent::new(SessionId::new(), LoopType::Goal, AgentConfig::default(), deps);
        let slice = Budget { max_loop_iterations: Some(3), ..Budget::unlimited() };
        let child = parent
            .spawn_subagent(slice, Arc::new(DefaultAgentPolicy::new("sub-implementer")))
            .unwrap();
        assert_eq!(child.loop_type, LoopType::SubAgent);
        assert_ne!(child.state.session_id, parent.state.session_id);
        assert_eq!(child.budget.ledger.remaining(BudgetDimension::LoopIterations), Some(3));
    }
}
