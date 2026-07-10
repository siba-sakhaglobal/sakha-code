//! Secret handles and store abstraction. Secrets never enter model context;
//! tools receive `SecretRef` handles resolved only at the point of use.
//!
//! Per spec ("Secret Rules"): secrets are never inserted into model context,
//! tools receive handles rather than raw values where possible, and secret
//! access is auditable. `SecretRef` itself intentionally never carries a raw
//! value so it is always safe to log, serialize, or include in an audit
//! event.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

use sakha_core::SakhaError;

/// An opaque reference to a secret value. Never carries the raw value.
///
/// `env_var` optionally names the environment variable this ref resolves
/// against for `EnvSecretStore`; when absent, the store falls back to
/// treating `key` itself as the environment variable name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretRef {
    pub key: String,
    pub env_var: Option<String>,
}

impl SecretRef {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            env_var: None,
        }
    }

    /// Builds a secret ref explicitly bound to an environment variable name.
    pub fn from_env_var(key: impl Into<String>, env_var: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            env_var: Some(env_var.into()),
        }
    }

    fn resolved_env_var(&self) -> &str {
        self.env_var.as_deref().unwrap_or(&self.key)
    }
}

/// One audited secret access.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretAccessRecord {
    pub key: String,
    pub action: SecretAction,
    pub found: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretAction {
    Get,
    Set,
    Delete,
    List,
}

/// Abstraction over a secret backend (env vars, OS keychain, vault). All
/// methods are async since real backends may involve I/O.
#[async_trait]
pub trait SecretStore: Send + Sync {
    async fn get(&self, secret_ref: &SecretRef) -> Result<Option<String>, SakhaError>;
    async fn set(&self, secret_ref: &SecretRef, value: &str) -> Result<(), SakhaError>;
    async fn delete(&self, secret_ref: &SecretRef) -> Result<(), SakhaError>;
    async fn list(&self) -> Result<Vec<SecretRef>, SakhaError>;

    /// Returns the audit trail of accesses made through this store, if it
    /// tracks one. Default implementation returns an empty log for stores
    /// that don't support auditing.
    fn access_log(&self) -> Vec<SecretAccessRecord> {
        Vec::new()
    }
}

/// A local, in-memory secret store fallback. Never persists to disk; suitable
/// for tests and as a safe default when no real backend is configured. Every
/// access is appended to an in-memory audit log.
#[derive(Debug, Default)]
pub struct InMemorySecretStore {
    values: Mutex<HashMap<String, String>>,
    log: Mutex<Vec<SecretAccessRecord>>,
}

impl InMemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an access, tolerating a poisoned lock: a panic during some
    /// unrelated access must not make the audit log itself unusable, since
    /// losing the ability to audit secret access is worse than recovering
    /// the (still-valid) data behind a poisoned lock. See spec "provide
    /// auditable safety" (secret access must stay auditable/recoverable, not
    /// crash the process).
    fn record(&self, key: &str, action: SecretAction, found: bool) {
        let mut log = self.log.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        log.push(SecretAccessRecord {
            key: key.to_string(),
            action,
            found,
        });
    }
}

#[async_trait]
impl SecretStore for InMemorySecretStore {
    async fn get(&self, secret_ref: &SecretRef) -> Result<Option<String>, SakhaError> {
        let value = self
            .values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&secret_ref.key)
            .cloned();
        self.record(&secret_ref.key, SecretAction::Get, value.is_some());
        Ok(value)
    }

    async fn set(&self, secret_ref: &SecretRef, value: &str) -> Result<(), SakhaError> {
        self.values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(secret_ref.key.clone(), value.to_string());
        self.record(&secret_ref.key, SecretAction::Set, true);
        Ok(())
    }

    async fn delete(&self, secret_ref: &SecretRef) -> Result<(), SakhaError> {
        let existed = self
            .values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&secret_ref.key)
            .is_some();
        self.record(&secret_ref.key, SecretAction::Delete, existed);
        Ok(())
    }

    async fn list(&self) -> Result<Vec<SecretRef>, SakhaError> {
        let refs = self
            .values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .keys()
            .map(|k| SecretRef::new(k.clone()))
            .collect();
        self.record("*", SecretAction::List, true);
        Ok(refs)
    }

    fn access_log(&self) -> Vec<SecretAccessRecord> {
        self.log.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }
}

/// An environment-variable-backed secret store. `set`/`delete` only affect
/// the current process's environment (not persisted across runs), which
/// matches the "env-ref based" secret model: the real source of truth is
/// whatever set the process environment (shell, `.env` loader, CI secrets),
/// and this store just resolves references against it at point of use.
#[derive(Debug, Default)]
pub struct EnvSecretStore {
    log: Mutex<Vec<SecretAccessRecord>>,
}

