//! sakha-research: web search/fetch/extract, source scoring, evidence packs,
//! and injection filtering.
//!
//! Public API — see spec `modules/11-web-search-research.md` and
//! `crates/crate-work-breakdown.md`. Search/fetch go behind traits with
//! `Null*`/`Mock*` fallbacks so tests never require live network access.

pub mod compress;
pub mod evidence;
pub mod extract;
pub mod fetch;
pub mod injection_filter;
pub mod scoring;
pub mod search;

pub use compress::{compress_document, compress_documents, default_compressor, CompressedDocument};
pub use evidence::{Citation, EvidencePack};
pub use extract::{ExtractedDocument, Extractor, Html2TextExtractor, PlainTextExtractor};
pub use fetch::{FetchRequest, FetchedPage, MockPageFetcher, NullPageFetcher, PageFetcher, ReqwestPageFetcher};
pub use injection_filter::{InjectionFilter, InjectionScanResult};
pub use scoring::{SourceScore, SourceScorer};
pub use search::{
    dedupe_results, plan_queries, HttpSearchClient, MockSearchClient, NullSearchClient, SearchClient,
    SearchEndpointConfig, SearchEndpointKind, SearchPlan, SearchProvider, SearchQuery, SearchResult,
};

use serde::{Deserialize, Serialize};

/// How long a visited URL's extracted facts are considered fresh before the
/// research loop should re-search it. See spec "Long-Running Research"
/// ("Re-search unstable facts", "Refresh package/API docs before
/// implementation").
pub const DEFAULT_STALE_AFTER_HOURS: i64 = 24;

/// One entry in a research loop's notebook of visited URLs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisitedUrl {
    pub url: String,
    pub visited_at: chrono::DateTime<chrono::Utc>,
}

/// Tracks state for a long-running research loop: visited URLs, stale
/// claims, and iteration count. See spec "Long-Running Research" and
/// "Research Loop" step 11 "Store research memory".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResearchLoopState {
    pub visited: Vec<VisitedUrl>,
    pub stale_claim_urls: Vec<String>,
    pub iterations: u32,

    // Legacy field kept for backward-compat with callers/tests that only
    // track raw URL strings without timestamps.
    #[serde(default)]
    pub visited_urls: Vec<String>,
}

impl ResearchLoopState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a visit, updating both the timestamped notebook and the
    /// legacy flat URL list.
    pub fn record_visit(&mut self, url: impl Into<String>) {
        let url = url.into();
        if !self.visited_urls.contains(&url) {
            self.visited_urls.push(url.clone());
        }
        self.visited.push(VisitedUrl { url, visited_at: sakha_core::time::now_utc() });
        self.iterations += 1;
    }

    /// True if `url` has already been visited in this loop (dedup check
    /// before issuing a fetch). See spec "Research Loop" step 4 and "Track
    /// visited URLs".
    pub fn already_visited(&self, url: &str) -> bool {
        let normalized = search::normalize_url(url);
        self.visited_urls.iter().any(|u| search::normalize_url(u) == normalized)
    }

    /// Returns URLs whose most recent visit is older than `max_age_hours`
    /// and thus should be re-searched before relying on their facts. See
    /// spec "Long-Running Research" ("Re-search unstable facts").
    pub fn stale_urls(&self, now: chrono::DateTime<chrono::Utc>, max_age_hours: i64) -> Vec<String> {
        let mut latest: std::collections::HashMap<String, chrono::DateTime<chrono::Utc>> =
            std::collections::HashMap::new();
        for v in &self.visited {
            latest
                .entry(v.url.clone())
                .and_modify(|t| {
                    if v.visited_at > *t {
                        *t = v.visited_at;
                    }
                })
                .or_insert(v.visited_at);
        }
        latest
            .into_iter()
            .filter(|(_, visited_at)| (now - *visited_at).num_hours() >= max_age_hours)
            .map(|(url, _)| url)
            .collect()
    }

    pub fn mark_stale_claim(&mut self, url: impl Into<String>) {
        let url = url.into();
        if !self.stale_claim_urls.contains(&url) {
            self.stale_claim_urls.push(url);
        }
    }
}

