//! Shared wiring: builds a `ProviderClient` and the collaborator graph the
//! `run`/`chat` commands drive, from `SakhaConfig`. Centralized here so every
//! subcommand constructs the same real crates the same way (spec: "Wire
//! everything through the real crates; MockProvider selectable via config for
//! offline smoke test").

use std::sync::Arc;

use sakha_memory::{Database, InMemorySessionStore, SessionStore, SqliteSessionStore};
use sakha_provider::{
    MessageRole, MockProviderClient, ModelEvent, ModelMessage, ModelRequest, OpenAiCompatibleClient, ProviderClient, StopReason,
    ToolDefinition,
};
use sakha_security::PermissionPolicy;
use sakha_tools::{ToolContext, ToolName, ToolRegistry};

use crate::config::{ProviderSelection, SakhaConfig};
use crate::secrets::{KeyringSecretBackend, SecretBackend};

/// Builds a `ProviderClient` per the configured `ProviderSelection`. Never
/// requires network access when `ProviderSelection::Mock` is selected (the
/// default), so the CLI always has a working offline smoke-test path.
///
/// Key resolution precedence for `ProviderSelection::OpenAiCompatible`, per
/// spec `modules/17-config-secrets-policy.md` "keys live in the OS
/// credential manager or the environment, never in the config file":
///   1. If `config.provider.api_key_env` names an environment variable that
///      is already set in this process, leave it untouched — the user (or
///      their shell/.env) is the source of truth and `OpenAiCompatibleClient`
///      will read it directly.
///   2. Otherwise, if `config.provider.preset` identifies a catalog preset
///      and the OS keyring has a secret stored for it (via `sakha login`),
///      copy that secret into the env var named by `api_key_env` for this
///      process only (`std::env::set_var`) before building the profile. This
///      never touches disk and does not leak the key to child processes'
///      config files.
pub fn build_provider(config: &SakhaConfig) -> Arc<dyn ProviderClient> {
    match config.provider.selection {
        ProviderSelection::Mock => Arc::new(MockProviderClient::default()),
        ProviderSelection::OpenAiCompatible => {
            resolve_key_into_env(config, &KeyringSecretBackend::new());
            let mut profile = sakha_provider::ProviderProfile::openai_compatible(
                "configured",
                &config.provider.base_url,
                &config.provider.model,
            );
            profile.api_key_ref = config.provider.api_key_env.clone();
            Arc::new(OpenAiCompatibleClient::new(profile))
        }
    }
}

/// Implements the precedence documented on `build_provider`: env var already
/// set wins; otherwise falls back to the keyring entry for
/// `config.provider.preset`, setting it into the process environment.
/// Split out from `build_provider` so tests can exercise the precedence
/// logic against an injected `SecretBackend` without touching the real OS
/// keyring.
fn resolve_key_into_env(config: &SakhaConfig, backend: &dyn SecretBackend) {
    let Some(env_name) = &config.provider.api_key_env else { return };
    if std::env::var_os(env_name).is_some() {
        // (1) Existing env var always wins — do nothing.
        return;
    }
    let Some(preset_id) = &config.provider.preset else { return };
    if let Ok(Some(secret)) = backend.get(preset_id) {
        // (2) Fall back to the keyring-stored secret for the configured
        // preset, process-local only.
        std::env::set_var(env_name, secret);
    }
}

/// Builds the default tool registry, used by `run`/`chat` to describe
/// available tools to the model and (in future) execute them.
pub fn build_tool_registry() -> ToolRegistry {
    sakha_tools::default_registry()
}