impl EnvSecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn record(&self, key: &str, action: SecretAction, found: bool) {
        self.log.lock().unwrap().push(SecretAccessRecord {
            key: key.to_string(),
            action,
            found,
        });
    }
}

#[async_trait]
impl SecretStore for EnvSecretStore {
    async fn get(&self, secret_ref: &SecretRef) -> Result<Option<String>, SakhaError> {
        let value = std::env::var(secret_ref.resolved_env_var()).ok();
        self.record(&secret_ref.key, SecretAction::Get, value.is_some());
        Ok(value)
    }

    async fn set(&self, secret_ref: &SecretRef, value: &str) -> Result<(), SakhaError> {
        // SAFETY (semantic, not memory-unsafety): mutating the process
        // environment affects all threads reading it concurrently; acceptable
        // here since this is an explicit, user-directed secret management
        // operation rather than incidental config reading.
        std::env::set_var(secret_ref.resolved_env_var(), value);
        self.record(&secret_ref.key, SecretAction::Set, true);
        Ok(())
    }

    async fn delete(&self, secret_ref: &SecretRef) -> Result<(), SakhaError> {
        let existed = std::env::var(secret_ref.resolved_env_var()).is_ok();
        std::env::remove_var(secret_ref.resolved_env_var());
        self.record(&secret_ref.key, SecretAction::Delete, existed);
        Ok(())
    }

    async fn list(&self) -> Result<Vec<SecretRef>, SakhaError> {
        // Enumerating the entire process environment as "secrets" would be
        // both noisy and unsafe to expose; env-backed stores only resolve
        // refs the caller already knows the name of.
        Err(SakhaError::not_implemented(
            "sakha-security",
            "EnvSecretStore::list is intentionally unsupported; env stores are resolve-only",
        ))
    }

    fn access_log(&self) -> Vec<SecretAccessRecord> {
        self.log.lock().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_store_roundtrips_and_audits() {
        let store = InMemorySecretStore::new();
        let secret_ref = SecretRef::new("api_key");
        store.set(&secret_ref, "raw-value").await.unwrap();
        let got = store.get(&secret_ref).await.unwrap();
        assert_eq!(got.as_deref(), Some("raw-value"));

        let log = store.access_log();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].action, SecretAction::Set);
        assert_eq!(log[1].action, SecretAction::Get);
        assert!(log[1].found);
    }

    #[tokio::test]
    async fn in_memory_store_delete_removes_value() {
        let store = InMemorySecretStore::new();
        let secret_ref = SecretRef::new("token");
        store.set(&secret_ref, "v").await.unwrap();
        store.delete(&secret_ref).await.unwrap();
        assert!(store.get(&secret_ref).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn env_store_resolves_from_process_environment() {
        std::env::set_var("SAKHA_TEST_SECRET_XYZ", "env-value");
        let store = EnvSecretStore::new();
        let secret_ref = SecretRef::from_env_var("xyz", "SAKHA_TEST_SECRET_XYZ");
        let got = store.get(&secret_ref).await.unwrap();
        assert_eq!(got.as_deref(), Some("env-value"));
        std::env::remove_var("SAKHA_TEST_SECRET_XYZ");
    }

    #[tokio::test]
    async fn in_memory_store_survives_poisoned_values_lock() {
        use std::sync::Arc;
        let store = Arc::new(InMemorySecretStore::new());
        let secret_ref = SecretRef::new("api_key");
        store.set(&secret_ref, "raw-value").await.unwrap();

        // Poison the `values` mutex by panicking while holding it.
        let poisoner = Arc::clone(&store);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.values.lock().unwrap();
            panic!("intentional poison for test");
        })
        .join();
        assert!(store.values.is_poisoned());

        // Access after poisoning must recover rather than panic.
        let got = store.get(&secret_ref).await.unwrap();
        assert_eq!(got.as_deref(), Some("raw-value"));
    }

    #[tokio::test]
    async fn in_memory_store_survives_poisoned_log_lock() {
        use std::sync::Arc;
        let store = Arc::new(InMemorySecretStore::new());

        let poisoner = Arc::clone(&store);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.log.lock().unwrap();
            panic!("intentional poison for test");
        })
        .join();
        assert!(store.log.is_poisoned());

        let secret_ref = SecretRef::new("token");
        store.set(&secret_ref, "v").await.unwrap();
        let log = store.access_log();
        assert!(log.iter().any(|r| r.key == "token" && r.action == SecretAction::Set));
    }

    #[test]
    fn secret_ref_serializes_without_value() {
        let secret_ref = SecretRef::new("api_key");
        let json = serde_json::to_string(&secret_ref).unwrap();
        assert!(json.contains("api_key"));
        // No raw value field exists on SecretRef at all, so there is nothing
        // to accidentally serialize — this test documents that invariant.
        assert!(!json.to_lowercase().contains("value"));
    }
}
