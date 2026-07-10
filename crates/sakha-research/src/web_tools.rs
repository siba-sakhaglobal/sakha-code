//! `web.search` / `web.fetch` agent tools: the economical search-then-fetch
//! flow described in `docs/web-search.md`. `web.search` returns cheap
//! snippets from the configured `SearchPool`; `web.fetch` returns clean
//! markdown for exactly one URL via `FetchPool`, size-capped. See spec
//! `modules/11-web-search-research.md` "Research Loop" and
//! `modules/07-tool-system.md` `Tool`.

use std::sync::Arc;

use async_trait::async_trait;

use sakha_core::{SakhaError, SakhaResult};
use sakha_security::PermissionKind;
use sakha_tools::{
    IdempotencyPolicy, Tool, ToolContext, ToolInputSchema, ToolName, ToolOutputSchema, ToolPermissionSpec, ToolPlan,
    ToolResult, ToolSpec, ToolSummary, ValidatedInput,
};

use crate::fetch_pool::{FetchPool, DEFAULT_MAX_CHARS};
use crate::pool::{SearchPool, DEFAULT_BACKEND_ORDER};

const MODULE: &str = "sakha-research::web_tools";
const DEFAULT_SEARCH_LIMIT: u32 = 5;
const MAX_SEARCH_LIMIT: u32 = 10;

/// Builds a `SearchPool` from every backend in `DEFAULT_BACKEND_ORDER`, each
/// constructed with its default (env-var-driven) configuration. This is the
/// pool `web.search` uses when the CLI hasn't wired up a config-driven
/// priority override; `sakha-cli` builds its own pool from `[search]` config
/// and passes it to `WebSearchTool::new` instead of relying on this default
/// where a custom priority order is needed.
pub fn default_search_pool() -> SearchPool {
    use crate::backends::{BraveBackend, ExaBackend, FirecrawlBackend, SearchBackend, SearxngBackend, SerpApiBackend, SerperBackend, TavilyBackend};

    let all: Vec<Box<dyn SearchBackend>> = vec![
        Box::new(FirecrawlBackend::new()),
        Box::new(BraveBackend::new()),
        Box::new(TavilyBackend::new()),
        Box::new(SerperBackend::new()),
        Box::new(SerpApiBackend::new()),
        Box::new(ExaBackend::new()),
        Box::new(SearxngBackend::new()),
    ];

    let mut ordered = Vec::with_capacity(all.len());
    let mut remaining = all;
    for id in DEFAULT_BACKEND_ORDER {
        if let Some(pos) = remaining.iter().position(|b| b.id() == *id) {
            ordered.push(remaining.remove(pos));
        }
    }
    ordered.extend(remaining);

    SearchPool::new(ordered)
}

/// `web.search`: cheap snippet discovery via the shared `SearchPool`.
pub struct WebSearchTool {
    pool: Arc<SearchPool>,
}

impl WebSearchTool {
    pub fn new(pool: Arc<SearchPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("web.search"),
            description: "Search the web. Cheap — returns snippets only. Call web.fetch on the most relevant URL(s) afterwards.".to_string(),
            input_schema: ToolInputSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "limit": {"type": "integer"}
                },
                "required": ["query"]
            })),
            output_schema: ToolOutputSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "backend": {"type": "string"},
                    "hits": {"type": "array"}
                }
            })),
            permission_spec: ToolPermissionSpec { required: vec![PermissionKind::NetworkAccess] },
            idempotency_policy: IdempotencyPolicy::Idempotent,
        }
    }

    fn validate(&self, input: serde_json::Value) -> SakhaResult<ValidatedInput> {
        sakha_tools::schema::validate_against_schema(MODULE, &input, &self.spec().input_schema.0)?;
        let query = input.get("query").and_then(|q| q.as_str()).unwrap_or("");
        if query.trim().is_empty() {
            return Err(SakhaError::invalid_input(MODULE, "query must not be empty"));
        }
        if let Some(limit) = input.get("limit").and_then(|l| l.as_i64()) {
            if !(1..=MAX_SEARCH_LIMIT as i64).contains(&limit) {
                return Err(SakhaError::invalid_input(MODULE, format!("limit must be between 1 and {MAX_SEARCH_LIMIT}")));
            }
        }
        Ok(ValidatedInput(input))
    }

    async fn plan(&self, input: &ValidatedInput, _context: &ToolContext) -> SakhaResult<ToolPlan> {
        let query = input.0.get("query").and_then(|q| q.as_str()).unwrap_or_default();
        Ok(ToolPlan { summary: format!("web search: {query}"), affected_paths: Vec::new(), is_destructive: false })
    }

    async fn execute(&self, input: &ValidatedInput, _context: &ToolContext) -> SakhaResult<ToolResult> {
        let query = input.0.get("query").and_then(|q| q.as_str()).ok_or_else(|| SakhaError::invalid_input(MODULE, "missing query"))?;
        let limit = input.0.get("limit").and_then(|l| l.as_u64()).map(|l| l as u32).unwrap_or(DEFAULT_SEARCH_LIMIT);

        let result = self.pool.search(query, limit).await?;
        let hits: Vec<serde_json::Value> = result
            .hits
            .iter()
            .map(|h| serde_json::json!({ "title": h.title, "url": h.url, "snippet": h.snippet }))
            .collect();

        Ok(ToolResult::success(serde_json::json!({
            "backend": result.backend,
            "hits": hits,
        })))
    }

    fn summarize(&self, result: &ToolResult) -> ToolSummary {
        let count = result.output_json.get("hits").and_then(|h| h.as_array()).map(|a| a.len()).unwrap_or(0);
        let backend = result.output_json.get("backend").and_then(|b| b.as_str()).unwrap_or("unknown");
        ToolSummary { text: format!("{count} hit(s) via {backend}"), truncated: false }
    }
}

