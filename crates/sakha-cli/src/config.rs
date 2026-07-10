//! `sakha config get/set`: on-disk TOML config at `~/.sakha/config.toml`
//! with `${ENV_VAR}` expansion. See `modules/17-config-secrets-policy.md`
//! and `modules/13-ui-cli-tui-web-desktop.md` "sakha config get/set".

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Which provider backend `sakha run`/`sakha chat` should talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSelection {
    /// `MockProviderClient`: no network, deterministic canned output. Used
    /// for offline smoke tests and as the safe default.
    Mock,
    /// `OpenAiCompatibleClient` against `provider.base_url`.
    OpenAiCompatible,
}

impl Default for ProviderSelection {
    fn default() -> Self {
        ProviderSelection::Mock
    }
}

/// Provider connection settings. `api_key_env` names an environment
/// variable holding the key; the raw key itself is never stored on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default)]
    pub selection: ProviderSelection,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub model: String,
    pub api_key_env: Option<String>,
}

fn default_base_url() -> String {
    "http://localhost:11434/v1".to_string()
}

fn default_model() -> String {
    "mock-model".to_string()
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            selection: ProviderSelection::default(),
            base_url: default_base_url(),
            model: default_model(),
            api_key_env: None,
        }
    }
}

/// Top-level on-disk config shape, `~/.sakha/config.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SakhaConfig {
    #[serde(default)]
    pub provider: ProviderConfig,
    /// SQLite database path for session/memory/handoff/research/artifact
    /// stores. `None` means use the in-memory store (non-durable).
    pub db_path: Option<String>,
    /// Free-form additional keys, so `config set some.custom.key value`
    /// never fails even for keys this struct doesn't model explicitly.
    #[serde(flatten)]
    pub extra: toml::value::Table,
}

/// Returns the default config file path: `~/.sakha/config.toml`.
///
/// Checks `SAKHA_HOME` first (an explicit override, e.g. for CI or tests
/// that want a hermetic config directory) before falling back to
/// `dirs::home_dir()`. This matters beyond convenience: on Windows,
/// `dirs::home_dir()` resolves via the `SHGetKnownFolderPath` API rather than
/// reading `HOME`/`USERPROFILE` from the process environment, so tests that
/// only override those env vars do not actually redirect
/// `default_config_path()` and can silently read/write the real
/// `~/.sakha/config.toml`. `SAKHA_HOME` sidesteps that platform quirk.
pub fn default_config_path() -> PathBuf {
    let home = std::env::var_os("SAKHA_HOME").map(PathBuf::from).or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("."));
    home.join(".sakha").join("config.toml")
}