/// Builds the `SessionStore` `run`/`chat`/`session`/`loops` commands share:
/// SQLite-backed (durable across CLI invocations, so `sakha session list`
/// and `sakha chat --session-id <id>` resume actually work) when
/// `config.db_path` is set, in-memory otherwise. Every subcommand invocation
/// calls this with the same `config`, so a configured `db_path` gives a
/// consistent, persistent view of sessions across process runs — the root
/// fix for "session list always empty" / "chat --session-id never resumes"
/// (spec `modules/13-ui-cli-tui-web-desktop.md` "CLI Commands").
pub fn build_session_store(config: &SakhaConfig) -> sakha_core::SakhaResult<Arc<dyn SessionStore>> {
    match &config.db_path {
        Some(path) => {
            let db = Database::open(path)?;
            Ok(Arc::new(SqliteSessionStore::new(Arc::new(db))))
        }
        None => Ok(Arc::new(InMemorySessionStore::new())),
    }
}

/// Builds a permissive-by-default `PermissionPolicy` for local CLI use:
/// medium-and-below risk auto-allows, higher risk needs approval. Real
/// deployments layer workspace/project/org policy on top of this via
/// `PermissionPolicy::with_layer`.
pub fn build_permission_policy() -> PermissionPolicy {
    PermissionPolicy::new()
}

/// Drains a `ModelEventStream`, writing text deltas to `on_text` as they
/// arrive (streaming to stdout) and returning the full accumulated text and
/// the final stop reason. A thin convenience wrapper over
/// `stream_to_completion_with_tool_calls` for callers that only want text
/// (e.g. a prompt known not to need tools); `run`/`chat` use
/// `run_agent_turn` instead, which drives tool calls to completion.
///
/// `ToolCallDelta` events are assembled via `ToolCallAssembler` rather than
/// discarded: a full multi-turn agent loop (`sakha-agent::Agent::run_turn`)
/// is out of scope here, but *capturing* what the model asked to call is not
/// — `run_agent_turn` below uses `stream_to_completion_with_tool_calls`
/// directly to actually execute those calls against a `ToolRegistry` and
/// feed results back to the model (spec Module 03 tool-loop, CLI Commands).
#[allow(dead_code)]
pub async fn stream_to_completion(
    provider: &dyn ProviderClient,
    request: sakha_provider::ModelRequest,
    on_text: impl FnMut(&str),
) -> sakha_core::SakhaResult<(String, StopReason)> {
    let (text, _tool_calls, stop_reason) = stream_to_completion_with_tool_calls(provider, request, on_text).await?;
    Ok((text, stop_reason))
}

/// Like `stream_to_completion`, but also returns the model's assembled tool
/// calls (empty if it asked for none).
pub async fn stream_to_completion_with_tool_calls(
    provider: &dyn ProviderClient,
    request: sakha_provider::ModelRequest,
    mut on_text: impl FnMut(&str),
) -> sakha_core::SakhaResult<(String, Vec<sakha_provider::AssembledToolCall>, StopReason)> {
    use tokio_stream::StreamExt;

    let mut stream = provider.stream(request).await?;
    let mut text = String::new();
    let mut assembler = sakha_provider::ToolCallAssembler::new();
    let mut stop_reason = StopReason::EndTurn;

    while let Some(event) = stream.next().await {
        match event? {
            ModelEvent::TextDelta { text: delta } => {
                on_text(&delta);
                text.push_str(&delta);
            }
            ModelEvent::ToolCallDelta(delta) => assembler.push(delta),
            ModelEvent::UsageUpdate(_) => {}
            ModelEvent::Stopped(reason) => stop_reason = reason,
            ModelEvent::Error { message } => {
                return Err(sakha_core::SakhaError::transient("sakha-cli", message));
            }
        }
    }

    Ok((text, assembler.finish(), stop_reason))
}

/// Maximum number of tool-call rounds `run_agent_turn` will drive in a single
/// turn before giving up and returning control to the caller, per spec
/// "Infinite Loop Prevention" applied at the single-turn tool-loop level (a
/// model that keeps requesting tools forever must not hang the CLI).
const MAX_TOOL_ROUNDS: u32 = 25;