/// `web.fetch`: fetches one URL as clean markdown via the shared
/// `FetchPool`, size-capped.
pub struct WebFetchTool {
    pool: Arc<FetchPool>,
}

impl WebFetchTool {
    pub fn new(pool: Arc<FetchPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("web.fetch"),
            description: "Fetch one page as clean markdown. Costs more tokens — only fetch URLs chosen from web.search results.".to_string(),
            input_schema: ToolInputSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {"type": "string"},
                    "max_chars": {"type": "integer"}
                },
                "required": ["url"]
            })),
            output_schema: ToolOutputSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {"type": "string"},
                    "backend": {"type": "string"},
                    "content_markdown": {"type": "string"}
                }
            })),
            permission_spec: ToolPermissionSpec { required: vec![PermissionKind::NetworkAccess] },
            idempotency_policy: IdempotencyPolicy::Idempotent,
        }
    }

    fn validate(&self, input: serde_json::Value) -> SakhaResult<ValidatedInput> {
        sakha_tools::schema::validate_against_schema(MODULE, &input, &self.spec().input_schema.0)?;
        let url = input.get("url").and_then(|u| u.as_str()).unwrap_or("");
        if url.trim().is_empty() {
            return Err(SakhaError::invalid_input(MODULE, "url must not be empty"));
        }
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(SakhaError::invalid_input(MODULE, "url must start with http:// or https://"));
        }
        Ok(ValidatedInput(input))
    }

    async fn plan(&self, input: &ValidatedInput, _context: &ToolContext) -> SakhaResult<ToolPlan> {
        let url = input.0.get("url").and_then(|u| u.as_str()).unwrap_or_default();
        Ok(ToolPlan { summary: format!("fetch: {url}"), affected_paths: Vec::new(), is_destructive: false })
    }

    async fn execute(&self, input: &ValidatedInput, _context: &ToolContext) -> SakhaResult<ToolResult> {
        let url = input.0.get("url").and_then(|u| u.as_str()).ok_or_else(|| SakhaError::invalid_input(MODULE, "missing url"))?;
        let max_chars = input.0.get("max_chars").and_then(|m| m.as_u64()).map(|m| m as usize).unwrap_or(DEFAULT_MAX_CHARS);

        let outcome = self.pool.fetch(url, max_chars).await?;

        Ok(ToolResult::success(serde_json::json!({
            "url": url,
            "backend": outcome.backend,
            "content_markdown": outcome.content_markdown,
        })))
    }

    fn summarize(&self, result: &ToolResult) -> ToolSummary {
        let backend = result.output_json.get("backend").and_then(|b| b.as_str()).unwrap_or("unknown");
        let len = result.output_json.get("content_markdown").and_then(|c| c.as_str()).map(|s| s.len()).unwrap_or(0);
        ToolSummary { text: format!("fetched {len} chars via {backend}"), truncated: false }
    }
}

