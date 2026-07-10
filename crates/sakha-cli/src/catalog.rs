//! Static provider catalog: well-known LLM providers/gateways with their
//! canonical base URL, default model, and the environment variable each
//! expects its API key under. Drives `sakha login`, `sakha providers
//! catalog`, and `sakha providers use`. See spec `modules/17-config-secrets-policy.md`
//! and `modules/02-llm-provider-gateway.md` "Provider Profiles".

use serde::Serialize;

/// Broad category a preset falls into, mostly informational (shown in
/// `sakha providers catalog`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// A model vendor's own first-party API (OpenAI, Anthropic, ...).
    Direct,
    /// A multi-model routing/aggregation service (OpenRouter, Together, ...).
    Aggregator,
    /// A locally-run inference server (Ollama, LM Studio).
    Local,
    /// A self-hosted or managed proxy/gateway in front of other providers.
    Gateway,
}

/// One entry in the static provider catalog.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderPreset {
    /// Stable identifier, e.g. `"openai"`. Matched case-insensitively by
    /// `find`.
    pub id: &'static str,
    pub display_name: &'static str,
    pub kind: ProviderKind,
    pub base_url: &'static str,
    /// Default model to configure when this preset is selected. Empty string
    /// for presets (e.g. self-hosted gateways) where no sensible default
    /// exists and the user is expected to supply one.
    pub default_model: &'static str,
    /// Canonical environment variable name this provider's API key is read
    /// from. `None` for providers that take no key (local gateways).
    pub api_key_env: Option<&'static str>,
    /// Where a human can go to mint an API key for this provider.
    pub keys_console_url: Option<&'static str>,
    /// Whether `sakha login <id>` (with no `--api-key`) should run the OAuth
    /// PKCE device-style flow instead of prompting for a pasted key.
    pub supports_oauth_pkce: bool,
    /// Freeform notes shown in `sakha providers catalog`.
    pub notes: Option<&'static str>,
}

/// The full static provider catalog, in the order presets should be listed.
pub fn all() -> &'static [ProviderPreset] {
    &CATALOG
}

/// Looks up a preset by id, case-insensitively.
pub fn find(id: &str) -> Option<&'static ProviderPreset> {
    CATALOG.iter().find(|p| p.id.eq_ignore_ascii_case(id))
}

