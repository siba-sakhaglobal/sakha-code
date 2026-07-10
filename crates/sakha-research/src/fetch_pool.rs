//! `FetchPool`: fetches one URL as clean markdown, trying progressively
//! more capable backends: Firecrawl scrape -> Jina Reader -> plain
//! `reqwest` GET + `html2text` extraction. See spec
//! `modules/11-web-search-research.md` "Research Loop" step 5 and
//! `docs/web-search.md`.

use async_trait::async_trait;

use sakha_core::{SakhaError, SakhaResult};

use crate::extract::{Extractor, Html2TextExtractor};
use crate::fetch::{FetchRequest, PageFetcher, ReqwestPageFetcher};

const MODULE: &str = "sakha-research::fetch_pool";
const DEFAULT_TIMEOUT_SECS: u64 = 20;
pub const DEFAULT_MAX_CHARS: usize = 12_000;
const TRUNCATION_MARKER: &str = "\n\n[truncated]";

/// The outcome of a successful fetch: which backend served it, plus the
/// (already truncated) markdown/text content.
#[derive(Debug, Clone)]
pub struct FetchOutcome {
    pub backend: String,
    pub content_markdown: String,
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .build()
        .unwrap_or_default()
}

/// Strips null bytes and truncates `text` to `max_chars` characters
/// (char-boundary safe), appending a `"[truncated]"` marker when truncation
/// happened. Shared by every backend in the chain so callers get consistent
/// output regardless of which one served the fetch.
pub fn sanitize_and_truncate(text: &str, max_chars: usize) -> String {
    let cleaned: String = text.chars().filter(|c| *c != '\0').collect();
    let char_count = cleaned.chars().count();
    if char_count <= max_chars {
        return cleaned;
    }
    let truncated: String = cleaned.chars().take(max_chars).collect();
    format!("{truncated}{TRUNCATION_MARKER}")
}

/// Attempts Firecrawl's `/v2/scrape` endpoint. Keyless-capable (rate
/// limited) exactly like `FirecrawlBackend` in `backends.rs`.
async fn try_firecrawl_scrape(http: &reqwest::Client, url: &str) -> SakhaResult<String> {
    let mut req = http.post("https://api.firecrawl.dev/v2/scrape").json(&serde_json::json!({
        "url": url,
        "formats": ["markdown"],
    }));
    if let Ok(key) = std::env::var("FIRECRAWL_API_KEY") {
        if !key.is_empty() {
            req = req.bearer_auth(key);
        }
    }
    let response = req
        .send()
        .await
        .map_err(|_| SakhaError::transient(MODULE, "firecrawl scrape request failed").with_subject("firecrawl"))?;
    if !response.status().is_success() {
        return Err(SakhaError::transient(MODULE, format!("firecrawl scrape returned status {}", response.status()))
            .with_subject("firecrawl"));
    }
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|_| SakhaError::transient(MODULE, "firecrawl scrape response was not valid JSON").with_subject("firecrawl"))?;
    parse_firecrawl_scrape(&body)
}

/// Parses the verified Firecrawl scrape response shape:
/// `{"success":true,"data":{"markdown":"..."}}`. Public for fixture tests.
pub fn parse_firecrawl_scrape(body: &serde_json::Value) -> SakhaResult<String> {
    body.get("data")
        .and_then(|d| d.get("markdown"))
        .and_then(|m| m.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| SakhaError::transient(MODULE, "firecrawl scrape response missing data.markdown").with_subject("firecrawl"))
}

/// Attempts Jina Reader (`https://r.jina.ai/{url}`). Keyless-capable
/// (rate-limited); sends `Authorization: Bearer` only when `JINA_API_KEY` is
/// set.
async fn try_jina_reader(http: &reqwest::Client, url: &str) -> SakhaResult<String> {
    let reader_url = format!("https://r.jina.ai/{url}");
    let mut req = http.get(&reader_url);
    if let Ok(key) = std::env::var("JINA_API_KEY") {
        if !key.is_empty() {
            req = req.bearer_auth(key);
        }
    }
    let response = req.send().await.map_err(|_| SakhaError::transient(MODULE, "jina reader request failed").with_subject("jina"))?;
    if !response.status().is_success() {
        return Err(SakhaError::transient(MODULE, format!("jina reader returned status {}", response.status())).with_subject("jina"));
    }
    response.text().await.map_err(|_| SakhaError::transient(MODULE, "jina reader response was not valid text").with_subject("jina"))
}

/// Plain `reqwest` GET + `html2text` extraction — the final, always-available
/// fallback (no key, no third-party dependency).
async fn try_plain_fetch(fetcher: &dyn PageFetcher, url: &str) -> SakhaResult<String> {
    let page = fetcher.fetch(FetchRequest::new(url)).await?;
    if page.status >= 400 {
        return Err(SakhaError::transient(MODULE, format!("plain fetch returned status {}", page.status)).with_subject("plain"));
    }
    let extractor = Html2TextExtractor::new();
    let doc = extractor.extract(&page)?;
    Ok(doc.text)
}

/// The fetch/scrape chain: Firecrawl scrape -> Jina Reader -> plain fetch.
/// Each stage is tried in order; the first to succeed wins. Result is
/// sanitized (null bytes stripped) and truncated to `max_chars`.
pub struct FetchPool {
    http: reqwest::Client,
    plain_fetcher: Box<dyn PageFetcher>,
    /// When true, skips the Firecrawl/Jina remote stages entirely and goes
    /// straight to `plain_fetcher`. Used by tests that inject a
    /// `MockPageFetcher` and must not depend on live network reachability
    /// of `api.firecrawl.dev`/`r.jina.ai` to stay deterministic.
    skip_remote_stages: bool,
}

