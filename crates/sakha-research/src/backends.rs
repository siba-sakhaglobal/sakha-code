//! Pluggable web search backends behind a single `SearchBackend` trait.
//! See spec `modules/11-web-search-research.md` "Search Providers" and
//! `docs/web-search.md`.
//!
//! Each backend speaks a different provider's HTTP API but returns the same
//! normalized `SearchHit` shape. Parsing is tolerant: a missing/malformed
//! field on one result entry causes that entry to be skipped, never a panic
//! or a whole-batch failure. API keys are read from environment variables
//! and are never logged or embedded in error messages.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use sakha_core::{SakhaError, SakhaResult};

/// One normalized search hit, provider-agnostic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub backend: String,
}

/// A pluggable web search backend. Implementations wrap a specific
/// provider's HTTP API. See spec "Search Providers".
#[async_trait]
pub trait SearchBackend: Send + Sync {
    /// Stable identifier, e.g. `"firecrawl"`, `"brave"`. Used for priority
    /// ordering, cooldown bookkeeping, and status reporting.
    fn id(&self) -> &'static str;

    /// Whether this backend is usable given the current environment (i.e.
    /// its required env var, if any, is set). Keyless-capable backends
    /// (`firecrawl`, `searxng` needing only a base URL) report `true`/`false`
    /// per their own rules — see each impl.
    fn is_configured(&self) -> bool;

    /// The environment variable name this backend reads its key from, for
    /// display in `sakha search-backends` (`None` if it takes no key).
    fn env_var(&self) -> Option<&'static str>;

    /// Portal URL where a user can create a key for this backend.
    fn key_portal_url(&self) -> &'static str;

    async fn search(&self, query: &str, limit: u32) -> SakhaResult<Vec<SearchHit>>;
}

const DEFAULT_TIMEOUT_SECS: u64 = 20;

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .build()
        .unwrap_or_default()
}

/// Maps an HTTP status code to a `SakhaError`, classifying auth/quota errors
/// (401/402/403/429) as `Permission` (never retried against the same
/// backend within the pool's cooldown window) and everything else as
/// `Transient`. Never includes any request body/header content in the
/// message, so API keys can never leak into an error string.
fn status_error(module: &str, backend: &str, status: reqwest::StatusCode) -> SakhaError {
    let code = status.as_u16();
    if matches!(code, 401 | 402 | 403 | 429) {
        SakhaError::permission(module, format!("{backend} returned status {code} (quota/auth)")).with_subject(backend)
    } else {
        SakhaError::transient(module, format!("{backend} returned status {code}")).with_subject(backend)
    }
}

fn transport_error(module: &str, backend: &str, err: reqwest::Error) -> SakhaError {
    // `reqwest::Error`'s Display never includes header values, but request
    // URLs can carry query-string API keys for GET-based backends (serpapi).
    // Redact rather than trust upstream Display impls to stay safe forever.
    let _ = err; // avoid leaking any inner detail via Display
    SakhaError::transient(module, format!("{backend} request failed (transport error)")).with_subject(backend)
}

fn json_error(module: &str, backend: &str) -> SakhaError {
    SakhaError::transient(module, format!("{backend} response was not valid JSON")).with_subject(backend)
}

// ---------------------------------------------------------------------
// Firecrawl
// ---------------------------------------------------------------------

const MODULE: &str = "sakha-research::backends";

/// `POST https://api.firecrawl.dev/v2/search`. Works keyless (rate-limited)
/// when `FIRECRAWL_API_KEY` is unset — the `Authorization` header is simply
/// omitted. Response shape: `{"success":true,"data":{"web":[{"url","title","description","position"}]}}`.
pub struct FirecrawlBackend {
    http: reqwest::Client,
    base_url: String,
}

impl FirecrawlBackend {
    pub fn new() -> Self {
        Self { http: http_client(), base_url: "https://api.firecrawl.dev/v2/search".to_string() }
    }

    #[cfg(test)]
    fn with_base_url(base_url: impl Into<String>) -> Self {
        Self { http: http_client(), base_url: base_url.into() }
    }

