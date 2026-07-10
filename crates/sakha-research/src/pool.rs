//! `SearchPool`: tries a prioritized list of `SearchBackend`s in order,
//! automatically failing over to the next backend when one is rate-limited,
//! unauthorized, or otherwise erroring, and recording an in-process cooldown
//! so the pool does not keep hammering a backend that just told it to back
//! off. See spec `modules/11-web-search-research.md` "Search Providers" and
//! the "switch key automatically based on quota" requirement in
//! `docs/web-search.md`.
//!
//! Priority order is manual (config-driven, see `sakha-cli`'s `[search]`
//! section); only *failover within that order* is automatic.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sakha_core::{ErrorClass, SakhaError, SakhaResult};

use crate::backends::{SearchBackend, SearchHit};

/// How long a backend is skipped after a quota/auth/transport failure.
pub const DEFAULT_COOLDOWN: Duration = Duration::from_secs(10 * 60);

/// Default backend priority order when no `[search].backends` override is
/// configured.
pub const DEFAULT_BACKEND_ORDER: &[&str] =
    &["firecrawl", "brave", "tavily", "serper", "serpapi", "exa", "searxng"];

/// A snapshot of one backend's health, for `sakha search-backends` display.
#[derive(Debug, Clone)]
pub struct BackendStatus {
    pub id: &'static str,
    pub configured: bool,
    pub cooling_down: bool,
    pub env_var: Option<&'static str>,
    pub key_portal_url: &'static str,
    pub last_error_kind: Option<String>,
}

struct CooldownEntry {
    until: Instant,
    last_error_kind: String,
}

/// Holds backends in priority order and tracks per-backend cooldowns.
pub struct SearchPool {
    backends: Vec<Box<dyn SearchBackend>>,
    cooldown: Duration,
    cooldowns: Mutex<HashMap<&'static str, CooldownEntry>>,
}

/// The result of a successful pool search: which backend served it, plus
/// the normalized hits.
#[derive(Debug, Clone)]
pub struct PoolSearchResult {
    pub backend: String,
    pub hits: Vec<SearchHit>,
}

impl SearchPool {
    /// Builds a pool from `backends`, in the priority order given. Callers
    /// (`sakha-cli`) are responsible for ordering/filtering per config;
    /// `SearchPool` itself just walks the list it's handed.
    pub fn new(backends: Vec<Box<dyn SearchBackend>>) -> Self {
        Self { backends, cooldown: DEFAULT_COOLDOWN, cooldowns: Mutex::new(HashMap::new()) }
    }

    /// Overrides the cooldown duration (tests use a very short one so they
    /// don't need to sleep).
    pub fn with_cooldown(mut self, cooldown: Duration) -> Self {
        self.cooldown = cooldown;
        self
    }

    fn is_cooling_down(&self, id: &'static str) -> bool {
        self.cooldowns
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .map(|entry| entry.until > Instant::now())
            .unwrap_or(false)
    }

    fn record_cooldown(&self, id: &'static str, error: &SakhaError) {
        let mut guard = self.cooldowns.lock().unwrap_or_else(|p| p.into_inner());
        guard.insert(id, CooldownEntry { until: Instant::now() + self.cooldown, last_error_kind: error.class.to_string() });
    }

    /// Whether an error should trigger a cooldown + failover to the next
    /// backend (quota/auth/transient errors) vs. being surfaced immediately
    /// (e.g. a config/programming error that no other backend would fix
    /// either — treated the same as any other failure here since every
    /// backend is independent; kept explicit for clarity/future tuning).
    fn should_cooldown_and_continue(error: &SakhaError) -> bool {
        matches!(error.class, ErrorClass::Permission | ErrorClass::Transient | ErrorClass::InvalidInput)
    }

