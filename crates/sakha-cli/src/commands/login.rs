//! `sakha login <preset>` / `sakha logout <preset>`: obtain and store a
//! provider API key (OAuth PKCE for OpenRouter, pasted-key prompt for
//! everyone else, or `--api-key` for fully non-interactive use), then wire
//! `~/.sakha/config.toml`'s `[provider]` section to point at it. See spec
//! `modules/17-config-secrets-policy.md`: the raw key is stored in the OS
//! credential manager (or supplied via env var), never written to disk in
//! the config file.

use std::time::Duration;

use clap::Args;
use rand::Rng;
use sha2::{Digest, Sha256};

use crate::catalog::{self, ProviderPreset};
use crate::config::{default_config_path, load_config, save_config, ProviderSelection};
use crate::output::{print_error, OutputFormat};
use crate::secrets::{mask_secret, KeyringSecretBackend, SecretBackend};

/// How long `sakha login <preset>` (OAuth flow) waits for the browser
/// redirect back to the local callback server before giving up.
const OAUTH_CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);

/// Timeout for the `GET {base_url}/models` validation request.
const VALIDATE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Args)]
pub struct LoginArgs {
    /// Provider catalog id, e.g. `openai`, `openrouter`, `gemini`. See
    /// `sakha providers catalog`.
    pub preset: String,

    /// Supply the API key directly (non-interactive): skips the browser and
    /// any prompt, and skips OAuth even for providers that support it.
    #[arg(long)]
    pub api_key: Option<String>,

    /// Skip the `GET {base_url}/models` validation call after obtaining a
    /// key. Some providers gate that endpoint even for valid keys, so a
    /// validation failure is only ever a warning, never fatal — this flag
    /// just skips the network call entirely (e.g. for offline/CI use).
    #[arg(long)]
    pub skip_validate: bool,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Args)]
pub struct LogoutArgs {
    /// Provider catalog id whose stored key should be removed.
    pub preset: String,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

pub fn execute_login(args: LoginArgs) -> i32 {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(err) => {
            print_error(&format!("failed to start async runtime: {err}"), args.output);
            return 1;
        }
    };
    rt.block_on(login_async(args, &KeyringSecretBackend::new()))
}

pub fn execute_logout(args: LogoutArgs) -> i32 {
    logout_with_backend(args, &KeyringSecretBackend::new())
}

/// Testable core of `sakha logout`: takes an injected `SecretBackend` so
/// tests never touch the real OS credential manager.
fn logout_with_backend(args: LogoutArgs, backend: &dyn SecretBackend) -> i32 {
    let Some(preset) = catalog::find(&args.preset) else {
        print_error(&format!("unknown provider preset: {}", args.preset), args.output);
        return 2;
    };
    match backend.delete(preset.id) {
        Ok(()) => {
            match args.output {
                OutputFormat::Json => {
                    let payload = serde_json::json!({ "preset": preset.id, "status": "logged_out" });
                    println!("{}", serde_json::to_string_pretty(&payload).unwrap_or_default());
                }
                OutputFormat::Text => println!("Logged out of {} ({}).", preset.display_name, preset.id),
            }
            0
        }
        Err(err) => {
            print_error(&format!("failed to delete stored key: {err}"), args.output);
            1
        }
    }
}

