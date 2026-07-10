//! Provider profile configuration: credentials, base URLs, and per-provider
//! capability overrides. See spec `modules/02-llm-provider-gateway.md`
//! "Provider Profiles".

use serde::{Deserialize, Serialize};

use sakha_core::{ProviderProfileId, SakhaError, SakhaResult};

/// Which wire protocol family a provider profile speaks. All current
/// providers (OpenAI, Gemini-via-OpenAI-compat, xAI/Grok, local gateways)
/// use the OpenAI-compatible family; kept as an enum for future providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    OpenAiCompatible,
}

/// A configured provider profile: credentials reference, base URL, model
/// selection, and capability overrides. See spec "OpenAI Compatible" fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderProfile {
    pub id: ProviderProfileId,
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    /// Reference to a secret (resolved via `sakha-security::SecretStore`),
    /// never the raw key.
    pub api_key_ref: Option<String>,
    pub model: String,
    pub small_fast_model: Option<String>,
    pub reasoning_model: Option<String>,
    pub max_tokens_field: String,
    pub supports_tools: bool,
    pub supports_json_schema: bool,
    pub supports_streaming_usage: bool,
    pub supports_reasoning_effort: bool,
    pub extra_headers: Vec<(String, String)>,
    pub extra_body: serde_json::Value,
}

/// Which model tier a request wants, per spec "Model fallback chain": a
/// caller can ask for the profile's primary model, its faster/cheaper model,
/// or its stronger reasoning model, and get a sensible fallback when the
/// requested tier isn't configured on this profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    /// The profile's default/general-purpose model.
    Primary,
    /// A smaller, faster, cheaper model for low-stakes/high-volume calls.
    SmallFast,
    /// A stronger model tuned for harder reasoning tasks.
    Reasoning,
}

impl ProviderProfile {
    /// Builds a minimal OpenAI-compatible profile pointing at `base_url`.
    pub fn openai_compatible(name: impl Into<String>, base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            id: ProviderProfileId::new(),
            name: name.into(),
            kind: ProviderKind::OpenAiCompatible,
            base_url: base_url.into(),
            api_key_ref: None,
            model: model.into(),
            small_fast_model: None,
            reasoning_model: None,
            max_tokens_field: "max_tokens".to_string(),
            supports_tools: true,
            supports_json_schema: true,
            supports_streaming_usage: false,
            supports_reasoning_effort: false,
            extra_headers: Vec::new(),
            extra_body: serde_json::Value::Null,
        }
    }

    /// Resolves the model name to use for a given tier, per spec "Model
    /// fallback chain": `SmallFast`/`Reasoning` fall back to the profile's
    /// primary `model` when that tier isn't configured, so callers can always
    /// request a tier without checking whether it's set first.
    pub fn model_for_tier(&self, tier: ModelTier) -> &str {
        match tier {
            ModelTier::Primary => &self.model,
            ModelTier::SmallFast => self.small_fast_model.as_deref().unwrap_or(&self.model),
            ModelTier::Reasoning => self.reasoning_model.as_deref().unwrap_or(&self.model),
        }
    }

    /// Builds the ordered model fallback chain for this profile, per spec
    /// "Model fallback chain": starts with the requested tier's model, then
    /// falls back through the remaining configured models (deduplicated,
    /// preserving order) so a caller can retry against progressively more
    /// conservative models if the preferred one fails.
    pub fn fallback_chain(&self, tier: ModelTier) -> Vec<String> {
        let ordered_tiers = match tier {
            ModelTier::Primary => [ModelTier::Primary, ModelTier::SmallFast, ModelTier::Reasoning],
            ModelTier::SmallFast => [ModelTier::SmallFast, ModelTier::Primary, ModelTier::Reasoning],
            ModelTier::Reasoning => [ModelTier::Reasoning, ModelTier::Primary, ModelTier::SmallFast],
        };
        let mut chain = Vec::new();
        for t in ordered_tiers {
            let candidate = match t {
                ModelTier::Primary => Some(self.model.clone()),
                ModelTier::SmallFast => self.small_fast_model.clone(),
                ModelTier::Reasoning => self.reasoning_model.clone(),
            };
            if let Some(candidate) = candidate {
                if !chain.contains(&candidate) {
                    chain.push(candidate);
                }
            }
        }
        chain
    }
}