/// Builds the `web.search` and `web.fetch` tools sharing one `SearchPool`
/// and one `FetchPool`, both built from default (env-var-driven) backend
/// configuration. `sakha-cli` calls this in `build_tool_registry`.
pub fn web_tools() -> Vec<Arc<dyn Tool>> {
    let search_pool = Arc::new(default_search_pool());
    let fetch_pool = Arc::new(FetchPool::new());
    vec![Arc::new(WebSearchTool::new(search_pool)), Arc::new(WebFetchTool::new(fetch_pool))]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::{SearchBackend, SearchHit};
    use crate::fetch::{FetchedPage, MockPageFetcher, PageFetcher};

    struct AlwaysSucceedsBackend;

    #[async_trait]
    impl SearchBackend for AlwaysSucceedsBackend {
        fn id(&self) -> &'static str {
            "fake"
        }
        fn is_configured(&self) -> bool {
            true
        }
        fn env_var(&self) -> Option<&'static str> {
            None
        }
        fn key_portal_url(&self) -> &'static str {
            "https://example.com"
        }
        async fn search(&self, _query: &str, _limit: u32) -> SakhaResult<Vec<SearchHit>> {
            Ok(vec![SearchHit { title: "Result".into(), url: "https://example.com".into(), snippet: "a snippet".into(), backend: "fake".into() }])
        }
    }

    #[test]
    fn web_search_tool_spec_requires_query() {
        let pool = Arc::new(SearchPool::new(vec![Box::new(AlwaysSucceedsBackend)]));
        let tool = WebSearchTool::new(pool);
        let spec = tool.spec();
        assert_eq!(spec.name.0, "web.search");
        let required = spec.input_schema.0.get("required").unwrap().as_array().unwrap();
        assert!(required.iter().any(|r| r.as_str() == Some("query")));
    }

    #[test]
    fn web_search_tool_rejects_empty_query() {
        let pool = Arc::new(SearchPool::new(vec![Box::new(AlwaysSucceedsBackend)]));
        let tool = WebSearchTool::new(pool);
        let result = tool.validate(serde_json::json!({"query": ""}));
        assert!(result.is_err());
    }

    #[test]
    fn web_search_tool_rejects_limit_out_of_range() {
        let pool = Arc::new(SearchPool::new(vec![Box::new(AlwaysSucceedsBackend)]));
        let tool = WebSearchTool::new(pool);
        let result = tool.validate(serde_json::json!({"query": "rust", "limit": 50}));
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn web_search_tool_executes_and_returns_hits() {
        let pool = Arc::new(SearchPool::new(vec![Box::new(AlwaysSucceedsBackend)]));
        let tool = WebSearchTool::new(pool);
        let input = tool.validate(serde_json::json!({"query": "rust tokio", "limit": 3})).unwrap();
        let context = ToolContext::new(".");
        let result = tool.execute(&input, &context).await.unwrap();
        let hits = result.output_json.get("hits").unwrap().as_array().unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(result.output_json.get("backend").unwrap().as_str().unwrap(), "fake");
    }

    #[test]
    fn web_fetch_tool_spec_requires_url() {
        let fetch_pool = Arc::new(FetchPool::new());
        let tool = WebFetchTool::new(fetch_pool);
        let spec = tool.spec();
        assert_eq!(spec.name.0, "web.fetch");
        let required = spec.input_schema.0.get("required").unwrap().as_array().unwrap();
        assert!(required.iter().any(|r| r.as_str() == Some("url")));
    }

    #[test]
    fn web_fetch_tool_rejects_non_url_input() {
        let fetch_pool = Arc::new(FetchPool::new());
        let tool = WebFetchTool::new(fetch_pool);
        let result = tool.validate(serde_json::json!({"url": "not-a-url"}));
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn web_fetch_tool_executes_via_injected_plain_fetcher() {
        let mock_fetcher: Box<dyn PageFetcher> = Box::new(MockPageFetcher::new().with_page(
            "https://example.com/page",
            FetchedPage {
                url: "https://example.com/page".into(),
                status: 200,
                content_type: Some("text/html".into()),
                body: "<html><body><p>Hello Fetch</p></body></html>".into(),
                fetched_at: sakha_core::time::now_utc(),
                truncated: false,
            },
        ));
        let fetch_pool = Arc::new(FetchPool::new().with_plain_fetcher(mock_fetcher).skip_remote_stages());
        let tool = WebFetchTool::new(fetch_pool);
        let input = tool.validate(serde_json::json!({"url": "https://example.com/page"})).unwrap();
        let context = ToolContext::new(".");
        let result = tool.execute(&input, &context).await.unwrap();
        let content = result.output_json.get("content_markdown").unwrap().as_str().unwrap();
        assert!(content.contains("Hello Fetch"));
        assert_eq!(result.output_json.get("backend").unwrap().as_str().unwrap(), "plain");
    }

    #[test]
    fn default_search_pool_orders_backends_per_default_priority() {
        let pool = default_search_pool();
        let statuses = pool.statuses();
        let ids: Vec<&str> = statuses.iter().map(|s| s.id).collect();
        assert_eq!(ids, DEFAULT_BACKEND_ORDER);
    }

    #[test]
    fn web_tools_returns_search_and_fetch() {
        let tools = web_tools();
        assert_eq!(tools.len(), 2);
        let names: Vec<String> = tools.iter().map(|t| t.spec().name.0).collect();
        assert!(names.contains(&"web.search".to_string()));
        assert!(names.contains(&"web.fetch".to_string()));
    }
}