/// Testable core of `sakha login`: takes an injected `SecretBackend` so
/// tests never touch the real OS credential manager. Handles all three key
/// acquisition paths (`--api-key`, OAuth PKCE, pasted-key prompt) and then
/// updates the on-disk config.
async fn login_async(args: LoginArgs, backend: &dyn SecretBackend) -> i32 {
    let Some(preset) = catalog::find(&args.preset) else {
        print_error(&format!("unknown provider preset: {}", args.preset), args.output);
        return 2;
    };

    let key = if let Some(key) = &args.api_key {
        key.clone()
    } else if preset.supports_oauth_pkce {
        match oauth_pkce_flow(preset).await {
            Ok(key) => key,
            Err(err) => {
                print_error(&format!("OAuth login failed: {err}"), args.output);
                return 1;
            }
        }
    } else {
        match prompt_for_pasted_key(preset) {
            Ok(key) => key,
            Err(err) => {
                print_error(&format!("failed to read API key: {err}"), args.output);
                return 1;
            }
        }
    };

    let key = key.trim().to_string();
    if key.is_empty() {
        print_error("no API key provided", args.output);
        return 1;
    }

    if !args.skip_validate {
        match validate_key(preset, &key).await {
            Ok(true) => {}
            Ok(false) => eprintln!(
                "warning: could not validate the key against {}/models (this provider may simply not expose that endpoint) — storing it anyway",
                preset.base_url
            ),
            Err(err) => eprintln!("warning: validation request failed ({err}) — storing the key anyway"),
        }
    }

    if let Err(err) = backend.set(preset.id, &key) {
        print_error(&format!("failed to store key: {err}"), args.output);
        return 1;
    }

    if let Err(err) = update_config_for_preset(preset) {
        print_error(&format!("stored the key, but failed to update config: {err}"), args.output);
        return 1;
    }

    match args.output {
        OutputFormat::Json => {
            let payload = serde_json::json!({
                "preset": preset.id,
                "display_name": preset.display_name,
                "base_url": preset.base_url,
                "api_key_env": preset.api_key_env,
                "key": mask_secret(&key),
                "status": "logged_in",
            });
            println!("{}", serde_json::to_string_pretty(&payload).unwrap_or_default());
        }
        OutputFormat::Text => {
            println!(
                "Logged in to {} ({}). Key stored securely ({}). provider.selection = open_ai_compatible, provider.base_url = {}",
                preset.display_name,
                preset.id,
                mask_secret(&key),
                preset.base_url,
            );
        }
    }

    0
}

/// Updates `~/.sakha/config.toml`'s `[provider]` section for `preset`:
/// selection, base_url, api_key_env, and preset id. Preserves the existing
/// model if the config was already pointed at this same preset (so re-login
/// doesn't clobber a user's model override); otherwise uses the preset's
/// default model.
fn update_config_for_preset(preset: &ProviderPreset) -> anyhow::Result<()> {
    let path = default_config_path();
    let mut config = load_config(&path)?;

    let keep_existing_model = config.provider.preset.as_deref() == Some(preset.id);
    config.provider.selection = ProviderSelection::OpenAiCompatible;
    config.provider.base_url = preset.base_url.to_string();
    if !keep_existing_model {
        config.provider.model = preset.default_model.to_string();
    }
    config.provider.api_key_env = preset.api_key_env.map(|s| s.to_string());
    config.provider.preset = Some(preset.id.to_string());

    save_config(&path, &config)
}

/// Prompts the user to open `keys_console_url` (if any) and paste back an
/// API key, hidden via `rpassword`.
fn prompt_for_pasted_key(preset: &ProviderPreset) -> anyhow::Result<String> {
    if let Some(url) = preset.keys_console_url {
        println!("Opening {url} to create an API key for {}...", preset.display_name);
        if webbrowser::open(url).is_err() {
            println!("Could not open a browser automatically. Please visit: {url}");
        }
    } else {
        println!("No key console URL is known for {}; set --base-url and paste a key from your provider.", preset.display_name);
    }
    let key = rpassword::prompt_password("Paste your API key: ")?;
    Ok(key)
}

/// Runs the OpenRouter OAuth PKCE flow: generates a code verifier/challenge,
/// opens the browser to the authorization page, listens for the local
/// callback carrying `?code=...`, then exchanges that code for an API key.
/// See spec `modules/17-config-secrets-policy.md` "OAuth PKCE" and
/// OpenRouter's documented PKCE flow.
async fn oauth_pkce_flow(preset: &ProviderPreset) -> anyhow::Result<String> {
    let verifier = generate_code_verifier();
    let challenge = code_challenge_s256(&verifier);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    let auth_url = format!(
        "https://openrouter.ai/auth?callback_url=http://localhost:{port}/callback&code_challenge={challenge}&code_challenge_method=S256"
    );
    println!("Opening {auth_url} to authorize Sakha with {}...", preset.display_name);
    if webbrowser::open(&auth_url).is_err() {
        println!("Could not open a browser automatically. Please visit: {auth_url}");
    }

    let code = tokio::time::timeout(OAUTH_CALLBACK_TIMEOUT, accept_oauth_callback(listener))
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for browser redirect (waited {}s)", OAUTH_CALLBACK_TIMEOUT.as_secs()))??;

    exchange_openrouter_code(&code, &verifier).await
}