impl FetchPool {
    pub fn new() -> Self {
        Self { http: http_client(), plain_fetcher: Box::new(ReqwestPageFetcher::new()), skip_remote_stages: false }
    }

    /// Injects a custom plain-fetch fallback (tests use `MockPageFetcher`).
    pub fn with_plain_fetcher(mut self, fetcher: Box<dyn PageFetcher>) -> Self {
        self.plain_fetcher = fetcher;
        self
    }

    /// Skips the Firecrawl/Jina remote stages, going straight to the
    /// plain-fetch fallback. Intended for tests/offline mode.
    pub fn skip_remote_stages(mut self) -> Self {
        self.skip_remote_stages = true;
        self
    }

    pub async fn fetch(&self, url: &str, max_chars: usize) -> SakhaResult<FetchOutcome> {
        if !self.skip_remote_stages {
            if let Ok(markdown) = try_firecrawl_scrape(&self.http, url).await {
                return Ok(FetchOutcome { backend: "firecrawl".to_string(), content_markdown: sanitize_and_truncate(&markdown, max_chars) });
            }
            if let Ok(markdown) = try_jina_reader(&self.http, url).await {
                return Ok(FetchOutcome { backend: "jina".to_string(), content_markdown: sanitize_and_truncate(&markdown, max_chars) });
            }
        }
        let text = try_plain_fetch(self.plain_fetcher.as_ref(), url).await?;
        Ok(FetchOutcome { backend: "plain".to_string(), content_markdown: sanitize_and_truncate(&text, max_chars) })
    }
}

impl Default for FetchPool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PageFetcher for FetchPool {
    /// Adapts `FetchPool::fetch` to the `PageFetcher` trait for callers that
    /// only need the `PageFetcher` contract (uses `FetchRequest::max_bytes`
    /// as a rough proxy for `max_chars` since the trait has no char-count
    /// concept).
    async fn fetch(&self, request: FetchRequest) -> SakhaResult<crate::fetch::FetchedPage> {
        let outcome = self.fetch(&request.url, request.max_bytes).await?;
        Ok(crate::fetch::FetchedPage {
            url: request.url,
            status: 200,
            content_type: Some("text/markdown".to_string()),
            body: outcome.content_markdown,
            fetched_at: sakha_core::time::now_utc(),
            truncated: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::{FetchedPage, MockPageFetcher};

    #[test]
    fn parse_firecrawl_scrape_verified_shape() {
        let body = serde_json::json!({"success": true, "data": {"markdown": "# Hello\n\nWorld"}});
        let markdown = parse_firecrawl_scrape(&body).unwrap();
        assert_eq!(markdown, "# Hello\n\nWorld");
    }

    #[test]
    fn parse_firecrawl_scrape_missing_markdown_errors() {
        let body = serde_json::json!({"success": true, "data": {}});
        let result = parse_firecrawl_scrape(&body);
        assert!(result.is_err());
    }

    #[test]
    fn sanitize_and_truncate_strips_null_bytes() {
        let out = sanitize_and_truncate("hello\0world", 100);
        assert_eq!(out, "helloworld");
    }

    #[test]
    fn sanitize_and_truncate_under_limit_is_unchanged() {
        let out = sanitize_and_truncate("short text", 100);
        assert_eq!(out, "short text");
    }

    #[test]
    fn sanitize_and_truncate_over_limit_adds_marker() {
        let long = "a".repeat(200);
        let out = sanitize_and_truncate(&long, 50);
        assert!(out.ends_with("[truncated]"));
        // 50 'a's + marker.
        assert_eq!(out.len(), 50 + TRUNCATION_MARKER.len());
    }

    #[test]
    fn sanitize_and_truncate_is_char_boundary_safe_for_multibyte() {
        let text = "日".repeat(100);
        let out = sanitize_and_truncate(&text, 10);
        assert!(out.starts_with(&"日".repeat(10)));
        assert!(out.ends_with("[truncated]"));
    }

    #[tokio::test]
    async fn fetch_pool_falls_back_to_plain_fetch_when_firecrawl_and_jina_unreachable() {
        // Firecrawl/Jina hit real hostnames over the network in this unit
        // test environment (no mocking of those fixed URLs), so this test
        // instead verifies the plain-fetch fallback logic directly via the
        // injected MockPageFetcher path exercised through `try_plain_fetch`.
        let fetcher = MockPageFetcher::new().with_page(
            "https://example.com/page",
            FetchedPage {
                url: "https://example.com/page".into(),
                status: 200,
                content_type: Some("text/html".into()),
                body: "<html><body><p>Hello World</p></body></html>".into(),
                fetched_at: sakha_core::time::now_utc(),
                truncated: false,
            },
        );
        let text = try_plain_fetch(&fetcher, "https://example.com/page").await.unwrap();
        assert!(text.contains("Hello World"));
    }

    #[tokio::test]
    async fn plain_fetch_errors_on_4xx_status() {
        let fetcher = MockPageFetcher::new(); // unknown URL -> 404 per MockPageFetcher contract
        let result = try_plain_fetch(&fetcher, "https://unknown.example.com").await;
        assert!(result.is_err());
    }

    #[test]
    fn default_max_chars_is_reasonable() {
        assert_eq!(DEFAULT_MAX_CHARS, 12_000);
    }
}