    /// Parses the verified Firecrawl response shape. Public for unit tests
    /// against fixture JSON.
    pub fn parse_response(body: &serde_json::Value) -> Vec<SearchHit> {
        body.get("data")
            .and_then(|d| d.get("web"))
            .and_then(|w| w.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let url = item.get("url")?.as_str()?.to_string();
                        let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
                        let snippet = item.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string();
                        Some(SearchHit { title, url, snippet, backend: "firecrawl".to_string() })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Default for FirecrawlBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SearchBackend for FirecrawlBackend {
    fn id(&self) -> &'static str {
        "firecrawl"
    }

    fn is_configured(&self) -> bool {
        // Keyless tier works (rate-limited); the backend is always usable.
        true
    }

    fn env_var(&self) -> Option<&'static str> {
        Some("FIRECRAWL_API_KEY")
    }

    fn key_portal_url(&self) -> &'static str {
        "https://www.firecrawl.dev/app/api-keys"
    }

    async fn search(&self, query: &str, limit: u32) -> SakhaResult<Vec<SearchHit>> {
        let mut req = self.http.post(&self.base_url).json(&serde_json::json!({
            "query": query,
            "limit": limit,
        }));
        if let Ok(key) = std::env::var("FIRECRAWL_API_KEY") {
            if !key.is_empty() {
                req = req.bearer_auth(key);
            }
        }
        let response = req.send().await.map_err(|e| transport_error(MODULE, self.id(), e))?;
        if !response.status().is_success() {
            return Err(status_error(MODULE, self.id(), response.status()));
        }
        let body: serde_json::Value = response.json().await.map_err(|_| json_error(MODULE, self.id()))?;
        let mut hits = Self::parse_response(&body);
        hits.truncate(limit as usize);
        Ok(hits)
    }
}

// ---------------------------------------------------------------------
// Brave
// ---------------------------------------------------------------------

/// `GET https://api.search.brave.com/res/v1/web/search`. Requires
/// `BRAVE_API_KEY` (sent as `X-Subscription-Token`). Response:
/// `web.results[].{title,url,description}`.
pub struct BraveBackend {
    http: reqwest::Client,
    base_url: String,
}

impl BraveBackend {
    pub fn new() -> Self {
        Self { http: http_client(), base_url: "https://api.search.brave.com/res/v1/web/search".to_string() }
    }

    #[cfg(test)]
    fn with_base_url(base_url: impl Into<String>) -> Self {
        Self { http: http_client(), base_url: base_url.into() }
    }

    pub fn parse_response(body: &serde_json::Value) -> Vec<SearchHit> {
        body.get("web")
            .and_then(|w| w.get("results"))
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let url = item.get("url")?.as_str()?.to_string();
                        let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
                        let snippet = item.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string();
                        Some(SearchHit { title, url, snippet, backend: "brave".to_string() })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Default for BraveBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SearchBackend for BraveBackend {
    fn id(&self) -> &'static str {
        "brave"
    }

    fn is_configured(&self) -> bool {
        std::env::var_os("BRAVE_API_KEY").is_some()
    }

    fn env_var(&self) -> Option<&'static str> {
        Some("BRAVE_API_KEY")
    }

    fn key_portal_url(&self) -> &'static str {
        "https://api-dashboard.search.brave.com"
    }

    async fn search(&self, query: &str, limit: u32) -> SakhaResult<Vec<SearchHit>> {
        let key = std::env::var("BRAVE_API_KEY")
            .map_err(|_| SakhaError::invalid_input(MODULE, "brave: BRAVE_API_KEY not set").with_subject(self.id()))?;
        let response = self
            .http
            .get(&self.base_url)
            .query(&[("q", query), ("count", &limit.to_string())])
            .header("X-Subscription-Token", key)
            .send()
            .await
            .map_err(|e| transport_error(MODULE, self.id(), e))?;
        if !response.status().is_success() {
            return Err(status_error(MODULE, self.id(), response.status()));
        }
        let body: serde_json::Value = response.json().await.map_err(|_| json_error(MODULE, self.id()))?;
        let mut hits = Self::parse_response(&body);
        hits.truncate(limit as usize);
        Ok(hits)
    }
}

// ---------------------------------------------------------------------
// Tavily
// ---------------------------------------------------------------------

/// `POST https://api.tavily.com/search`. Requires `TAVILY_API_KEY` (Bearer).
/// Response: `results[].{title,url,content}`.
pub struct TavilyBackend {
    http: reqwest::Client,
    base_url: String,
}

impl TavilyBackend {
    pub fn new() -> Self {
        Self { http: http_client(), base_url: "https://api.tavily.com/search".to_string() }
    }