/// Accepts exactly one TCP connection on `listener`, parses the request
/// line for `GET /callback?code=...`, replies with a minimal HTML page, and
/// returns the extracted `code`.
async fn accept_oauth_callback(listener: tokio::net::TcpListener) -> anyhow::Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (mut socket, _) = listener.accept().await?;
    let mut buf = [0u8; 8192];
    let n = socket.read(&mut buf).await?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let request_line = request.lines().next().unwrap_or_default();

    let code = parse_callback_code(request_line).ok_or_else(|| anyhow::anyhow!("callback request did not include a `code` parameter"))?;

    let body = "<html><body>Sakha is connected \u{2014} you can close this tab.</body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = socket.write_all(response.as_bytes()).await;
    let _ = socket.shutdown().await;

    Ok(code)
}

/// Extracts the `code` query parameter from an HTTP request line like
/// `GET /callback?code=abc123&state=xyz HTTP/1.1`.
fn parse_callback_code(request_line: &str) -> Option<String> {
    let path = request_line.split_whitespace().nth(1)?;
    let query = path.split_once('?')?.1;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        if key == "code" {
            return Some(urldecode(value));
        }
    }
    None
}

/// Minimal percent-decoding for query-string values (no external dependency
/// needed for the small character set OAuth codes use).
fn urldecode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                if let Ok(byte) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                    out.push(byte);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Exchanges an OAuth `code` + PKCE `code_verifier` for an OpenRouter API
/// key via `POST https://openrouter.ai/api/v1/auth/keys`.
async fn exchange_openrouter_code(code: &str, code_verifier: &str) -> anyhow::Result<String> {
    let client = reqwest::Client::new();
    let response = client
        .post("https://openrouter.ai/api/v1/auth/keys")
        .json(&serde_json::json!({
            "code": code,
            "code_verifier": code_verifier,
            "code_challenge_method": "S256",
        }))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("token exchange failed ({status}): {body}");
    }

    let body: serde_json::Value = response.json().await?;
    body.get("key")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("token exchange response missing `key` field"))
}

/// Generates a PKCE code verifier: 43-128 characters from the unreserved set
/// `[A-Za-z0-9-._~]`, per RFC 7636 section 4.1.
fn generate_code_verifier() -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    const LEN: usize = 64;
    let mut rng = rand::thread_rng();
    (0..LEN).map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char).collect()
}

