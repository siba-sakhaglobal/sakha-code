//! Source scoring: ranks fetched documents by reliability/relevance signals.
//! See spec "Source Scoring".

use serde::{Deserialize, Serialize};

use crate::extract::ExtractedDocument;

/// A composite score for a source, per spec "Source Scoring" signals.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct SourceScore {
    pub is_primary_source: bool,
    pub is_official_docs: bool,
    pub relevance: f32,
    pub freshness: f32,
    pub has_code_examples: bool,
    pub conflicts_with_other_sources: bool,
    /// True when the page looks paywalled or heavily JS-rendered, so the
    /// fetched text is likely incomplete/unreliable. See spec
    /// "Paywall/dynamic content risk".
    pub paywall_or_dynamic_risk: bool,
}

impl SourceScore {
    /// A single scalar combining the signals, higher is better. Weighting is
    /// a placeholder heuristic until tuned against real evals.
    pub fn composite(&self) -> f32 {
        let mut score = self.relevance * 0.5 + self.freshness * 0.2;
        if self.is_primary_source {
            score += 0.15;
        }
        if self.is_official_docs {
            score += 0.15;
        }
        if self.has_code_examples {
            score += 0.05;
        }
        if self.conflicts_with_other_sources {
            score -= 0.2;
        }
        if self.paywall_or_dynamic_risk {
            score -= 0.1;
        }
        score.clamp(0.0, 1.0)
    }
}

/// Known official documentation host fragments. Heuristic allowlist, not
/// exhaustive; extend as needed.
const OFFICIAL_DOC_MARKERS: &[&str] = &[
    "docs.",
    "/docs/",
    "developer.",
    "devdocs.",
    "readthedocs.io",
    "docs.rs",
    "pkg.go.dev",
    ".github.io",
];

/// Known primary-source host fragments (upstream project homes, registries,
/// standards bodies) as opposed to secondary commentary (blogs, forums).
const PRIMARY_SOURCE_MARKERS: &[&str] =
    &["github.com", "crates.io", "npmjs.com", "pypi.org", "rfc-editor.org", "w3.org"];

/// Host fragments that commonly gate content behind a paywall or render
/// primarily client-side (so a plain HTTP fetch yields little usable text).
const PAYWALL_OR_DYNAMIC_MARKERS: &[&str] =
    &["medium.com", "nytimes.com", "wsj.com", "ft.com", "bloomberg.com"];

/// Scores an `ExtractedDocument` against a query for relevance, plus
/// source-shape heuristics (official docs, primary source, freshness,
/// paywall risk, code examples). See spec "Source Scoring" signals.
pub struct SourceScorer {
    /// Optional reference timestamp for freshness scoring; defaults to now.
    now: chrono::DateTime<chrono::Utc>,
}

impl SourceScorer {
    pub fn new() -> Self {
        Self { now: sakha_core::time::now_utc() }
    }

    pub fn with_now(now: chrono::DateTime<chrono::Utc>) -> Self {
        Self { now }
    }

    /// Scores a document with no query context (relevance defaults to a mid
    /// value since it cannot be judged without a query).
    pub fn score(&self, doc: &ExtractedDocument) -> SourceScore {
        self.score_for_query(doc, None)
    }

    /// Scores a document against an optional query string. When `query` is
    /// provided, relevance is estimated via keyword overlap between the
    /// query and the document title/text.
    pub fn score_for_query(&self, doc: &ExtractedDocument, query: Option<&str>) -> SourceScore {
        let url_lower = doc.url.to_lowercase();

        let is_official_docs = OFFICIAL_DOC_MARKERS.iter().any(|m| url_lower.contains(m));
        let is_primary_source = PRIMARY_SOURCE_MARKERS.iter().any(|m| url_lower.contains(m));
        let paywall_or_dynamic_risk = PAYWALL_OR_DYNAMIC_MARKERS.iter().any(|m| url_lower.contains(m));
        let has_code_examples = doc.text.contains("```") || doc.text.contains("    fn ") || doc.text.contains("def ");

        let relevance = match query {
            Some(q) => keyword_overlap(q, &doc.text, doc.title.as_deref()),
            None => 0.5,
        };

        let freshness = self.freshness_score(doc.published_at);

        SourceScore {
            is_primary_source,
            is_official_docs,
            relevance,
            freshness,
            has_code_examples,
            conflicts_with_other_sources: false,
            paywall_or_dynamic_risk,
        }
    }