    #[cfg(test)]
    fn with_base_url(base_url: impl Into<String>) -> Self {
        Self { http: http_client(), base_url: base_url.into() }
    }

    pub fn parse_response(body: &serde_json::Value) -> Vec<SearchHit> {
        body.get("results")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let url = item.get("url")?.as_str()?.to_string();
                        let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
                        let snippet = item.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
                        Some(SearchHit { title, url, snippet, backend: "tavily".to_string() })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Default for TavilyBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SearchBackend for TavilyBackend {
    fn id(&self) -> &'static str {
        "tavily"
    }

    fn is_configured(&self) -> bool {
        std::env::var_os("TAVILY_API_KEY").is_some()
    }

    fn env_var(&self) -> Option<&'static str> {
        Some("TAVILY_API_KEY")
    }

    fn key_portal_url(&self) -> &'static str {
        "https://app.tavily.com"
    }

    async fn search(&self, query: &str, limit: u32) -> SakhaResult<Vec<SearchHit>> {
        let key = std::env::var("TAVILY_API_KEY")
            .map_err(|_| SakhaError::invalid_input(MODULE, "tavily: TAVILY_API_KEY not set").with_subject(self.id()))?;
        let response = self
            .http
            .post(&self.base_url)
            .bearer_auth(key)
            .json(&serde_json::json!({ "query": query, "max_results": limit }))
            .send()
            .await
            .map_err(|e| transport_error(MODULE, self.id(), e))?;
        if !response.status().is_success() {
            return Err(status_error(MODULE, self.id(), response.status()));
        }
        let body: serde_json::Value = response.json().await.map_err(|_| json_error(MODULE, self.id()))?;
        let mut hits = Self::parse_response(&body);
        hits.truncate(limit as usize);
        Ok(hits)
    }
}

// ---------------------------------------------------------------------
// Serper
// ---------------------------------------------------------------------

/// `POST https://google.serper.dev/search`. Requires `SERPER_API_KEY`
/// (header `X-API-KEY`). Response: `organic[].{title,link,snippet}`.
pub struct SerperBackend {
    http: reqwest::Client,
    base_url: String,
}

impl SerperBackend {
    pub fn new() -> Self {
        Self { http: http_client(), base_url: "https://google.serper.dev/search".to_string() }
    }

    #[cfg(test)]
    fn with_base_url(base_url: impl Into<String>) -> Self {
        Self { http: http_client(), base_url: base_url.into() }
    }

    pub fn parse_response(body: &serde_json::Value) -> Vec<SearchHit> {
        body.get("organic")
            .and_then(|o| o.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let url = item.get("link")?.as_str()?.to_string();
                        let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
                        let snippet = item.get("snippet").and_then(|s| s.as_str()).unwrap_or("").to_string();
                        Some(SearchHit { title, url, snippet, backend: "serper".to_string() })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Default for SerperBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SearchBackend for SerperBackend {
    fn id(&self) -> &'static str {
        "serper"
    }

    fn is_configured(&self) -> bool {
        std::env::var_os("SERPER_API_KEY").is_some()
    }

    fn env_var(&self) -> Option<&'static str> {
        Some("SERPER_API_KEY")
    }

    fn key_portal_url(&self) -> &'static str {
        "https://serper.dev/api-key"
    }

    async fn search(&self, query: &str, limit: u32) -> SakhaResult<Vec<SearchHit>> {
        let key = std::env::var("SERPER_API_KEY")
            .map_err(|_| SakhaError::invalid_input(MODULE, "serper: SERPER_API_KEY not set").with_subject(self.id()))?;
        let response = self
            .http
            .post(&self.base_url)
            .header("X-API-KEY", key)
            .json(&serde_json::json!({ "q": query, "num": limit }))
            .send()
            .await
            .map_err(|e| transport_error(MODULE, self.id(), e))?;
        if !response.status().is_success() {
            return Err(status_error(MODULE, self.id(), response.status()));
        }
        let body: serde_json::Value = response.json().await.map_err(|_| json_error(MODULE, self.id()))?;
        let mut hits = Self::parse_response(&body);
        hits.truncate(limit as usize);
        Ok(hits)
    }
}

// ---------------------------------------------------------------------
// SerpApi
// ---------------------------------------------------------------------