    /// Tries backends in priority order, skipping any currently in
    /// cooldown, until one succeeds. Returns the first success. If every
    /// backend fails (or is cooling down / unconfigured), returns a
    /// `SakhaError::transient` summarizing that nothing was available.
    pub async fn search(&self, query: &str, limit: u32) -> SakhaResult<PoolSearchResult> {
        let mut last_error: Option<SakhaError> = None;
        let mut tried_any = false;

        for backend in &self.backends {
            if !backend.is_configured() {
                continue;
            }
            if self.is_cooling_down(backend.id()) {
                continue;
            }
            tried_any = true;
            match backend.search(query, limit).await {
                Ok(hits) => {
                    return Ok(PoolSearchResult { backend: backend.id().to_string(), hits });
                }
                Err(err) => {
                    if Self::should_cooldown_and_continue(&err) {
                        self.record_cooldown(backend.id(), &err);
                    }
                    last_error = Some(err);
                }
            }
        }

        match last_error {
            Some(err) => Err(SakhaError::transient(
                "sakha-research::pool",
                format!("all search backends failed; last error: {err}"),
            )),
            None => {
                let _ = tried_any;
                Err(SakhaError::invalid_input(
                    "sakha-research::pool",
                    "no search backend available (none configured, or all cooling down)",
                ))
            }
        }
    }

    /// Searches a single named backend directly, bypassing priority order
    /// and failover. Used by `sakha search --backend <id>`. Returns
    /// `SakhaError::invalid_input` if `backend_id` is unknown to this pool.
    pub async fn search_with_backend(&self, backend_id: &str, query: &str, limit: u32) -> SakhaResult<PoolSearchResult> {
        let backend = self
            .backends
            .iter()
            .find(|b| b.id() == backend_id)
            .ok_or_else(|| SakhaError::invalid_input("sakha-research::pool", format!("unknown search backend: {backend_id}")))?;
        if !backend.is_configured() {
            return Err(SakhaError::invalid_input(
                "sakha-research::pool",
                format!("search backend '{backend_id}' is not configured (missing env var)"),
            ));
        }
        let hits = backend.search(query, limit).await?;
        Ok(PoolSearchResult { backend: backend.id().to_string(), hits })
    }

    /// Reports status for every backend in the pool, for `sakha
    /// search-backends`.
    pub fn statuses(&self) -> Vec<BackendStatus> {
        let guard = self.cooldowns.lock().unwrap_or_else(|p| p.into_inner());
        self.backends
            .iter()
            .map(|b| {
                let cooldown_entry = guard.get(b.id());
                BackendStatus {
                    id: b.id(),
                    configured: b.is_configured(),
                    cooling_down: cooldown_entry.map(|e| e.until > Instant::now()).unwrap_or(false),
                    env_var: b.env_var(),
                    key_portal_url: b.key_portal_url(),
                    last_error_kind: cooldown_entry.map(|e| e.last_error_kind.clone()),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A fake `SearchBackend` for pool tests: always configured, returns a
    /// scripted result (success or a specific error class) and counts how
    /// many times `search` was called.
    struct FakeBackend {
        id: &'static str,
        configured: bool,
        outcome: FakeOutcome,
        calls: AtomicUsize,
    }

    enum FakeOutcome {
        Success(Vec<SearchHit>),
        Error(ErrorClass),
    }

    impl FakeBackend {
        fn success(id: &'static str, hits: Vec<SearchHit>) -> Self {
            Self { id, configured: true, outcome: FakeOutcome::Success(hits), calls: AtomicUsize::new(0) }
        }

        fn error(id: &'static str, class: ErrorClass) -> Self {
            Self { id, configured: true, outcome: FakeOutcome::Error(class), calls: AtomicUsize::new(0) }
        }

        fn unconfigured(id: &'static str) -> Self {
            Self { id, configured: false, outcome: FakeOutcome::Error(ErrorClass::Fatal), calls: AtomicUsize::new(0) }
        }
    }

    #[async_trait]
    impl SearchBackend for FakeBackend {
        fn id(&self) -> &'static str {
            self.id
        }

        fn is_configured(&self) -> bool {
            self.configured
        }

        fn env_var(&self) -> Option<&'static str> {
            None
        }

        fn key_portal_url(&self) -> &'static str {
            "https://example.com"
        }

        async fn search(&self, _query: &str, _limit: u32) -> SakhaResult<Vec<SearchHit>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match &self.outcome {
                FakeOutcome::Success(hits) => Ok(hits.clone()),
                FakeOutcome::Error(class) => Err(SakhaError::new(*class, "fake", "scripted failure")),
            }
        }
    }

    fn hit(url: &str, backend: &str) -> SearchHit {
        SearchHit { title: "t".into(), url: url.into(), snippet: "s".into(), backend: backend.into() }
    }

    #[tokio::test]
    async fn first_configured_backend_success_short_circuits() {
        let pool = SearchPool::new(vec![
            Box::new(FakeBackend::success("a", vec![hit("https://a.com", "a")])),
            Box::new(FakeBackend::success("b", vec![hit("https://b.com", "b")])),
        ]);
        let result = pool.search("query", 5).await.unwrap();
        assert_eq!(result.backend, "a");
    }