/// On-disk representation of a single provider profile in a TOML config file.
/// Mirrors `ProviderProfile` but with defaults for optional fields so a
/// profile only needs to specify what it overrides.
#[derive(Debug, Clone, Deserialize)]
struct ProviderProfileToml {
    name: String,
    #[serde(default = "default_kind")]
    kind: ProviderKind,
    base_url: String,
    api_key_ref: Option<String>,
    model: String,
    small_fast_model: Option<String>,
    reasoning_model: Option<String>,
    #[serde(default = "default_max_tokens_field")]
    max_tokens_field: String,
    #[serde(default = "default_true")]
    supports_tools: bool,
    #[serde(default = "default_true")]
    supports_json_schema: bool,
    #[serde(default)]
    supports_streaming_usage: bool,
    #[serde(default)]
    supports_reasoning_effort: bool,
    #[serde(default)]
    extra_headers: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    extra_body: serde_json::Value,
}

fn default_kind() -> ProviderKind {
    ProviderKind::OpenAiCompatible
}

fn default_max_tokens_field() -> String {
    "max_tokens".to_string()
}

fn default_true() -> bool {
    true
}

/// Top-level shape of a provider profiles config file: `[[profiles]]` array.
#[derive(Debug, Clone, Deserialize, Default)]
struct ProviderProfilesFile {
    #[serde(default)]
    profiles: Vec<ProviderProfileToml>,
}

/// Loads provider profiles from a TOML config source (file contents, not a
/// path). Expects a top-level `[[profiles]]` array; each entry maps to a
/// `ProviderProfile`. `api_key_ref` is stored as-is (an env var name) and is
/// resolved lazily by the adapter, never eagerly read here.
pub fn load_profiles_from_toml(toml_source: &str) -> SakhaResult<Vec<ProviderProfile>> {
    let parsed: ProviderProfilesFile = toml::from_str(toml_source).map_err(|e| {
        SakhaError::invalid_input("sakha-provider", format!("invalid provider profile TOML: {e}"))
    })?;

    parsed
        .profiles
        .into_iter()
        .map(|p| {
            let extra_body = if p.extra_body.is_null() {
                serde_json::Value::Null
            } else {
                p.extra_body
            };
            Ok(ProviderProfile {
                id: ProviderProfileId::new(),
                name: p.name,
                kind: p.kind,
                base_url: p.base_url,
                api_key_ref: p.api_key_ref,
                model: p.model,
                small_fast_model: p.small_fast_model,
                reasoning_model: p.reasoning_model,
                max_tokens_field: p.max_tokens_field,
                supports_tools: p.supports_tools,
                supports_json_schema: p.supports_json_schema,
                supports_streaming_usage: p.supports_streaming_usage,
                supports_reasoning_effort: p.supports_reasoning_effort,
                extra_headers: p.extra_headers.into_iter().collect(),
                extra_body,
            })
        })
        .collect()
}

/// Well-known provider base URLs, per spec "Provider Profiles". Used to build
/// ready-made profiles for Gemini/xAI/local gateways without hand-typing URLs.
pub mod well_known {
    pub const GEMINI_OPENAI_COMPAT_BASE_URL: &str =
        "https://generativelanguage.googleapis.com/v1beta/openai/";
    pub const XAI_BASE_URL: &str = "https://api.x.ai/v1";
    pub const OLLAMA_DEFAULT_BASE_URL: &str = "http://localhost:11434/v1";
    pub const LM_STUDIO_DEFAULT_BASE_URL: &str = "http://localhost:1234/v1";
    pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
}

impl ProviderProfile {
    /// Builds a Gemini profile using OpenAI-compatible mode, per spec
    /// "Gemini: Use OpenAI-compatible mode with base_url=...".
    pub fn gemini(model: impl Into<String>, api_key_ref: impl Into<String>) -> Self {
        let mut profile = Self::openai_compatible("gemini", well_known::GEMINI_OPENAI_COMPAT_BASE_URL, model);
        profile.api_key_ref = Some(api_key_ref.into());
        profile.supports_reasoning_effort = false;
        profile
    }

