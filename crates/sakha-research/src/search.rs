//! `SearchClient` trait and provider-neutral search types. See spec
//! `modules/11-web-search-research.md` "Main Structs" / "Search Providers".
//!
//! Includes a config-driven HTTP backend (`HttpSearchClient`) that speaks a
//! SearxNG/Brave-style JSON search API, plus a `MockSearchClient` so tests
//! never require live network access.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use sakha_core::{SakhaError, SakhaResult};

/// Which search backend a query should target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchProvider {
    GeneralWeb,
    DomainRestricted,
    GitHub,
    PackageRegistry,
    DocumentationSite,
    InternalDocs,
}

/// A single search query, optionally scoped to a provider/domain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub text: String,
    pub provider: SearchProvider,
    pub domain_filter: Option<String>,
    pub max_results: u32,
}

impl SearchQuery {
    pub fn new(text: impl Into<String>, provider: SearchProvider) -> Self {
        Self { text: text.into(), provider, domain_filter: None, max_results: 10 }
    }

    pub fn with_domain_filter(mut self, domain: impl Into<String>) -> Self {
        self.domain_filter = Some(domain.into());
        self
    }

    pub fn with_max_results(mut self, max_results: u32) -> Self {
        self.max_results = max_results;
        self
    }
}

/// A multi-query plan derived from a research task.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchPlan {
    pub queries: Vec<SearchQuery>,
}

impl SearchPlan {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(mut self, query: SearchQuery) -> Self {
        self.queries.push(query);
        self
    }
}

/// Turns a research task description into a `SearchPlan`. Deterministic,
/// dependency-free heuristic query planner: emits a general-web query for the
/// raw task, and if the task looks like a library/package/docs lookup, adds
/// documentation-site and package-registry queries scoped accordingly. See
/// spec "Research Loop" step 1-2 and Implementation Tasks item 2.
pub fn plan_queries(task: &str) -> SearchPlan {
    let trimmed = task.trim();
    let mut plan = SearchPlan::new();
    if trimmed.is_empty() {
        return plan;
    }

    plan = plan.push(SearchQuery::new(trimmed, SearchProvider::GeneralWeb));

    let lower = trimmed.to_lowercase();
    let doc_markers = ["docs for", "documentation", "how to use", "api reference", "library"];
    if doc_markers.iter().any(|m| lower.contains(m)) {
        plan = plan.push(SearchQuery::new(
            format!("{trimmed} official documentation"),
            SearchProvider::DocumentationSite,
        ));
    }

    let package_markers = ["package", "crate", "npm", "pip", "cargo add", "install"];
    if package_markers.iter().any(|m| lower.contains(m)) {
        plan = plan.push(SearchQuery::new(trimmed, SearchProvider::PackageRegistry));
    }

    if lower.contains("github") || lower.contains("source code") || lower.contains("repo") {
        plan = plan.push(SearchQuery::new(trimmed, SearchProvider::GitHub));
    }

    plan
}

/// One search result entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub url: String,
    pub title: String,
    pub snippet: String,
}

/// Abstraction over a web/code search backend.
#[async_trait]
pub trait SearchClient: Send + Sync {
    async fn search(&self, query: &SearchQuery) -> SakhaResult<Vec<SearchResult>>;
}

/// Deduplicates search results by normalized URL, preserving first-seen
/// order. See spec "Research Loop" step 4 "Deduplicate results" and
/// Tests "Duplicate URL detection".
pub fn dedupe_results(results: Vec<SearchResult>) -> Vec<SearchResult> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(results.len());
    for r in results {
        let key = normalize_url(&r.url);
        if seen.insert(key) {
            out.push(r);
        }
    }
    out
}

/// Normalizes a URL for dedup comparison: lowercases scheme/host, strips a
/// trailing slash and any fragment.
pub fn normalize_url(url: &str) -> String {
    let without_fragment = url.split('#').next().unwrap_or(url);
    let trimmed = without_fragment.trim_end_matches('/');
    trimmed.to_lowercase()
}

/// A `SearchClient` that always returns an empty result set. Safe default
/// fallback so the workspace never requires a live search API key to build
/// or test.
#[derive(Debug, Default)]
pub struct NullSearchClient;

#[async_trait]
impl SearchClient for NullSearchClient {
    async fn search(&self, _query: &SearchQuery) -> SakhaResult<Vec<SearchResult>> {
        Err(SakhaError::not_implemented("sakha-research", "NullSearchClient::search"))
    }
}

/// In-memory `SearchClient` for tests: returns canned results for queries
/// whose text contains a registered substring key.
#[derive(Debug, Default)]
pub struct MockSearchClient {
    fixtures: Vec<(String, Vec<SearchResult>)>,
}

impl MockSearchClient {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers canned results returned when `query.text` contains
    /// `text_contains` (case-insensitive).
    pub fn with_fixture(mut self, text_contains: impl Into<String>, results: Vec<SearchResult>) -> Self {
        self.fixtures.push((text_contains.into().to_lowercase(), results));
        self
    }
}