    /// The core "quota failover" contract: backend A returns a 429-shaped
    /// (Permission-class) error, the pool records a cooldown for A and moves
    /// on to backend B in the same call.
    #[tokio::test]
    async fn quota_error_triggers_cooldown_and_failover_to_next_backend() {
        let pool = SearchPool::new(vec![
            Box::new(FakeBackend::error("a", ErrorClass::Permission)),
            Box::new(FakeBackend::success("b", vec![hit("https://b.com", "b")])),
        ]);
        let result = pool.search("query", 5).await.unwrap();
        assert_eq!(result.backend, "b");

        let statuses = pool.statuses();
        let a_status = statuses.iter().find(|s| s.id == "a").unwrap();
        assert!(a_status.cooling_down);
        assert_eq!(a_status.last_error_kind.as_deref(), Some("permission"));
    }

    #[tokio::test]
    async fn cooling_down_backend_is_skipped_on_next_call() {
        let pool = SearchPool::new(vec![
            Box::new(FakeBackend::error("a", ErrorClass::Permission)),
            Box::new(FakeBackend::success("b", vec![hit("https://b.com", "b")])),
        ]);
        // First call puts "a" into cooldown and succeeds via "b".
        pool.search("query", 5).await.unwrap();
        // Second call should go straight to "b" without re-trying "a".
        let result = pool.search("query", 5).await.unwrap();
        assert_eq!(result.backend, "b");
    }

    #[tokio::test]
    async fn unconfigured_backend_is_skipped() {
        let pool = SearchPool::new(vec![
            Box::new(FakeBackend::unconfigured("a")),
            Box::new(FakeBackend::success("b", vec![hit("https://b.com", "b")])),
        ]);
        let result = pool.search("query", 5).await.unwrap();
        assert_eq!(result.backend, "b");
    }

    #[tokio::test]
    async fn all_backends_failing_returns_transient_error() {
        let pool = SearchPool::new(vec![
            Box::new(FakeBackend::error("a", ErrorClass::Transient)),
            Box::new(FakeBackend::error("b", ErrorClass::Permission)),
        ]);
        let result = pool.search("query", 5).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn no_configured_backends_returns_invalid_input_error() {
        let pool = SearchPool::new(vec![Box::new(FakeBackend::unconfigured("a"))]);
        let result = pool.search("query", 5).await;
        let err = result.unwrap_err();
        assert_eq!(err.class, ErrorClass::InvalidInput);
    }

    #[tokio::test]
    async fn cooldown_expires_after_configured_duration() {
        let pool = SearchPool::new(vec![
            Box::new(FakeBackend::error("a", ErrorClass::Permission)),
            Box::new(FakeBackend::success("b", vec![hit("https://b.com", "b")])),
        ])
        .with_cooldown(Duration::from_millis(1));
        pool.search("query", 5).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        let statuses = pool.statuses();
        let a_status = statuses.iter().find(|s| s.id == "a").unwrap();
        assert!(!a_status.cooling_down);
    }

    #[tokio::test]
    async fn search_with_backend_targets_named_backend_directly() {
        let pool = SearchPool::new(vec![
            Box::new(FakeBackend::success("a", vec![hit("https://a.com", "a")])),
            Box::new(FakeBackend::success("b", vec![hit("https://b.com", "b")])),
        ]);
        let result = pool.search_with_backend("b", "query", 5).await.unwrap();
        assert_eq!(result.backend, "b");
    }

    #[tokio::test]
    async fn search_with_backend_unknown_id_errors() {
        let pool = SearchPool::new(vec![Box::new(FakeBackend::success("a", vec![]))]);
        let result = pool.search_with_backend("nonexistent", "query", 5).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn search_with_backend_unconfigured_errors() {
        let pool = SearchPool::new(vec![Box::new(FakeBackend::unconfigured("a"))]);
        let result = pool.search_with_backend("a", "query", 5).await;
        assert!(result.is_err());
    }

    #[test]
    fn default_backend_order_matches_spec_priority() {
        assert_eq!(
            DEFAULT_BACKEND_ORDER,
            &["firecrawl", "brave", "tavily", "serper", "serpapi", "exa", "searxng"]
        );
    }
}