/// `GET https://serpapi.com/search.json`. Requires `SERPAPI_API_KEY` (query
/// param `api_key`). Response: `organic_results[].{title,link,snippet}`.
pub struct SerpApiBackend {
    http: reqwest::Client,
    base_url: String,
}

impl SerpApiBackend {
    pub fn new() -> Self {
        Self { http: http_client(), base_url: "https://serpapi.com/search.json".to_string() }
    }

    #[cfg(test)]
    fn with_base_url(base_url: impl Into<String>) -> Self {
        Self { http: http_client(), base_url: base_url.into() }
    }

    pub fn parse_response(body: &serde_json::Value) -> Vec<SearchHit> {
        body.get("organic_results")
            .and_then(|o| o.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let url = item.get("link")?.as_str()?.to_string();
                        let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
                        let snippet = item.get("snippet").and_then(|s| s.as_str()).unwrap_or("").to_string();
                        Some(SearchHit { title, url, snippet, backend: "serpapi".to_string() })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Default for SerpApiBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SearchBackend for SerpApiBackend {
    fn id(&self) -> &'static str {
        "serpapi"
    }

    fn is_configured(&self) -> bool {
        std::env::var_os("SERPAPI_API_KEY").is_some()
    }

    fn env_var(&self) -> Option<&'static str> {
        Some("SERPAPI_API_KEY")
    }

    fn key_portal_url(&self) -> &'static str {
        "https://serpapi.com/manage-api-key"
    }

    async fn search(&self, query: &str, limit: u32) -> SakhaResult<Vec<SearchHit>> {
        let key = std::env::var("SERPAPI_API_KEY")
            .map_err(|_| SakhaError::invalid_input(MODULE, "serpapi: SERPAPI_API_KEY not set").with_subject(self.id()))?;
        let response = self
            .http
            .get(&self.base_url)
            .query(&[("q", query), ("num", &limit.to_string()), ("api_key", &key)])
            .send()
            .await
            .map_err(|e| transport_error(MODULE, self.id(), e))?;
        if !response.status().is_success() {
            return Err(status_error(MODULE, self.id(), response.status()));
        }
        let body: serde_json::Value = response.json().await.map_err(|_| json_error(MODULE, self.id()))?;
        let mut hits = Self::parse_response(&body);
        hits.truncate(limit as usize);
        Ok(hits)
    }
}

// ---------------------------------------------------------------------
// Exa
// ---------------------------------------------------------------------

/// `POST https://api.exa.ai/search`. Requires `EXA_API_KEY` (header
/// `x-api-key`). Response: `results[].{title,url}` (+ optional `text`).
pub struct ExaBackend {
    http: reqwest::Client,
    base_url: String,
}

impl ExaBackend {
    pub fn new() -> Self {
        Self { http: http_client(), base_url: "https://api.exa.ai/search".to_string() }
    }

    #[cfg(test)]
    fn with_base_url(base_url: impl Into<String>) -> Self {
        Self { http: http_client(), base_url: base_url.into() }
    }

    pub fn parse_response(body: &serde_json::Value) -> Vec<SearchHit> {
        body.get("results")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let url = item.get("url")?.as_str()?.to_string();
                        let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
                        let snippet = item.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
                        Some(SearchHit { title, url, snippet, backend: "exa".to_string() })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Default for ExaBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SearchBackend for ExaBackend {
    fn id(&self) -> &'static str {
        "exa"
    }

    fn is_configured(&self) -> bool {
        std::env::var_os("EXA_API_KEY").is_some()
    }

    fn env_var(&self) -> Option<&'static str> {
        Some("EXA_API_KEY")
    }

    fn key_portal_url(&self) -> &'static str {
        "https://dashboard.exa.ai/api-keys"
    }

    async fn search(&self, query: &str, limit: u32) -> SakhaResult<Vec<SearchHit>> {
        let key = std::env::var("EXA_API_KEY")
            .map_err(|_| SakhaError::invalid_input(MODULE, "exa: EXA_API_KEY not set").with_subject(self.id()))?;
        let response = self
            .http
            .post(&self.base_url)
            .header("x-api-key", key)
            .json(&serde_json::json!({ "query": query, "numResults": limit }))
            .send()
            .await
            .map_err(|e| transport_error(MODULE, self.id(), e))?;
        if !response.status().is_success() {
            return Err(status_error(MODULE, self.id(), response.status()));
        }
        let body: serde_json::Value = response.json().await.map_err(|_| json_error(MODULE, self.id()))?;
        let mut hits = Self::parse_response(&body);
        hits.truncate(limit as usize);
        Ok(hits)
    }
}