/// Drives one full agent turn: streams the model's response, and whenever it
/// requests tool calls, executes each against `registry` and feeds the
/// results back as `Tool`-role messages, repeating until the model stops
/// asking for tools (or `MAX_TOOL_ROUNDS` is hit). This is the "tool
/// invocation" half of module 03's tool-loop that `sakha-agent::Agent` will
/// eventually own end-to-end; `run`/`chat` use this directly in the
/// meantime so tool calls are not silently dropped (spec "CLI Commands",
/// Module 03 tool-loop).
///
/// `on_text` receives text deltas as they stream in, across every round.
/// `on_tool_call` is notified (name, input, whether it succeeded) after each
/// tool executes, so callers can render progress.
pub async fn run_agent_turn(
    provider: &dyn ProviderClient,
    registry: &ToolRegistry,
    mut request: ModelRequest,
    mut on_text: impl FnMut(&str),
    mut on_tool_call: impl FnMut(&str, bool),
) -> sakha_core::SakhaResult<(String, StopReason)> {
    if request.tools.is_empty() {
        request.tools = registry
            .specs()
            .into_iter()
            .map(|spec| ToolDefinition { name: spec.name.0, description: spec.description, input_schema: spec.input_schema.0 })
            .collect();
    }

    let mut final_text = String::new();
    let mut final_stop = StopReason::EndTurn;

    for _round in 0..MAX_TOOL_ROUNDS {
        let (text, tool_calls, stop_reason) =
            stream_to_completion_with_tool_calls(provider, request.clone(), &mut on_text).await?;
        final_text = text.clone();
        final_stop = stop_reason;

        if tool_calls.is_empty() {
            break;
        }

        // Record the assistant's turn (including the tool-call text, if any)
        // before appending tool results, so the next round's context is
        // faithful to what the model actually said/asked for.
        request.messages.push(ModelMessage { role: MessageRole::Assistant, content: text, tool_call_id: None, name: None });

        let context = ToolContext::new(".");
        for call in &tool_calls {
            let arguments: serde_json::Value = serde_json::from_str(&call.arguments_json).unwrap_or(serde_json::Value::Null);
            let (output_text, succeeded) = match registry.get_or_err(&ToolName::new(call.name.clone())) {
                Ok(tool) => match tool.validate(arguments) {
                    Ok(validated) => match tool.execute(&validated, &context).await {
                        Ok(result) => (serde_json::to_string(&result.output_json).unwrap_or_default(), true),
                        Err(err) => (format!("tool error: {err}"), false),
                    },
                    Err(err) => (format!("invalid tool input: {err}"), false),
                },
                Err(err) => (format!("unknown tool: {err}"), false),
            };
            on_tool_call(&call.name, succeeded);
            request.messages.push(ModelMessage {
                role: MessageRole::Tool,
                content: output_text,
                tool_call_id: Some(call.id.clone()),
                name: Some(call.name.clone()),
            });
        }
        // Tool results were appended to `request.messages`; loop back around
        // to re-query the model with them. `MAX_TOOL_ROUNDS` bounds how many
        // times this can happen for one turn.
    }

    Ok((final_text, final_stop))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::test_support::InMemorySecretBackend;
    use std::sync::{Mutex, OnceLock};

    /// `resolve_key_into_env` reads/writes process-global env vars; this
    /// lock serializes the tests below against each other (mirrors
    /// `test_support::TempHome`'s home-dir lock) so they can't race.
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap_or_else(|p| p.into_inner())
    }

    #[tokio::test]
    async fn build_provider_mock_streams_without_network() {
        let config = SakhaConfig::default();
        let provider = build_provider(&config);
        let health = provider.health().await.unwrap();
        assert_eq!(health, sakha_provider::ProviderHealth::Healthy);
    }

    /// (1) An already-set env var wins over any keyring-stored secret.
    #[test]
    fn resolve_key_into_env_prefers_existing_env_var() {
        let _lock = env_lock();
        let env_name = "SAKHA_TEST_PRECEDENCE_ENV_WINS";
        std::env::set_var(env_name, "from-env");

        let mut config = SakhaConfig::default();
        config.provider.api_key_env = Some(env_name.to_string());
        config.provider.preset = Some("openai".to_string());

        let backend = InMemorySecretBackend::new();
        backend.set("openai", "from-keyring").unwrap();

        resolve_key_into_env(&config, &backend);
        assert_eq!(std::env::var(env_name).unwrap(), "from-env");
        std::env::remove_var(env_name);
    }

    /// (2) Falls back to the keyring secret for `config.provider.preset` when
    /// the env var is unset.
    #[test]
    fn resolve_key_into_env_falls_back_to_keyring_when_env_unset() {
        let _lock = env_lock();
        let env_name = "SAKHA_TEST_PRECEDENCE_KEYRING_FALLBACK";
        std::env::remove_var(env_name);

        let mut config = SakhaConfig::default();
        config.provider.api_key_env = Some(env_name.to_string());
        config.provider.preset = Some("openai".to_string());

        let backend = InMemorySecretBackend::new();
        backend.set("openai", "from-keyring").unwrap();

        resolve_key_into_env(&config, &backend);
        assert_eq!(std::env::var(env_name).unwrap(), "from-keyring");
        std::env::remove_var(env_name);
    }

    /// Neither env var nor preset/keyring secret present: nothing is set,
    /// and no error occurs.
    #[test]
    fn resolve_key_into_env_no_op_when_nothing_available() {
        let _lock = env_lock();
        let env_name = "SAKHA_TEST_PRECEDENCE_NOTHING_AVAILABLE";
        std::env::remove_var(env_name);

        let mut config = SakhaConfig::default();
        config.provider.api_key_env = Some(env_name.to_string());
        config.provider.preset = None;

        let backend = InMemorySecretBackend::new();
        resolve_key_into_env(&config, &backend);
        assert!(std::env::var(env_name).is_err());
    }

    #[tokio::test]
    async fn stream_to_completion_accumulates_text_deltas() {
        let provider = MockProviderClient::new(vec![
            ModelEvent::TextDelta { text: "hel".into() },
            ModelEvent::TextDelta { text: "lo".into() },
            ModelEvent::Stopped(StopReason::EndTurn),
        ]);
        let mut chunks = Vec::new();
        let (text, stop) = stream_to_completion(&provider, sakha_provider::ModelRequest::new("mock"), |chunk| {
            chunks.push(chunk.to_string());
        })
        .await
        .unwrap();
        assert_eq!(text, "hello");
        assert_eq!(stop, StopReason::EndTurn);
        assert_eq!(chunks, vec!["hel".to_string(), "lo".to_string()]);
    }

    #[test]
    fn build_tool_registry_contains_builtins() {
        let registry = build_tool_registry();
        assert!(registry.get(&sakha_tools::ToolName::new("file.read")).is_some());
    }

    /// A `ProviderClient` test double that plays a different scripted
    /// response on each successive `stream()` call, so tests can simulate
    /// "round 1: model asks for a tool call" -> "round 2: model replies with
    /// text now that it has the tool result" — `MockProviderClient` alone
    /// can't do this since it replays the same fixed events every call.
    struct ScriptedRoundsProvider {
        rounds: std::sync::Mutex<std::collections::VecDeque<Vec<ModelEvent>>>,
        capabilities: sakha_provider::ProviderCapabilities,
    }

    impl ScriptedRoundsProvider {
        fn new(rounds: Vec<Vec<ModelEvent>>) -> Self {
            Self {
                rounds: std::sync::Mutex::new(rounds.into_iter().collect()),
                capabilities: sakha_provider::ProviderCapabilities {
                    supports_tools: true,
                    supports_json_schema: true,
                    supports_streaming: true,
                    supports_streaming_usage: true,
                    supports_reasoning_effort: false,
                    supports_parallel_tool_calls: true,
                    max_context_tokens: None,
                    max_output_tokens: None,
                },
            }
        }
    }

    #[async_trait::async_trait]
    impl sakha_provider::ProviderClient for ScriptedRoundsProvider {
        async fn stream(&self, _request: ModelRequest) -> sakha_core::SakhaResult<sakha_provider::ModelEventStream> {
            let events = self.rounds.lock().unwrap().pop_front().unwrap_or_else(|| vec![ModelEvent::Stopped(StopReason::EndTurn)]);
            let (tx, rx) = tokio::sync::mpsc::channel(events.len().max(1));
            for event in events {
                let _ = tx.send(Ok(event)).await;
            }
            Ok(tokio_stream::wrappers::ReceiverStream::new(rx))
        }

        async fn complete(&self, _request: ModelRequest) -> sakha_core::SakhaResult<sakha_provider::ModelResponse> {
            unimplemented!("not exercised by run_agent_turn tests")
        }

        async fn count_tokens(
            &self,
            _request: sakha_provider::TokenCountRequest,
        ) -> sakha_core::SakhaResult<sakha_provider::TokenCountResult> {
            Ok(sakha_provider::TokenCountResult::default())
        }

        async fn health(&self) -> sakha_core::SakhaResult<sakha_provider::ProviderHealth> {
            Ok(sakha_provider::ProviderHealth::Healthy)
        }

        fn capabilities(&self) -> sakha_provider::ProviderCapabilities {
            self.capabilities.clone()
        }
    }

    /// End-to-end proof that a `ToolCallDelta` the model streams actually
    /// gets executed against the `ToolRegistry` and fed back, rather than
    /// silently ignored (spec Module 03 tool-loop, "CLI Commands"): round 1
    /// asks for `file.write`, round 2 (after seeing the tool result) replies
    /// with plain text.
    #[tokio::test]
    async fn run_agent_turn_executes_requested_tool_call_and_continues() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.txt");

        let provider = ScriptedRoundsProvider::new(vec![
            vec![
                ModelEvent::ToolCallDelta(sakha_provider::ToolCallDelta {
                    index: 0,
                    id: Some("call_1".into()),
                    name: Some("file.write".into()),
                    arguments_fragment: serde_json::json!({"path": target.to_string_lossy(), "content": "hello from tool"}).to_string(),
                }),
                ModelEvent::Stopped(StopReason::ToolUse),
            ],
            vec![ModelEvent::TextDelta { text: "done writing the file".into() }, ModelEvent::Stopped(StopReason::EndTurn)],
        ]);

        let registry = build_tool_registry();
        let mut text_chunks = Vec::new();
        let mut tool_calls_seen = Vec::new();
        let (final_text, stop_reason) = run_agent_turn(
            &provider,
            &registry,
            ModelRequest::new("scripted"),
            |chunk| text_chunks.push(chunk.to_string()),
            |name, succeeded| tool_calls_seen.push((name.to_string(), succeeded)),
        )
        .await
        .unwrap();

        assert_eq!(stop_reason, StopReason::EndTurn);
        assert_eq!(final_text, "done writing the file");
        assert_eq!(tool_calls_seen, vec![("file.write".to_string(), true)]);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "hello from tool");
    }

    /// A model that keeps requesting tool calls forever must not hang the
    /// CLI indefinitely (spec "Infinite Loop Prevention" applied to the
    /// single-turn tool loop): `run_agent_turn` bails out after
    /// `MAX_TOOL_ROUNDS` rounds rather than looping forever.
    #[tokio::test]
    async fn run_agent_turn_bounds_rounds_when_model_never_stops_calling_tools() {
        let always_calls_tool = vec![
            ModelEvent::ToolCallDelta(sakha_provider::ToolCallDelta {
                index: 0,
                id: Some("call_x".into()),
                name: Some("git.status".into()),
                arguments_fragment: "{}".into(),
            }),
            ModelEvent::Stopped(StopReason::ToolUse),
        ];
        let rounds = (0..(MAX_TOOL_ROUNDS + 5)).map(|_| always_calls_tool.clone()).collect();
        let provider = ScriptedRoundsProvider::new(rounds);
        let registry = build_tool_registry();

        let mut round_count = 0u32;
        let result = run_agent_turn(
            &provider,
            &registry,
            ModelRequest::new("scripted"),
            |_chunk| {},
            |_name, _succeeded| round_count += 1,
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(round_count, MAX_TOOL_ROUNDS);
    }
}
