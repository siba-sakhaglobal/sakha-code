//! Secret storage for provider API keys: a thin trait over the OS credential
//! manager (via `keyring`), so `sakha login`/`sakha logout` and their tests
//! never need to touch the real credential store. See spec
//! `modules/17-config-secrets-policy.md`: raw API keys are never written to
//! `~/.sakha/config.toml`, only referenced by env var name; this module is
//! where the actual secret bytes live when the user authenticates via
//! `sakha login` instead of setting the env var themselves.

/// Service name all Sakha credentials are stored under in the OS credential
/// manager (Windows Credential Manager / macOS Keychain / Secret Service).
const SERVICE: &str = "sakha";

/// Storage backend for provider API keys, keyed by catalog preset id.
/// Abstracted behind a trait so unit tests use an in-memory double instead of
/// touching the real OS credential manager.
pub trait SecretBackend {
    /// Returns the stored secret for `preset_id`, if any.
    fn get(&self, preset_id: &str) -> anyhow::Result<Option<String>>;
    /// Stores (overwriting any existing) secret for `preset_id`.
    fn set(&self, preset_id: &str, secret: &str) -> anyhow::Result<()>;
    /// Deletes the stored secret for `preset_id`, if any. Not an error if
    /// nothing was stored.
    fn delete(&self, preset_id: &str) -> anyhow::Result<()>;
}

/// Real `SecretBackend` backed by the OS credential manager via the
/// `keyring` crate (Windows Credential Manager on Windows).
#[derive(Debug, Default)]
pub struct KeyringSecretBackend;

impl KeyringSecretBackend {
    pub fn new() -> Self {
        Self
    }

    fn entry(&self, preset_id: &str) -> anyhow::Result<keyring::Entry> {
        let account = format!("provider:{preset_id}");
        keyring::Entry::new(SERVICE, &account).map_err(|e| anyhow::anyhow!("failed to open keyring entry: {e}"))
    }
}

impl SecretBackend for KeyringSecretBackend {
    fn get(&self, preset_id: &str) -> anyhow::Result<Option<String>> {
        match self.entry(preset_id)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(anyhow::anyhow!("failed to read keyring entry: {e}")),
        }
    }

    fn set(&self, preset_id: &str, secret: &str) -> anyhow::Result<()> {
        self.entry(preset_id)?.set_password(secret).map_err(|e| anyhow::anyhow!("failed to write keyring entry: {e}"))
    }

    fn delete(&self, preset_id: &str) -> anyhow::Result<()> {
        match self.entry(preset_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(anyhow::anyhow!("failed to delete keyring entry: {e}")),
        }
    }
}

/// Masks a secret for safe display: never the raw value, at most the last 4
/// characters preceded by a bullet mask. Use this (never `{:?}`/`{}`) any
/// time a key might be printed or logged.
pub fn mask_secret(secret: &str) -> String {
    let tail: String = secret.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    if secret.chars().count() <= 4 {
        "\u{2022}\u{2022}\u{2022}".to_string()
    } else {
        format!("\u{2022}\u{2022}\u{2022}{tail}")
    }
}

#[cfg(test)]
pub mod test_support {
    //! In-memory `SecretBackend` for tests. Never touches the OS credential
    //! manager, so unit tests stay hermetic and safe to run in CI/sandboxes.
    use super::SecretBackend;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct InMemorySecretBackend {
        store: Mutex<HashMap<String, String>>,
    }

    impl InMemorySecretBackend {
        pub fn new() -> Self {
            Self::default()
        }
    }

    impl SecretBackend for InMemorySecretBackend {
        fn get(&self, preset_id: &str) -> anyhow::Result<Option<String>> {
            Ok(self.store.lock().unwrap().get(preset_id).cloned())
        }

        fn set(&self, preset_id: &str, secret: &str) -> anyhow::Result<()> {
            self.store.lock().unwrap().insert(preset_id.to_string(), secret.to_string());
            Ok(())
        }

        fn delete(&self, preset_id: &str) -> anyhow::Result<()> {
            self.store.lock().unwrap().remove(preset_id);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::InMemorySecretBackend;
    use super::*;

    #[test]
    fn in_memory_backend_round_trips_set_get() {
        let backend = InMemorySecretBackend::new();
        backend.set("openai", "sk-abc123").unwrap();
        assert_eq!(backend.get("openai").unwrap().as_deref(), Some("sk-abc123"));
    }

    #[test]
    fn in_memory_backend_get_missing_returns_none() {
        let backend = InMemorySecretBackend::new();
        assert!(backend.get("missing").unwrap().is_none());
    }

    #[test]
    fn in_memory_backend_delete_removes_entry() {
        let backend = InMemorySecretBackend::new();
        backend.set("openai", "sk-abc123").unwrap();
        backend.delete("openai").unwrap();
        assert!(backend.get("openai").unwrap().is_none());
    }

    #[test]
    fn in_memory_backend_delete_missing_is_not_an_error() {
        let backend = InMemorySecretBackend::new();
        assert!(backend.delete("missing").is_ok());
    }

    #[test]
    fn mask_secret_never_reveals_full_value() {
        let masked = mask_secret("sk-super-secret-value-12345");
        assert!(!masked.contains("super-secret"));
        assert!(masked.ends_with("2345"));
        assert!(masked.starts_with('\u{2022}'));
    }

    #[test]
    fn mask_secret_handles_short_values() {
        let masked = mask_secret("ab");
        assert!(!masked.contains("ab"));
    }
}