/// Expands `${VAR}` (and bare `$VAR`) references in `input` using the
/// current process environment. Unresolved references are left as-is
/// rather than erroring, so partially-configured environments still parse.
pub fn expand_env(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(end) = input[i + 2..].find('}') {
                let var_name = &input[i + 2..i + 2 + end];
                match std::env::var(var_name) {
                    Ok(val) => out.push_str(&val),
                    Err(_) => {
                        out.push_str("${");
                        out.push_str(var_name);
                        out.push('}');
                    }
                }
                i += 2 + end + 1;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Loads config from `path`, applying env-var expansion to the raw TOML
/// source before parsing. Returns `SakhaConfig::default()` if the file does
/// not exist yet (first run).
pub fn load_config(path: &Path) -> anyhow::Result<SakhaConfig> {
    if !path.exists() {
        return Ok(SakhaConfig::default());
    }
    let raw = std::fs::read_to_string(path)?;
    let expanded = expand_env(&raw);
    let config: SakhaConfig = toml::from_str(&expanded)?;
    Ok(config)
}

/// Writes `config` back to `path` as TOML, creating parent directories as
/// needed.
pub fn save_config(path: &Path, config: &SakhaConfig) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let serialized = toml::to_string_pretty(config)?;
    std::fs::write(path, serialized)?;
    Ok(())
}

/// Reads a dotted key (e.g. `"provider.model"`) out of `config`, returning
/// its value as a display string, if present.
pub fn get_key(config: &SakhaConfig, key: &str) -> Option<String> {
    let value = toml::Value::try_from(config).ok()?;
    let mut current = &value;
    for part in key.split('.') {
        current = current.as_table()?.get(part)?;
    }
    Some(match current {
        toml::Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

/// Sets a dotted key (e.g. `"provider.model"`) to `value` (parsed as TOML
/// where possible, falling back to a plain string), returning the updated
/// config. Unknown top-level sections are created under `extra`.
pub fn set_key(config: &mut SakhaConfig, key: &str, value: &str) -> anyhow::Result<()> {
    let mut root = toml::Value::try_from(&*config)?;
    let parsed_value = parse_scalar(value).unwrap_or_else(|| toml::Value::String(value.to_string()));

    set_dotted(&mut root, key, parsed_value)?;
    *config = root.try_into()?;
    Ok(())
}

/// Attempts to interpret `raw` as a bool/int/float TOML scalar; falls back
/// to `None` (caller then stores it as a plain string) for anything else.
fn parse_scalar(raw: &str) -> Option<toml::Value> {
    if let Ok(b) = raw.parse::<bool>() {
        return Some(toml::Value::Boolean(b));
    }
    if let Ok(i) = raw.parse::<i64>() {
        return Some(toml::Value::Integer(i));
    }
    if let Ok(f) = raw.parse::<f64>() {
        return Some(toml::Value::Float(f));
    }
    None
}

fn set_dotted(root: &mut toml::Value, key: &str, value: toml::Value) -> anyhow::Result<()> {
    let parts: Vec<&str> = key.split('.').collect();
    let mut current = root
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("config root is not a table"))?;

    for part in &parts[..parts.len() - 1] {
        current = current
            .entry(part.to_string())
            .or_insert_with(|| toml::Value::Table(toml::value::Table::new()))
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("config path '{}' is not a table", key))?;
    }
    current.insert(parts[parts.len() - 1].to_string(), value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_env_replaces_known_variable() {
        std::env::set_var("SAKHA_TEST_VAR_X", "resolved");
        let out = expand_env("value = \"${SAKHA_TEST_VAR_X}\"");
        assert_eq!(out, "value = \"resolved\"");
        std::env::remove_var("SAKHA_TEST_VAR_X");
    }

    #[test]
    fn expand_env_leaves_unknown_variable_untouched() {
        std::env::remove_var("SAKHA_DEFINITELY_UNSET_VAR");
        let out = expand_env("value = \"${SAKHA_DEFINITELY_UNSET_VAR}\"");
        assert_eq!(out, "value = \"${SAKHA_DEFINITELY_UNSET_VAR}\"");
    }

    #[test]
    fn load_config_missing_file_yields_default() {
        let path = std::env::temp_dir().join("sakha-cli-test-missing-config.toml");
        let _ = std::fs::remove_file(&path);
        let config = load_config(&path).unwrap();
        assert_eq!(config.provider.model, "mock-model");
    }

    #[test]
    fn set_key_then_get_key_round_trips() {
        let mut config = SakhaConfig::default();
        set_key(&mut config, "provider.model", "gpt-4o-mini").unwrap();
        assert_eq!(get_key(&config, "provider.model").as_deref(), Some("gpt-4o-mini"));
    }

    #[test]
    fn set_key_creates_nested_path_under_extra() {
        let mut config = SakhaConfig::default();
        set_key(&mut config, "custom.nested.key", "hello").unwrap();
        assert_eq!(get_key(&config, "custom.nested.key").as_deref(), Some("hello"));
    }

    #[test]
    fn save_and_load_round_trip_through_tempfile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = SakhaConfig::default();
        config.provider.model = "custom-model".to_string();
        save_config(&path, &config).unwrap();
        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.provider.model, "custom-model");
    }
}