static CATALOG: [ProviderPreset; 19] = [
    ProviderPreset {
        id: "openai",
        display_name: "OpenAI",
        kind: ProviderKind::Direct,
        base_url: "https://api.openai.com/v1",
        default_model: "gpt-5-mini",
        api_key_env: Some("OPENAI_API_KEY"),
        keys_console_url: Some("https://platform.openai.com/api-keys"),
        supports_oauth_pkce: false,
        notes: None,
    },
    ProviderPreset {
        id: "anthropic",
        display_name: "Anthropic",
        kind: ProviderKind::Direct,
        base_url: "https://api.anthropic.com/v1",
        default_model: "claude-sonnet-5",
        api_key_env: Some("ANTHROPIC_API_KEY"),
        keys_console_url: Some("https://console.anthropic.com/settings/keys"),
        supports_oauth_pkce: false,
        notes: Some("OpenAI-compat endpoint"),
    },
    ProviderPreset {
        id: "gemini",
        display_name: "Google Gemini",
        kind: ProviderKind::Direct,
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        default_model: "gemini-3.1-flash-lite",
        api_key_env: Some("GEMINI_API_KEY"),
        keys_console_url: Some("https://aistudio.google.com/apikey"),
        supports_oauth_pkce: false,
        notes: None,
    },
    ProviderPreset {
        id: "xai",
        display_name: "xAI Grok",
        kind: ProviderKind::Direct,
        base_url: "https://api.x.ai/v1",
        default_model: "grok-4-fast",
        api_key_env: Some("XAI_API_KEY"),
        keys_console_url: Some("https://console.x.ai"),
        supports_oauth_pkce: false,
        notes: None,
    },
    ProviderPreset {
        id: "mistral",
        display_name: "Mistral",
        kind: ProviderKind::Direct,
        base_url: "https://api.mistral.ai/v1",
        default_model: "mistral-small-latest",
        api_key_env: Some("MISTRAL_API_KEY"),
        keys_console_url: Some("https://console.mistral.ai/api-keys"),
        supports_oauth_pkce: false,
        notes: None,
    },
    ProviderPreset {
        id: "deepseek",
        display_name: "DeepSeek",
        kind: ProviderKind::Direct,
        base_url: "https://api.deepseek.com/v1",
        default_model: "deepseek-chat",
        api_key_env: Some("DEEPSEEK_API_KEY"),
        keys_console_url: Some("https://platform.deepseek.com/api_keys"),
        supports_oauth_pkce: false,
        notes: None,
    },
    ProviderPreset {
        id: "groq",
        display_name: "Groq",
        kind: ProviderKind::Direct,
        base_url: "https://api.groq.com/openai/v1",
        default_model: "llama-3.3-70b-versatile",
        api_key_env: Some("GROQ_API_KEY"),
        keys_console_url: Some("https://console.groq.com/keys"),
        supports_oauth_pkce: false,
        notes: None,
    },
    ProviderPreset {
        id: "cerebras",
        display_name: "Cerebras",
        kind: ProviderKind::Direct,
        base_url: "https://api.cerebras.ai/v1",
        default_model: "llama-3.3-70b",
        api_key_env: Some("CEREBRAS_API_KEY"),
        keys_console_url: Some("https://cloud.cerebras.ai"),
        supports_oauth_pkce: false,
        notes: None,
    },
    ProviderPreset {
        id: "together",
        display_name: "Together AI",
        kind: ProviderKind::Aggregator,
        base_url: "https://api.together.xyz/v1",
        default_model: "meta-llama/Llama-3.3-70B-Instruct-Turbo",
        api_key_env: Some("TOGETHER_API_KEY"),
        keys_console_url: Some("https://api.together.ai/settings/api-keys"),
        supports_oauth_pkce: false,
        notes: None,
    },
    ProviderPreset {
        id: "fireworks",
        display_name: "Fireworks AI",
        kind: ProviderKind::Aggregator,
        base_url: "https://api.fireworks.ai/inference/v1",
        default_model: "accounts/fireworks/models/llama-v3p3-70b-instruct",
        api_key_env: Some("FIREWORKS_API_KEY"),
        keys_console_url: Some("https://fireworks.ai/account/api-keys"),
        supports_oauth_pkce: false,
        notes: None,
    },
    ProviderPreset {
        id: "deepinfra",
        display_name: "DeepInfra",
        kind: ProviderKind::Aggregator,
        base_url: "https://api.deepinfra.com/v1/openai",
        default_model: "meta-llama/Llama-3.3-70B-Instruct",
        api_key_env: Some("DEEPINFRA_API_KEY"),
        keys_console_url: Some("https://deepinfra.com/dash/api_keys"),
        supports_oauth_pkce: false,
        notes: None,
    },
    ProviderPreset {
        id: "moonshot",
        display_name: "Moonshot Kimi",
        kind: ProviderKind::Direct,
        base_url: "https://api.moonshot.ai/v1",
        default_model: "kimi-k2-turbo-preview",
        api_key_env: Some("MOONSHOT_API_KEY"),
        keys_console_url: Some("https://platform.moonshot.ai/console/api-keys"),
        supports_oauth_pkce: false,
        notes: None,
    },
    ProviderPreset {
        id: "openrouter",
        display_name: "OpenRouter",
        kind: ProviderKind::Aggregator,
        base_url: "https://openrouter.ai/api/v1",
        default_model: "openrouter/auto",
        api_key_env: Some("OPENROUTER_API_KEY"),
        keys_console_url: Some("https://openrouter.ai/settings/keys"),
        supports_oauth_pkce: true,
        notes: None,
    },
    ProviderPreset {
        id: "vercel-ai-gateway",
        display_name: "Vercel AI Gateway",
        kind: ProviderKind::Gateway,
        base_url: "https://ai-gateway.vercel.sh/v1",
        default_model: "openai/gpt-5-mini",
        api_key_env: Some("AI_GATEWAY_API_KEY"),
        keys_console_url: Some("https://vercel.com/dashboard/ai-gateway"),
        supports_oauth_pkce: false,
        notes: None,
    },
    ProviderPreset {
        id: "portkey",
        display_name: "Portkey",
        kind: ProviderKind::Gateway,
        base_url: "https://api.portkey.ai/v1",
        default_model: "gpt-5-mini",
        api_key_env: Some("PORTKEY_API_KEY"),
        keys_console_url: Some("https://app.portkey.ai"),
        supports_oauth_pkce: false,
        notes: None,
    },
    ProviderPreset {
        id: "requesty",
        display_name: "Requesty",
        kind: ProviderKind::Aggregator,
        base_url: "https://router.requesty.ai/v1",
        default_model: "openai/gpt-5-mini",
        api_key_env: Some("REQUESTY_API_KEY"),
        keys_console_url: Some("https://app.requesty.ai/api-keys"),
        supports_oauth_pkce: false,
        notes: None,
    },
    ProviderPreset {
        id: "litellm",
        display_name: "LiteLLM (self-hosted)",
        kind: ProviderKind::Gateway,
        base_url: "http://localhost:4000",
        default_model: "",
        api_key_env: Some("LITELLM_API_KEY"),
        keys_console_url: None,
        supports_oauth_pkce: false,
        notes: Some("set your own base_url with --base-url"),
    },
    ProviderPreset {
        id: "ollama",
        display_name: "Ollama (local)",
        kind: ProviderKind::Local,
        base_url: "http://localhost:11434/v1",
        default_model: "llama3.3",
        api_key_env: None,
        keys_console_url: None,
        supports_oauth_pkce: false,
        notes: None,
    },
    ProviderPreset {
        id: "lmstudio",
        display_name: "LM Studio (local)",
        kind: ProviderKind::Local,
        base_url: "http://localhost:1234/v1",
        default_model: "local-model",
        api_key_env: None,
        keys_console_url: None,
        supports_oauth_pkce: false,
        notes: None,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn all_ids_are_unique() {
        let ids: HashSet<&str> = all().iter().map(|p| p.id).collect();
        assert_eq!(ids.len(), all().len(), "duplicate preset id found");
    }

    #[test]
    fn remote_base_urls_use_https() {
        for preset in all() {
            if preset.kind == ProviderKind::Local || preset.base_url.contains("localhost") {
                continue;
            }
            assert!(
                preset.base_url.starts_with("https://"),
                "preset {} has non-https base_url: {}",
                preset.id,
                preset.base_url
            );
        }
    }

    #[test]
    fn every_non_local_preset_has_api_key_env() {
        for preset in all() {
            if preset.kind == ProviderKind::Local {
                continue;
            }
            assert!(preset.api_key_env.is_some(), "preset {} is missing api_key_env", preset.id);
        }
    }

    #[test]
    fn find_is_case_insensitive() {
        assert!(find("OpenAI").is_some());
        assert!(find("openai").is_some());
        assert!(find("OPENROUTER").is_some());
        assert!(find("does-not-exist").is_none());
    }

    #[test]
    fn openrouter_supports_oauth_pkce() {
        let preset = find("openrouter").unwrap();
        assert!(preset.supports_oauth_pkce);
    }

    #[test]
    fn local_presets_have_no_api_key_env() {
        assert!(find("ollama").unwrap().api_key_env.is_none());
        assert!(find("lmstudio").unwrap().api_key_env.is_none());
    }
}