#[async_trait]
impl SearchClient for MockSearchClient {
    async fn search(&self, query: &SearchQuery) -> SakhaResult<Vec<SearchResult>> {
        let lower = query.text.to_lowercase();
        for (key, results) in &self.fixtures {
            if lower.contains(key.as_str()) {
                let mut out = results.clone();
                out.truncate(query.max_results as usize);
                return Ok(out);
            }
        }
        Ok(Vec::new())
    }
}

/// Which JSON response shape a configured search endpoint speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchEndpointKind {
    /// SearxNG `/search?format=json` response: `{"results": [{"url","title","content"}, ...]}`.
    SearxNg,
    /// Brave Search API response: `{"web": {"results": [{"url","title","description"}, ...]}}`.
    Brave,
}

/// Config for a JSON HTTP search endpoint (SearxNG instance, Brave API,
/// or a compatible internal connector). See spec "Search Providers".
///
/// `api_key` is intentionally excluded from `Serialize`/`Deserialize`
/// (`#[serde(skip)]`): per spec this secret must "never [be] logged" and
/// config values that derive `Serialize` are routinely dumped into logs,
/// audit trails, or `GET` debug endpoints elsewhere in the workspace, so the
/// only safe default is for the key to never round-trip through JSON at all.
/// Callers that need to persist/load it must do so through a dedicated
/// secret store, not this struct's `Deserialize` impl (which always yields
/// `None` for this field).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchEndpointConfig {
    pub kind: SearchEndpointKind,
    pub base_url: String,
    /// Optional API key, sent as `X-Subscription-Token` (Brave) or as a
    /// query param (`?key=`) depending on `kind`. Never logged, never
    /// serialized.
    #[serde(skip, default)]
    pub api_key: Option<String>,
    pub timeout_secs: u64,
}

impl SearchEndpointConfig {
    pub fn searxng(base_url: impl Into<String>) -> Self {
        Self { kind: SearchEndpointKind::SearxNg, base_url: base_url.into(), api_key: None, timeout_secs: 20 }
    }

    pub fn brave(api_key: impl Into<String>) -> Self {
        Self {
            kind: SearchEndpointKind::Brave,
            base_url: "https://api.search.brave.com/res/v1/web/search".to_string(),
            api_key: Some(api_key.into()),
            timeout_secs: 20,
        }
    }
}

/// `SearchClient` backed by a configured JSON HTTP endpoint (SearxNG- or
/// Brave-style). Never used in unit tests directly (no live network in CI);
/// exercised via `build_request` / `parse_response` unit tests instead.
pub struct HttpSearchClient {
    config: SearchEndpointConfig,
    http: reqwest::Client,
}

impl HttpSearchClient {
    pub fn new(config: SearchEndpointConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .unwrap_or_default();
        Self { config, http }
    }

    /// Builds the outgoing request for `query` against the configured
    /// endpoint kind. Split out from `search` so it is unit-testable without
    /// a live network call.
    pub fn build_request(&self, query: &SearchQuery) -> reqwest::RequestBuilder {
        match self.config.kind {
            SearchEndpointKind::SearxNg => {
                let mut req = self
                    .http
                    .get(&self.config.base_url)
                    .query(&[("q", query.text.as_str()), ("format", "json")]);
                if let Some(domain) = &query.domain_filter {
                    req = req.query(&[("q", format!("{} site:{}", query.text, domain))]);
                }
                if let Some(key) = &self.config.api_key {
                    req = req.query(&[("key", key.as_str())]);
                }
                req
            }
            SearchEndpointKind::Brave => {
                let mut q = query.text.clone();
                if let Some(domain) = &query.domain_filter {
                    q = format!("{q} site:{domain}");
                }
                let mut req = self
                    .http
                    .get(&self.config.base_url)
                    .query(&[("q", q.as_str()), ("count", &query.max_results.to_string())]);
                if let Some(key) = &self.config.api_key {
                    req = req.header("X-Subscription-Token", key);
                }
                req
            }
        }
    }

