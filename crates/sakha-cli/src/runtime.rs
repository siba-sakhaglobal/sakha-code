//! Shared wiring: builds a `ProviderClient` and the collaborator graph the
//! `run`/`chat` commands drive, from `SakhaConfig`. Centralized here so every
//! subcommand constructs the same real crates the same way (spec: "Wire
//! everything through the real crates; MockProvider selectable via config for
//! offline smoke test").

use std::sync::Arc;

use sakha_provider::{MockProviderClient, ModelEvent, OpenAiCompatibleClient, ProviderClient, StopReason};
use sakha_security::PermissionPolicy;
use sakha_tools::ToolRegistry;

use crate::config::{ProviderSelection, SakhaConfig};

/// Builds a `ProviderClient` per the configured `ProviderSelection`. Never
/// requires network access when `ProviderSelection::Mock` is selected (the
/// default), so the CLI always has a working offline smoke-test path.
pub fn build_provider(config: &SakhaConfig) -> Arc<dyn ProviderClient> {
    match config.provider.selection {
        ProviderSelection::Mock => Arc::new(MockProviderClient::default()),
        ProviderSelection::OpenAiCompatible => {
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

/// Builds the default tool registry, used by `run`/`chat` to describe
/// available tools to the model and (in future) execute them.
pub fn build_tool_registry() -> ToolRegistry {
    sakha_tools::default_registry()
}

/// Builds a permissive-by-default `PermissionPolicy` for local CLI use:
/// medium-and-below risk auto-allows, higher risk needs approval. Real
/// deployments layer workspace/project/org policy on top of this via
/// `PermissionPolicy::with_layer`.
pub fn build_permission_policy() -> PermissionPolicy {
    PermissionPolicy::new()
}

/// Drains a `ModelEventStream`, writing text deltas to `on_text` as they
/// arrive (streaming to stdout) and returning the full accumulated text plus
/// the final stop reason. Used by `run`/`chat` since `Agent::run_turn` is not
/// yet wired end-to-end (module 03 tool-loop wiring is still `not_implemented`
/// upstream in `sakha-agent`); this drives the real provider stream directly.
pub async fn stream_to_completion(
    provider: &dyn ProviderClient,
    request: sakha_provider::ModelRequest,
    mut on_text: impl FnMut(&str),
) -> sakha_core::SakhaResult<(String, StopReason)> {
    use tokio_stream::StreamExt;

    let mut stream = provider.stream(request).await?;
    let mut text = String::new();
    let mut stop_reason = StopReason::EndTurn;

    while let Some(event) = stream.next().await {
        match event? {
            ModelEvent::TextDelta { text: delta } => {
                on_text(&delta);
                text.push_str(&delta);
            }
            ModelEvent::ToolCallDelta(_) => {
                // Tool-call execution loop is out of scope until
                // `sakha-agent::Agent::run_turn` lands; deltas are ignored
                // here rather than silently dropped-and-forgotten by a
                // stub loop.
            }
            ModelEvent::UsageUpdate(_) => {}
            ModelEvent::Stopped(reason) => stop_reason = reason,
            ModelEvent::Error { message } => {
                return Err(sakha_core::SakhaError::transient("sakha-cli", message));
            }
        }
    }

    Ok((text, stop_reason))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn build_provider_mock_streams_without_network() {
        let config = SakhaConfig::default();
        let provider = build_provider(&config);
        let health = provider.health().await.unwrap();
        assert_eq!(health, sakha_provider::ProviderHealth::Healthy);
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
}
