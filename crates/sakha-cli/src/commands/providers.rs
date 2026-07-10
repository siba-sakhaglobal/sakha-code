//! `sakha providers list|catalog|use`: inspect the currently configured
//! provider, browse the static provider catalog (`src/catalog.rs`), and
//! switch `~/.sakha/config.toml` to a catalog preset without going through
//! `sakha login`.

use clap::Args;
use serde::Serialize;

use crate::catalog::{self, ProviderPreset};
use crate::config::{default_config_path, load_config, save_config, ProviderSelection};
use crate::output::{print_error, print_output, OutputFormat};
use crate::runtime::build_provider;
use crate::secrets::{KeyringSecretBackend, SecretBackend};

#[derive(Debug, Args)]
pub struct ProvidersListArgs {
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Args)]
pub struct ProvidersCatalogArgs {
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Args)]
pub struct ProvidersUseArgs {
    /// Provider catalog id, e.g. `openai`, `ollama`. See `sakha providers catalog`.
    pub preset: String,

    /// Override the preset's default model.
    #[arg(long)]
    pub model: Option<String>,

    /// Override the preset's default base URL (useful for self-hosted
    /// gateways like LiteLLM).
    #[arg(long)]
    pub base_url: Option<String>,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Serialize)]
struct ProviderView {
    selection: String,
    model: String,
    base_url: String,
    health: String,
    capabilities: sakha_provider::ProviderCapabilities,
}

pub fn execute(args: ProvidersListArgs) -> i32 {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("failed to start async runtime: {err}");
            return 1;
        }
    };
    rt.block_on(execute_async(args))
}

async fn execute_async(args: ProvidersListArgs) -> i32 {
    let config = match crate::config::load_config(&default_config_path()) {
        Ok(c) => c,
        Err(err) => {
            print_error(&format!("failed to load config: {err}"), args.output);
            return 2;
        }
    };
    let provider = build_provider(&config);
    let health = provider.health().await.map(|h| format!("{h:?}")).unwrap_or_else(|e| e.to_string());

    let view = ProviderView {
        selection: format!("{:?}", config.provider.selection),
        model: config.provider.model.clone(),
        base_url: config.provider.base_url.clone(),
        health,
        capabilities: provider.capabilities(),
    };

    match args.output {
        OutputFormat::Json => print_output(&vec![view], args.output),
        OutputFormat::Text => {
            println!("{:<20} {:<30} {:<40} {:<12}", "SELECTION", "MODEL", "BASE URL", "HEALTH");
            println!("{:<20} {:<30} {:<40} {:<12}", view.selection, view.model, view.base_url, view.health);
        }
    }
    0
}

/// `sakha providers catalog [--output json]`: lists every static catalog
/// preset with its kind, base URL, default model, auth method, and whether a
/// key is currently available for it (env var set, or a keyring entry
/// stored via `sakha login`).
pub fn execute_catalog(args: ProvidersCatalogArgs) -> i32 {
    execute_catalog_with_backend(args, &KeyringSecretBackend::new())
}

fn execute_catalog_with_backend(args: ProvidersCatalogArgs, backend: &dyn SecretBackend) -> i32 {
    #[derive(Debug, Serialize)]
    struct CatalogRow {
        id: &'static str,
        display_name: &'static str,
        kind: catalog::ProviderKind,
        base_url: &'static str,
        default_model: &'static str,
        auth: &'static str,
        key_available: bool,
        notes: Option<&'static str>,
    }

    let rows: Vec<CatalogRow> = catalog::all()
        .iter()
        .map(|preset| {
            let auth = if preset.supports_oauth_pkce {
                "oauth"
            } else if preset.api_key_env.is_some() {
                "api-key"
            } else {
                "none"
            };
            CatalogRow {
                id: preset.id,
                display_name: preset.display_name,
                kind: preset.kind,
                base_url: preset.base_url,
                default_model: preset.default_model,
                auth,
                key_available: key_available_for(preset, backend),
                notes: preset.notes,
            }
        })
        .collect();

    match args.output {
        OutputFormat::Json => print_output(&rows, args.output),
        OutputFormat::Text => {
            println!("{:<20} {:<10} {:<45} {:<9} {:<8} NOTES", "ID", "KIND", "BASE URL", "AUTH", "KEY?");
            for row in &rows {
                println!(
                    "{:<20} {:<10} {:<45} {:<9} {:<8} {}",
                    row.id,
                    format!("{:?}", row.kind),
                    row.base_url,
                    row.auth,
                    if row.key_available { "yes" } else { "no" },
                    row.notes.unwrap_or(""),
                );
            }
        }
    }
    0
}

/// Whether a key is currently available for `preset`: either its
/// `api_key_env` var is set in the process environment, or the keyring has a
/// stored secret for it (from `sakha login`). Local presets with no
/// `api_key_env` are trivially "available" (no key needed).
fn key_available_for(preset: &ProviderPreset, backend: &dyn SecretBackend) -> bool {
    match preset.api_key_env {
        None => true,
        Some(env_name) => std::env::var_os(env_name).is_some() || backend.get(preset.id).ok().flatten().is_some(),
    }
}