    /// Builds an xAI/Grok profile using OpenAI-compatible mode.
    pub fn xai(model: impl Into<String>, api_key_ref: impl Into<String>) -> Self {
        let mut profile = Self::openai_compatible("xai", well_known::XAI_BASE_URL, model);
        profile.api_key_ref = Some(api_key_ref.into());
        profile
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_profiles_from_toml_parses_minimal_profile() {
        let toml_source = r#"
[[profiles]]
name = "openai"
base_url = "https://api.openai.com/v1"
model = "gpt-4o-mini"
api_key_ref = "OPENAI_API_KEY"
"#;
        let profiles = load_profiles_from_toml(toml_source).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "openai");
        assert_eq!(profiles[0].base_url, "https://api.openai.com/v1");
        assert_eq!(profiles[0].api_key_ref.as_deref(), Some("OPENAI_API_KEY"));
        // Defaults applied.
        assert_eq!(profiles[0].max_tokens_field, "max_tokens");
        assert!(profiles[0].supports_tools);
    }

    #[test]
    fn load_profiles_from_toml_parses_capability_overrides() {
        let toml_source = r#"
[[profiles]]
name = "local"
base_url = "http://localhost:11434/v1"
model = "llama3"
supports_tools = false
max_tokens_field = "max_completion_tokens"

[extra_headers]
"#;
        let profiles = load_profiles_from_toml(toml_source).unwrap();
        assert_eq!(profiles.len(), 1);
        assert!(!profiles[0].supports_tools);
        assert_eq!(profiles[0].max_tokens_field, "max_completion_tokens");
    }

    #[test]
    fn load_profiles_from_toml_rejects_invalid_toml() {
        let result = load_profiles_from_toml("not valid toml {{{");
        assert!(result.is_err());
    }

    #[test]
    fn load_profiles_from_toml_empty_source_yields_empty_list() {
        let profiles = load_profiles_from_toml("").unwrap();
        assert!(profiles.is_empty());
    }

    #[test]
    fn gemini_profile_uses_well_known_base_url() {
        let profile = ProviderProfile::gemini("gemini-2.0-flash", "GEMINI_API_KEY");
        assert_eq!(profile.base_url, well_known::GEMINI_OPENAI_COMPAT_BASE_URL);
        assert_eq!(profile.api_key_ref.as_deref(), Some("GEMINI_API_KEY"));
    }

    #[test]
    fn xai_profile_uses_well_known_base_url() {
        let profile = ProviderProfile::xai("grok-2", "XAI_API_KEY");
        assert_eq!(profile.base_url, well_known::XAI_BASE_URL);
    }

    #[test]
    fn model_for_tier_falls_back_to_primary_when_tier_unset() {
        let profile = ProviderProfile::openai_compatible("test", "https://example.com", "gpt-main");
        assert_eq!(profile.model_for_tier(ModelTier::SmallFast), "gpt-main");
        assert_eq!(profile.model_for_tier(ModelTier::Reasoning), "gpt-main");
        assert_eq!(profile.model_for_tier(ModelTier::Primary), "gpt-main");
    }

    #[test]
    fn model_for_tier_uses_configured_tier_model() {
        let mut profile = ProviderProfile::openai_compatible("test", "https://example.com", "gpt-main");
        profile.small_fast_model = Some("gpt-mini".to_string());
        profile.reasoning_model = Some("gpt-reasoning".to_string());
        assert_eq!(profile.model_for_tier(ModelTier::SmallFast), "gpt-mini");
        assert_eq!(profile.model_for_tier(ModelTier::Reasoning), "gpt-reasoning");
    }

    #[test]
    fn fallback_chain_starts_with_requested_tier_then_remaining_models() {
        let mut profile = ProviderProfile::openai_compatible("test", "https://example.com", "gpt-main");
        profile.small_fast_model = Some("gpt-mini".to_string());
        profile.reasoning_model = Some("gpt-reasoning".to_string());
        let chain = profile.fallback_chain(ModelTier::Reasoning);
        assert_eq!(chain, vec!["gpt-reasoning".to_string(), "gpt-main".to_string(), "gpt-mini".to_string()]);
    }

    #[test]
    fn fallback_chain_dedups_when_tiers_share_a_model() {
        let profile = ProviderProfile::openai_compatible("test", "https://example.com", "gpt-main");
        let chain = profile.fallback_chain(ModelTier::Primary);
        assert_eq!(chain, vec!["gpt-main".to_string()]);
    }
}