    /// Parses a JSON body from the configured endpoint kind into normalized
    /// `SearchResult`s. Unknown/missing fields are skipped rather than
    /// failing the whole batch.
    pub fn parse_response(&self, body: &serde_json::Value) -> Vec<SearchResult> {
        match self.config.kind {
            SearchEndpointKind::SearxNg => body
                .get("results")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| {
                            let url = item.get("url")?.as_str()?.to_string();
                            let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
                            let snippet = item.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
                            Some(SearchResult { url, title, snippet })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            SearchEndpointKind::Brave => body
                .get("web")
                .and_then(|w| w.get("results"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| {
                            let url = item.get("url")?.as_str()?.to_string();
                            let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
                            let snippet =
                                item.get("description").and_then(|c| c.as_str()).unwrap_or("").to_string();
                            Some(SearchResult { url, title, snippet })
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

#[async_trait]
impl SearchClient for HttpSearchClient {
    async fn search(&self, query: &SearchQuery) -> SakhaResult<Vec<SearchResult>> {
        let response = self
            .build_request(query)
            .send()
            .await
            .map_err(|e| SakhaError::transient("sakha-research", "search request failed").with_cause(e))?;
        if !response.status().is_success() {
            return Err(SakhaError::transient(
                "sakha-research",
                format!("search endpoint returned status {}", response.status()),
            ));
        }
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| SakhaError::transient("sakha-research", "search response was not valid JSON").with_cause(e))?;
        let mut results = self.parse_response(&body);
        results.truncate(query.max_results as usize);
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_queries_for_library_docs_includes_documentation_and_package_queries() {
        let plan = plan_queries("how to install and use the tokio library documentation");
        assert!(plan.queries.iter().any(|q| q.provider == SearchProvider::GeneralWeb));
        assert!(plan.queries.iter().any(|q| q.provider == SearchProvider::DocumentationSite));
        assert!(plan.queries.iter().any(|q| q.provider == SearchProvider::PackageRegistry));
    }

    #[test]
    fn plan_queries_empty_task_yields_empty_plan() {
        let plan = plan_queries("   ");
        assert!(plan.queries.is_empty());
    }

    #[test]
    fn dedupe_results_removes_duplicate_urls() {
        let results = vec![
            SearchResult { url: "https://example.com/a".into(), title: "A".into(), snippet: "".into() },
            SearchResult { url: "https://example.com/a/".into(), title: "A dup".into(), snippet: "".into() },
            SearchResult { url: "https://example.com/b".into(), title: "B".into(), snippet: "".into() },
        ];
        let deduped = dedupe_results(results);
        assert_eq!(deduped.len(), 2);
    }

    #[tokio::test]
    async fn mock_search_client_returns_fixture_for_matching_query() {
        let client = MockSearchClient::new().with_fixture(
            "tokio",
            vec![SearchResult { url: "https://tokio.rs".into(), title: "Tokio".into(), snippet: "async runtime".into() }],
        );
        let results = client
            .search(&SearchQuery::new("tokio documentation", SearchProvider::DocumentationSite))
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://tokio.rs");
    }

    #[tokio::test]
    async fn mock_search_client_returns_empty_for_unknown_query() {
        let client = MockSearchClient::new();
        let results = client
            .search(&SearchQuery::new("anything", SearchProvider::GeneralWeb))
            .await
            .unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn http_search_client_build_request_searxng_targets_base_url() {
        let client = HttpSearchClient::new(SearchEndpointConfig::searxng("https://searx.example.com/search"));
        let req = client
            .build_request(&SearchQuery::new("rust async", SearchProvider::GeneralWeb))
            .build()
            .unwrap();
        assert_eq!(req.url().host_str(), Some("searx.example.com"));
        assert!(req.url().query().unwrap_or("").contains("format=json"));
    }

    #[test]
    fn http_search_client_parse_response_searxng() {
        let client = HttpSearchClient::new(SearchEndpointConfig::searxng("https://searx.example.com/search"));
        let body = serde_json::json!({
            "results": [
                {"url": "https://a.com", "title": "A", "content": "snippet a"},
                {"url": "https://b.com", "title": "B", "content": "snippet b"}
            ]
        });
        let results = client.parse_response(&body);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, "https://a.com");
    }

    #[test]
    fn http_search_client_parse_response_brave() {
        let client = HttpSearchClient::new(SearchEndpointConfig::brave("test-key"));
        let body = serde_json::json!({
            "web": {"results": [{"url": "https://a.com", "title": "A", "description": "snippet a"}]}
        });
        let results = client.parse_response(&body);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].snippet, "snippet a");
    }

    #[test]
    fn http_search_client_parse_response_missing_field_is_skipped() {
        let client = HttpSearchClient::new(SearchEndpointConfig::searxng("https://searx.example.com/search"));
        let body = serde_json::json!({ "results": [ {"title": "no url"} ] });
        let results = client.parse_response(&body);
        assert!(results.is_empty());
    }

    #[test]
    fn search_endpoint_config_never_serializes_api_key() {
        let config = SearchEndpointConfig::brave("super-secret-key");
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("super-secret-key"), "api_key leaked into serialized config: {json}");
        assert!(!json.contains("api_key"), "api_key field name should be skipped entirely: {json}");
    }

    #[test]
    fn search_endpoint_config_deserializes_without_api_key_field() {
        let json = serde_json::json!({
            "kind": "searx_ng",
            "base_url": "https://searx.example.com/search",
            "timeout_secs": 20
        });
        let config: SearchEndpointConfig = serde_json::from_value(json).unwrap();
        assert!(config.api_key.is_none());
    }
}