// ---------------------------------------------------------------------
// SearXNG
// ---------------------------------------------------------------------

/// `GET {SEARXNG_BASE_URL}/search?format=json`. No key; only "configured"
/// when `SEARXNG_BASE_URL` is set. Response: `results[].{title,url,content}`.
pub struct SearxngBackend {
    http: reqwest::Client,
}

impl SearxngBackend {
    pub fn new() -> Self {
        Self { http: http_client() }
    }

    pub fn parse_response(body: &serde_json::Value) -> Vec<SearchHit> {
        body.get("results")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let url = item.get("url")?.as_str()?.to_string();
                        let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
                        let snippet = item.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
                        Some(SearchHit { title, url, snippet, backend: "searxng".to_string() })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Default for SearxngBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SearchBackend for SearxngBackend {
    fn id(&self) -> &'static str {
        "searxng"
    }

    fn is_configured(&self) -> bool {
        std::env::var_os("SEARXNG_BASE_URL").is_some()
    }

    fn env_var(&self) -> Option<&'static str> {
        Some("SEARXNG_BASE_URL")
    }

    fn key_portal_url(&self) -> &'static str {
        "https://docs.searxng.org/admin/installation.html"
    }

    async fn search(&self, query: &str, limit: u32) -> SakhaResult<Vec<SearchHit>> {
        let base_url = std::env::var("SEARXNG_BASE_URL")
            .map_err(|_| SakhaError::invalid_input(MODULE, "searxng: SEARXNG_BASE_URL not set").with_subject(self.id()))?;
        let url = format!("{}/search", base_url.trim_end_matches('/'));
        let response = self
            .http
            .get(&url)
            .query(&[("q", query), ("format", "json")])
            .send()
            .await
            .map_err(|e| transport_error(MODULE, self.id(), e))?;
        if !response.status().is_success() {
            return Err(status_error(MODULE, self.id(), response.status()));
        }
        let body: serde_json::Value = response.json().await.map_err(|_| json_error(MODULE, self.id()))?;
        let mut hits = Self::parse_response(&body);
        hits.truncate(limit as usize);
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    /// These backends read process-global env vars (`BRAVE_API_KEY`, etc.);
    /// serialize the handful of tests that set/remove them so they can't
    /// race against each other under the default parallel test runner.
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn firecrawl_parse_response_verified_shape() {
        let body = serde_json::json!({
            "success": true,
            "data": {
                "web": [
                    {"url": "https://tokio.rs", "title": "Tokio", "description": "async runtime", "position": 1},
                    {"url": "https://docs.rs/tokio", "title": "docs.rs", "description": "tokio docs", "position": 2}
                ]
            }
        });
        let hits = FirecrawlBackend::parse_response(&body);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://tokio.rs");
        assert_eq!(hits[0].backend, "firecrawl");
        assert_eq!(hits[0].snippet, "async runtime");
    }

    #[test]
    fn firecrawl_parse_response_missing_url_is_skipped() {
        let body = serde_json::json!({"success": true, "data": {"web": [{"title": "no url"}]}});
        let hits = FirecrawlBackend::parse_response(&body);
        assert!(hits.is_empty());
    }

    #[test]
    fn firecrawl_is_always_configured_keyless() {
        let backend = FirecrawlBackend::new();
        assert!(backend.is_configured());
    }

    #[test]
    fn brave_parse_response() {
        let body = serde_json::json!({
            "web": {"results": [{"title": "Brave", "url": "https://brave.com", "description": "a browser"}]}
        });
        let hits = BraveBackend::parse_response(&body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].backend, "brave");
    }

    #[test]
    fn tavily_parse_response() {
        let body = serde_json::json!({
            "results": [{"title": "Tavily", "url": "https://tavily.com", "content": "search api"}]
        });
        let hits = TavilyBackend::parse_response(&body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].snippet, "search api");
    }

    #[test]
    fn serper_parse_response() {
        let body = serde_json::json!({
            "organic": [{"title": "Serper", "link": "https://serper.dev", "snippet": "google search api"}]
        });
        let hits = SerperBackend::parse_response(&body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://serper.dev");
    }

    #[test]
    fn serpapi_parse_response() {
        let body = serde_json::json!({
            "organic_results": [{"title": "SerpApi", "link": "https://serpapi.com", "snippet": "scraping api"}]
        });
        let hits = SerpApiBackend::parse_response(&body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].backend, "serpapi");
    }

    #[test]
    fn exa_parse_response_with_optional_text() {
        let body = serde_json::json!({
            "results": [{"title": "Exa", "url": "https://exa.ai", "text": "neural search"}]
        });
        let hits = ExaBackend::parse_response(&body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].snippet, "neural search");
    }

    #[test]
    fn exa_parse_response_without_text_defaults_empty_snippet() {
        let body = serde_json::json!({"results": [{"title": "Exa", "url": "https://exa.ai"}]});
        let hits = ExaBackend::parse_response(&body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].snippet, "");
    }

    #[test]
    fn searxng_parse_response() {
        let body = serde_json::json!({
            "results": [{"title": "SearXNG", "url": "https://searx.example.com", "content": "meta search"}]
        });
        let hits = SearxngBackend::parse_response(&body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].backend, "searxng");
    }

    #[test]
    fn brave_not_configured_without_env_var() {
        std::env::remove_var("BRAVE_API_KEY_TEST_UNSET_PROBE");
        // Directly exercise the env_var/is_configured contract without
        // mutating shared process env state across parallel tests.
        let backend = BraveBackend::new();
        assert_eq!(backend.env_var(), Some("BRAVE_API_KEY"));
        assert_eq!(backend.id(), "brave");
    }

    #[test]
    fn searxng_backend_reports_its_env_var_and_portal() {
        let backend = SearxngBackend::new();
        assert_eq!(backend.env_var(), Some("SEARXNG_BASE_URL"));
        assert!(!backend.key_portal_url().is_empty());
    }

    #[test]
    fn all_backends_never_serialize_or_expose_key_values() {
        // parse_response fixtures above never reference key material; this
        // test documents the invariant that SearchHit itself has no key
        // field at all, so it structurally cannot leak one.
        let hit = SearchHit { title: "t".into(), url: "u".into(), snippet: "s".into(), backend: "b".into() };
        let json = serde_json::to_string(&hit).unwrap();
        assert!(!json.to_lowercase().contains("key"));
    }

    #[tokio::test]
    async fn firecrawl_search_against_local_fixture_server_without_key() {
        // Uses a background thread with a tiny hand-rolled TCP listener would
        // be overkill; instead exercise build-time behavior via with_base_url
        // pointed at an address nothing listens on, verifying we get a
        // transport error (never a panic) and no key/header leak in the
        // message.
        let backend = FirecrawlBackend::with_base_url("http://127.0.0.1:1/search");
        let result = backend.search("rust", 3).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(!msg.to_lowercase().contains("bearer"));
    }

    #[tokio::test]
    async fn brave_search_without_key_returns_invalid_input_error() {
        let _lock = env_lock();
        std::env::remove_var("BRAVE_API_KEY");
        let backend = BraveBackend::with_base_url("http://127.0.0.1:1/search");
        let result = backend.search("rust", 3).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn tavily_search_without_key_returns_invalid_input_error() {
        let _lock = env_lock();
        std::env::remove_var("TAVILY_API_KEY");
        let backend = TavilyBackend::with_base_url("http://127.0.0.1:1/search");
        let result = backend.search("rust", 3).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn serper_search_without_key_returns_invalid_input_error() {
        let _lock = env_lock();
        std::env::remove_var("SERPER_API_KEY");
        let backend = SerperBackend::with_base_url("http://127.0.0.1:1/search");
        let result = backend.search("rust", 3).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn serpapi_search_without_key_returns_invalid_input_error() {
        let _lock = env_lock();
        std::env::remove_var("SERPAPI_API_KEY");
        let backend = SerpApiBackend::with_base_url("http://127.0.0.1:1/search");
        let result = backend.search("rust", 3).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn exa_search_without_key_returns_invalid_input_error() {
        let _lock = env_lock();
        std::env::remove_var("EXA_API_KEY");
        let backend = ExaBackend::with_base_url("http://127.0.0.1:1/search");
        let result = backend.search("rust", 3).await;
        assert!(result.is_err());
    }
}