/// `sakha providers use <preset>`: writes `~/.sakha/config.toml`'s
/// `[provider]` section from a catalog preset without performing any login
/// flow. Warns (but does not fail) if no key is found anywhere for the
/// preset, since some presets (local gateways) need none.
pub fn execute_use(args: ProvidersUseArgs) -> i32 {
    execute_use_with_backend(args, &KeyringSecretBackend::new())
}

fn execute_use_with_backend(args: ProvidersUseArgs, backend: &dyn SecretBackend) -> i32 {
    let Some(preset) = catalog::find(&args.preset) else {
        print_error(&format!("unknown provider preset: {}", args.preset), args.output);
        return 2;
    };

    let path = default_config_path();
    let mut config = match load_config(&path) {
        Ok(c) => c,
        Err(err) => {
            print_error(&format!("failed to load config: {err}"), args.output);
            return 2;
        }
    };

    config.provider.selection = ProviderSelection::OpenAiCompatible;
    config.provider.base_url = args.base_url.clone().unwrap_or_else(|| preset.base_url.to_string());
    config.provider.model = args.model.clone().unwrap_or_else(|| preset.default_model.to_string());
    config.provider.api_key_env = preset.api_key_env.map(|s| s.to_string());
    config.provider.preset = Some(preset.id.to_string());

    if let Err(err) = save_config(&path, &config) {
        print_error(&format!("failed to save config: {err}"), args.output);
        return 1;
    }

    if !key_available_for(preset, backend) {
        eprintln!(
            "warning: no API key found for {} (env var {} not set, no stored login) — run `sakha login {}` or set the env var before use",
            preset.display_name,
            preset.api_key_env.unwrap_or("<none>"),
            preset.id,
        );
    }

    match args.output {
        OutputFormat::Json => {
            let payload = serde_json::json!({
                "preset": preset.id,
                "base_url": config.provider.base_url,
                "model": config.provider.model,
                "api_key_env": config.provider.api_key_env,
            });
            println!("{}", serde_json::to_string_pretty(&payload).unwrap_or_default());
        }
        OutputFormat::Text => {
            println!("Configured provider = {} (base_url = {}, model = {})", preset.id, config.provider.base_url, config.provider.model);
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::test_support::InMemorySecretBackend;

    #[test]
    fn providers_list_reports_configured_provider() {
        let code = execute(ProvidersListArgs { output: OutputFormat::Json });
        assert_eq!(code, 0);
    }

    #[test]
    fn providers_catalog_lists_all_presets() {
        let backend = InMemorySecretBackend::new();
        let code = execute_catalog_with_backend(ProvidersCatalogArgs { output: OutputFormat::Json }, &backend);
        assert_eq!(code, 0);
    }

    #[test]
    fn providers_use_writes_expected_config() {
        let _home = crate::test_support::TempHome::new();
        let backend = InMemorySecretBackend::new();
        let code = execute_use_with_backend(
            ProvidersUseArgs { preset: "ollama".to_string(), model: None, base_url: None, output: OutputFormat::Json },
            &backend,
        );
        assert_eq!(code, 0);

        let config = load_config(&default_config_path()).unwrap();
        assert_eq!(config.provider.selection, ProviderSelection::OpenAiCompatible);
        assert_eq!(config.provider.base_url, "http://localhost:11434/v1");
        assert_eq!(config.provider.model, "llama3.3");
        assert_eq!(config.provider.preset.as_deref(), Some("ollama"));
        assert!(config.provider.api_key_env.is_none());
    }

    #[test]
    fn providers_use_honors_model_and_base_url_overrides() {
        let _home = crate::test_support::TempHome::new();
        let backend = InMemorySecretBackend::new();
        let code = execute_use_with_backend(
            ProvidersUseArgs {
                preset: "litellm".to_string(),
                model: Some("my-custom-model".to_string()),
                base_url: Some("http://localhost:5000".to_string()),
                output: OutputFormat::Json,
            },
            &backend,
        );
        assert_eq!(code, 0);

        let config = load_config(&default_config_path()).unwrap();
        assert_eq!(config.provider.base_url, "http://localhost:5000");
        assert_eq!(config.provider.model, "my-custom-model");
    }

    #[test]
    fn providers_use_unknown_preset_fails() {
        let _home = crate::test_support::TempHome::new();
        let backend = InMemorySecretBackend::new();
        let code = execute_use_with_backend(
            ProvidersUseArgs { preset: "not-a-real-preset".to_string(), model: None, base_url: None, output: OutputFormat::Json },
            &backend,
        );
        assert_eq!(code, 2);
    }

    #[test]
    fn key_available_for_true_when_keyring_has_secret() {
        let backend = InMemorySecretBackend::new();
        let preset = catalog::find("openai").unwrap();
        assert!(!key_available_for(preset, &backend));
        backend.set("openai", "sk-test").unwrap();
        assert!(key_available_for(preset, &backend));
    }

    #[test]
    fn key_available_for_local_preset_with_no_key_env_is_true() {
        let backend = InMemorySecretBackend::new();
        let preset = catalog::find("ollama").unwrap();
        assert!(key_available_for(preset, &backend));
    }
}