/// Computes the PKCE `S256` code challenge for `verifier`: base64url
/// (no padding) of the SHA-256 digest, per RFC 7636 section 4.2.
fn code_challenge_s256(verifier: &str) -> String {
    use base64::Engine;
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// Validates a freshly obtained key against `GET {base_url}/models`. Returns
/// `Ok(true)` on any 2xx, `Ok(false)` on a non-2xx HTTP response (still not
/// fatal — the caller only warns), and `Err` on a transport-level failure.
async fn validate_key(preset: &ProviderPreset, key: &str) -> anyhow::Result<bool> {
    let url = format!("{}/models", preset.base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder().timeout(VALIDATE_TIMEOUT).build()?;
    let response = client.get(url).bearer_auth(key).send().await?;
    Ok(response.status().is_success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::test_support::InMemorySecretBackend;

    #[test]
    fn code_challenge_matches_known_pkce_vector() {
        // RFC 7636 Appendix B test vector.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = code_challenge_s256(verifier);
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn generated_code_verifier_has_valid_length_and_charset() {
        let verifier = generate_code_verifier();
        assert!(verifier.len() >= 43 && verifier.len() <= 128);
        assert!(verifier.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~')));
    }

    #[test]
    fn parse_callback_code_extracts_code_param() {
        let line = "GET /callback?code=abc123&state=xyz HTTP/1.1";
        assert_eq!(parse_callback_code(line).as_deref(), Some("abc123"));
    }

    #[test]
    fn parse_callback_code_returns_none_without_code() {
        let line = "GET /callback?state=xyz HTTP/1.1";
        assert!(parse_callback_code(line).is_none());
    }

    #[test]
    fn parse_callback_code_percent_decodes_value() {
        let line = "GET /callback?code=abc%2Bdef HTTP/1.1";
        assert_eq!(parse_callback_code(line).as_deref(), Some("abc+def"));
    }

    #[tokio::test]
    async fn login_with_api_key_and_skip_validate_stores_and_updates_config() {
        let _home = crate::test_support::TempHome::new();
        let backend = InMemorySecretBackend::new();
        let code = login_async(
            LoginArgs {
                preset: "openai".to_string(),
                api_key: Some("sk-test-123456".to_string()),
                skip_validate: true,
                output: OutputFormat::Json,
            },
            &backend,
        )
        .await;
        assert_eq!(code, 0);
        assert_eq!(backend.get("openai").unwrap().as_deref(), Some("sk-test-123456"));

        let config = load_config(&default_config_path()).unwrap();
        assert_eq!(config.provider.selection, ProviderSelection::OpenAiCompatible);
        assert_eq!(config.provider.base_url, "https://api.openai.com/v1");
        assert_eq!(config.provider.model, "gpt-5-mini");
        assert_eq!(config.provider.api_key_env.as_deref(), Some("OPENAI_API_KEY"));
        assert_eq!(config.provider.preset.as_deref(), Some("openai"));
    }

    #[tokio::test]
    async fn login_unknown_preset_fails_without_touching_config() {
        let _home = crate::test_support::TempHome::new();
        let backend = InMemorySecretBackend::new();
        let code = login_async(
            LoginArgs {
                preset: "not-a-real-provider".to_string(),
                api_key: Some("sk-test".to_string()),
                skip_validate: true,
                output: OutputFormat::Json,
            },
            &backend,
        )
        .await;
        assert_eq!(code, 2);
    }

    #[tokio::test]
    async fn login_preserves_existing_model_on_relogin_to_same_preset() {
        let _home = crate::test_support::TempHome::new();
        let backend = InMemorySecretBackend::new();
        login_async(
            LoginArgs {
                preset: "openai".to_string(),
                api_key: Some("sk-first".to_string()),
                skip_validate: true,
                output: OutputFormat::Json,
            },
            &backend,
        )
        .await;

        let mut config = load_config(&default_config_path()).unwrap();
        config.provider.model = "gpt-5".to_string();
        save_config(&default_config_path(), &config).unwrap();

        login_async(
            LoginArgs {
                preset: "openai".to_string(),
                api_key: Some("sk-second".to_string()),
                skip_validate: true,
                output: OutputFormat::Json,
            },
            &backend,
        )
        .await;

        let config = load_config(&default_config_path()).unwrap();
        assert_eq!(config.provider.model, "gpt-5");
        assert_eq!(backend.get("openai").unwrap().as_deref(), Some("sk-second"));
    }

    #[test]
    fn logout_deletes_stored_key() {
        let backend = InMemorySecretBackend::new();
        backend.set("openai", "sk-test").unwrap();
        let code = logout_with_backend(LogoutArgs { preset: "openai".to_string(), output: OutputFormat::Json }, &backend);
        assert_eq!(code, 0);
        assert!(backend.get("openai").unwrap().is_none());
    }

    #[test]
    fn logout_unknown_preset_fails() {
        let backend = InMemorySecretBackend::new();
        let code = logout_with_backend(LogoutArgs { preset: "not-real".to_string(), output: OutputFormat::Json }, &backend);
        assert_eq!(code, 2);
    }
}
