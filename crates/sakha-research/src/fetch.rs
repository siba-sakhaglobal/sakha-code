//! Page fetching. See spec "Research Loop" step 5 "Fetch highest quality pages".

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use sakha_core::{SakhaError, SakhaResult};

/// A request to fetch a single URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchRequest {
    pub url: String,
    pub timeout_secs: u64,
    /// Hard cap on response body size in bytes. Prevents unbounded memory
    /// use from a hostile/misbehaving server. See spec "Web Search Safety".
    pub max_bytes: usize,
}

impl FetchRequest {
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into(), timeout_secs: 30, max_bytes: 2 * 1024 * 1024 }
    }

    pub fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes;
        self
    }
}

/// A fetched page: raw bytes/text plus response metadata. Content is
/// untrusted per spec "Web Search Safety" until passed through the
/// injection filter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchedPage {
    pub url: String,
    pub status: u16,
    pub content_type: Option<String>,
    pub body: String,
    pub fetched_at: chrono::DateTime<chrono::Utc>,
    /// True when the body was truncated to respect `FetchRequest::max_bytes`.
    #[serde(default)]
    pub truncated: bool,
}

/// Abstraction over an HTTP fetcher, so tests never require real network access.
#[async_trait]
pub trait PageFetcher: Send + Sync {
    async fn fetch(&self, request: FetchRequest) -> SakhaResult<FetchedPage>;
}

/// A `PageFetcher` that always fails with `not_implemented`. Safe default.
#[derive(Debug, Default)]
pub struct NullPageFetcher;

#[async_trait]
impl PageFetcher for NullPageFetcher {
    async fn fetch(&self, _request: FetchRequest) -> SakhaResult<FetchedPage> {
        Err(SakhaError::not_implemented("sakha-research", "NullPageFetcher::fetch"))
    }
}

/// In-memory `PageFetcher` for tests: returns canned pages registered by
/// exact URL, otherwise a 404-shaped response.
#[derive(Debug, Default)]
pub struct MockPageFetcher {
    pages: std::collections::HashMap<String, FetchedPage>,
}

impl MockPageFetcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_page(mut self, url: impl Into<String>, page: FetchedPage) -> Self {
        self.pages.insert(url.into(), page);
        self
    }
}

#[async_trait]
impl PageFetcher for MockPageFetcher {
    async fn fetch(&self, request: FetchRequest) -> SakhaResult<FetchedPage> {
        match self.pages.get(&request.url) {
            Some(page) => Ok(page.clone()),
            None => Ok(FetchedPage {
                url: request.url,
                status: 404,
                content_type: None,
                body: String::new(),
                fetched_at: sakha_core::time::now_utc(),
                truncated: false,
            }),
        }
    }
}

/// `PageFetcher` backed by `reqwest`, enforcing a size cap and timeout.
/// Never executes downloaded content; only reads bytes as text. See spec
/// "Web Search Safety" ("Never execute downloaded code").
pub struct ReqwestPageFetcher {
    http: reqwest::Client,
}

impl ReqwestPageFetcher {
    pub fn new() -> Self {
        Self { http: reqwest::Client::new() }
    }
}

impl Default for ReqwestPageFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PageFetcher for ReqwestPageFetcher {
    async fn fetch(&self, request: FetchRequest) -> SakhaResult<FetchedPage> {
        let response = self
            .http
            .get(&request.url)
            .timeout(std::time::Duration::from_secs(request.timeout_secs))
            .send()
            .await
            .map_err(|e| SakhaError::transient("sakha-research", "fetch request failed").with_cause(e))?;

        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let mut body_bytes: Vec<u8> = Vec::new();
        let mut truncated = false;
        let mut stream = response;
        while let Some(chunk) = stream
            .chunk()
            .await
            .map_err(|e| SakhaError::transient("sakha-research", "fetch stream failed").with_cause(e))?
        {
            if body_bytes.len() + chunk.len() > request.max_bytes {
                let remaining = request.max_bytes.saturating_sub(body_bytes.len());
                body_bytes.extend_from_slice(&chunk[..remaining.min(chunk.len())]);
                truncated = true;
                break;
            }
            body_bytes.extend_from_slice(&chunk);
        }

        let body = String::from_utf8_lossy(&body_bytes).into_owned();

        Ok(FetchedPage {
            url: request.url,
            status,
            content_type,
            body,
            fetched_at: sakha_core::time::now_utc(),
            truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_fetcher_returns_registered_page() {
        let fetcher = MockPageFetcher::new().with_page(
            "https://example.com",
            FetchedPage {
                url: "https://example.com".into(),
                status: 200,
                content_type: Some("text/html".into()),
                body: "<p>hi</p>".into(),
                fetched_at: sakha_core::time::now_utc(),
                truncated: false,
            },
        );
        let page = fetcher.fetch(FetchRequest::new("https://example.com")).await.unwrap();
        assert_eq!(page.status, 200);
        assert_eq!(page.body, "<p>hi</p>");
    }

    #[tokio::test]
    async fn mock_fetcher_returns_404_for_unknown_url() {
        let fetcher = MockPageFetcher::new();
        let page = fetcher.fetch(FetchRequest::new("https://unknown.example.com")).await.unwrap();
        assert_eq!(page.status, 404);
    }

    #[test]
    fn fetch_request_default_has_size_cap() {
        let req = FetchRequest::new("https://example.com");
        assert!(req.max_bytes > 0);
    }

    #[test]
    fn fetch_request_with_max_bytes_overrides_default() {
        let req = FetchRequest::new("https://example.com").with_max_bytes(1024);
        assert_eq!(req.max_bytes, 1024);
    }
}