/// Stop conditions for a research loop, guarding against unbounded search
/// spend. See spec Implementation Tasks item 9 "Implement loop stop
/// conditions".
#[derive(Debug, Clone, Copy)]
pub struct ResearchStopConditions {
    pub max_iterations: u32,
    pub max_visited_urls: usize,
}

impl Default for ResearchStopConditions {
    fn default() -> Self {
        Self { max_iterations: 25, max_visited_urls: 40 }
    }
}

impl ResearchStopConditions {
    /// Whether the loop should stop given current state.
    pub fn should_stop(&self, state: &ResearchLoopState) -> bool {
        state.iterations >= self.max_iterations || state.visited_urls.len() >= self.max_visited_urls
    }
}

pub fn crate_name() -> &'static str {
    "sakha-research"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_scoring_prioritizes_official_docs() {
        let scorer = SourceScorer::new();
        let official = ExtractedDocument {
            url: "https://docs.rust-lang.org/book".into(),
            title: None,
            text: "guide".into(),
            published_at: None,
        };
        let random = ExtractedDocument {
            url: "https://example.com/blog".into(),
            title: None,
            text: "guide".into(),
            published_at: None,
        };
        let official_score = scorer.score(&official).composite();
        let random_score = scorer.score(&random).composite();
        assert!(official_score > random_score);
    }

    #[test]
    fn extraction_produces_text_from_page() {
        let extractor = PlainTextExtractor;
        let page = FetchedPage {
            url: "https://example.com".into(),
            status: 200,
            content_type: Some("text/plain".into()),
            body: "hello world".into(),
            fetched_at: sakha_core::time::now_utc(),
            truncated: false,
        };
        let doc = extractor.extract(&page).unwrap();
        assert_eq!(doc.text, "hello world");
    }

    #[test]
    fn evidence_pack_cites_fetched_sources() {
        let mut pack = EvidencePack::new("what is Rust?");
        pack.citations.push(Citation {
            url: "https://rust-lang.org".into(),
            quote: "Rust is a systems programming language".into(),
            score: SourceScore::default(),
        });
        assert!(pack.is_grounded());
    }

    #[test]
    fn duplicate_url_detection_via_loop_state() {
        let mut state = ResearchLoopState::default();
        state.record_visit("https://example.com");
        assert!(state.already_visited("https://example.com/"));
        assert!(state.already_visited("https://EXAMPLE.com"));
        assert!(!state.already_visited("https://other.example.com"));
    }

    #[test]
    fn long_running_loop_flags_stale_source_for_refresh() {
        let mut state = ResearchLoopState::default();
        state.visited.push(VisitedUrl {
            url: "https://example.com/api-docs".into(),
            visited_at: sakha_core::time::now_utc() - chrono::Duration::hours(48),
        });
        state.visited_urls.push("https://example.com/api-docs".into());

        let now = sakha_core::time::now_utc();
        let stale = state.stale_urls(now, DEFAULT_STALE_AFTER_HOURS);
        assert_eq!(stale, vec!["https://example.com/api-docs".to_string()]);
    }

    #[test]
    fn stop_conditions_trigger_after_max_iterations() {
        let mut state = ResearchLoopState::default();
        let stop = ResearchStopConditions { max_iterations: 3, max_visited_urls: 100 };
        for i in 0..3 {
            assert!(!stop.should_stop(&state));
            state.record_visit(format!("https://example.com/{i}"));
        }
        assert!(stop.should_stop(&state));
    }

    #[test]
    fn query_planning_for_library_docs() {
        let plan = plan_queries("documentation for the serde crate library");
        assert!(!plan.queries.is_empty());
        assert!(plan.queries.iter().any(|q| q.provider == SearchProvider::DocumentationSite));
    }
}