    fn freshness_score(&self, published_at: Option<chrono::DateTime<chrono::Utc>>) -> f32 {
        match published_at {
            None => 0.5,
            Some(ts) => {
                let age_days = (self.now - ts).num_days().max(0) as f32;
                // Linear decay: brand-new = 1.0, ~2 years old = ~0.0.
                (1.0 - age_days / 730.0).clamp(0.0, 1.0)
            }
        }
    }

    /// Marks `score` as conflicting with other retrieved sources (caller
    /// detects contradictions across a result set and applies this).
    pub fn mark_conflicting(mut score: SourceScore) -> SourceScore {
        score.conflicts_with_other_sources = true;
        score
    }
}

impl Default for SourceScorer {
    fn default() -> Self {
        Self::new()
    }
}

/// Fraction of query keywords (lowercased, length > 2) found in the
/// title+text, clamped to [0, 1]. Simple bag-of-words overlap; no ML.
fn keyword_overlap(query: &str, text: &str, title: Option<&str>) -> f32 {
    let haystack = format!("{} {}", title.unwrap_or(""), text).to_lowercase();
    let query_lower = query.to_lowercase();
    let keywords: Vec<&str> = query_lower.split_whitespace().filter(|w| w.len() > 2).collect();
    if keywords.is_empty() {
        return 0.5;
    }
    let matched = keywords.iter().filter(|k| haystack.contains(*k)).count();
    (matched as f32 / keywords.len() as f32).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(url: &str, text: &str) -> ExtractedDocument {
        ExtractedDocument { url: url.into(), title: None, text: text.into(), published_at: None }
    }

    #[test]
    fn official_docs_score_higher_than_blog() {
        let scorer = SourceScorer::new();
        let official = doc("https://docs.rust-lang.org/book", "guide");
        let blog = doc("https://randomblog.example.com/post", "guide");
        assert!(scorer.score(&official).composite() > scorer.score(&blog).composite());
    }

    #[test]
    fn primary_source_detected_for_github() {
        let scorer = SourceScorer::new();
        let s = scorer.score(&doc("https://github.com/rust-lang/rust", "source"));
        assert!(s.is_primary_source);
    }

    #[test]
    fn paywall_marker_reduces_composite_score() {
        let scorer = SourceScorer::new();
        let paywalled = scorer.score(&doc("https://medium.com/some-post", "content"));
        let clean = scorer.score(&doc("https://example.com/some-post", "content"));
        assert!(paywalled.composite() < clean.composite());
    }

    #[test]
    fn freshness_decays_with_age() {
        let now = chrono::Utc::now();
        let scorer = SourceScorer::with_now(now);
        let mut fresh = doc("https://example.com/a", "text");
        fresh.published_at = Some(now - chrono::Duration::days(1));
        let mut stale = doc("https://example.com/b", "text");
        stale.published_at = Some(now - chrono::Duration::days(700));
        assert!(scorer.score(&fresh).freshness > scorer.score(&stale).freshness);
    }

    #[test]
    fn relevance_scores_higher_for_keyword_overlap() {
        let scorer = SourceScorer::new();
        let relevant = doc("https://example.com/a", "this page explains async rust tokio runtime");
        let irrelevant = doc("https://example.com/b", "this page is about gardening tips");
        let relevant_score = scorer.score_for_query(&relevant, Some("async tokio runtime"));
        let irrelevant_score = scorer.score_for_query(&irrelevant, Some("async tokio runtime"));
        assert!(relevant_score.relevance > irrelevant_score.relevance);
    }

    #[test]
    fn conflicting_flag_reduces_composite() {
        let scorer = SourceScorer::new();
        let base = scorer.score(&doc("https://example.com/a", "text"));
        let conflicting = SourceScorer::mark_conflicting(base);
        assert!(conflicting.composite() < base.composite());
    }

    #[test]
    fn code_examples_detected() {
        let scorer = SourceScorer::new();
        let s = scorer.score(&doc("https://example.com/a", "here is a snippet:\n```rust\nfn main() {}\n```"));
        assert!(s.has_code_examples);
    }
}
