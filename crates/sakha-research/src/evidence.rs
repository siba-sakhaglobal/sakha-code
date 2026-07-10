//! Evidence packs: the citable, verifiable output of a research loop. See
//! spec "Research Loop" step "Build evidence pack".

use serde::{Deserialize, Serialize};

use sakha_core::GoalId;

use crate::scoring::SourceScore;

/// A single citation referencing a fetched source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    pub url: String,
    pub quote: String,
    pub score: SourceScore,
}

impl Citation {
    pub fn new(url: impl Into<String>, quote: impl Into<String>, score: SourceScore) -> Self {
        Self { url: url.into(), quote: quote.into(), score }
    }

    /// Renders as a Markdown-style inline citation, e.g. `[1](https://...)`.
    pub fn format_markdown(&self, index: usize) -> String {
        format!("[{index}]({url})", index = index, url = self.url)
    }
}

/// The finished output of a research loop: claim -> supporting citations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidencePack {
    pub goal_id: Option<GoalId>,
    pub question: String,
    pub answer: String,
    pub citations: Vec<Citation>,
    pub built_at: chrono::DateTime<chrono::Utc>,
}

impl EvidencePack {
    pub fn new(question: impl Into<String>) -> Self {
        Self {
            goal_id: None,
            question: question.into(),
            answer: String::new(),
            citations: Vec::new(),
            built_at: sakha_core::time::now_utc(),
        }
    }

    pub fn with_goal_id(mut self, goal_id: GoalId) -> Self {
        self.goal_id = Some(goal_id);
        self
    }

    pub fn with_answer(mut self, answer: impl Into<String>) -> Self {
        self.answer = answer.into();
        self
    }

    pub fn add_citation(&mut self, citation: Citation) {
        self.citations.push(citation);
    }

    /// True if the pack has at least one citation and every citation
    /// references a non-empty URL and quote. Stub-level grounding check:
    /// real claim-level grounding (matching each sentence in `answer` to a
    /// citation) is future work; this at minimum rejects packs that claim to
    /// be grounded but carry no usable source text.
    pub fn is_grounded(&self) -> bool {
        !self.citations.is_empty()
            && self
                .citations
                .iter()
                .all(|c| !c.url.trim().is_empty() && !c.quote.trim().is_empty())
    }

    /// Formats the answer with numbered footnote-style citations appended,
    /// suitable for a user-facing response. See spec "Cite sources in
    /// user-facing answers".
    pub fn format_with_citations(&self) -> String {
        let mut out = self.answer.clone();
        if !self.citations.is_empty() {
            out.push_str("\n\nSources:\n");
            for (i, c) in self.citations.iter().enumerate() {
                out.push_str(&format!("{}. {}\n", i + 1, c.url));
            }
        }
        out
    }

    /// Distinct source URLs cited in this pack.
    pub fn cited_urls(&self) -> Vec<String> {
        let mut urls: Vec<String> = self.citations.iter().map(|c| c.url.clone()).collect();
        urls.sort();
        urls.dedup();
        urls
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score() -> SourceScore {
        SourceScore { relevance: 0.8, ..Default::default() }
    }

    #[test]
    fn new_pack_is_not_grounded_without_citations() {
        let pack = EvidencePack::new("what is rust?");
        assert!(!pack.is_grounded());
    }

    #[test]
    fn pack_with_citation_is_grounded() {
        let mut pack = EvidencePack::new("what is rust?");
        pack.add_citation(Citation::new("https://rust-lang.org", "Rust is a systems language", score()));
        assert!(pack.is_grounded());
    }

    #[test]
    fn pack_with_empty_quote_citation_is_not_grounded() {
        let mut pack = EvidencePack::new("what is rust?");
        pack.add_citation(Citation::new("https://rust-lang.org", "", score()));
        assert!(!pack.is_grounded());
    }

    #[test]
    fn format_with_citations_lists_sources() {
        let mut pack = EvidencePack::new("q").with_answer("Rust is great.");
        pack.add_citation(Citation::new("https://rust-lang.org", "quote", score()));
        let formatted = pack.format_with_citations();
        assert!(formatted.contains("Rust is great."));
        assert!(formatted.contains("https://rust-lang.org"));
    }

    #[test]
    fn cited_urls_deduplicates_and_sorts() {
        let mut pack = EvidencePack::new("q");
        pack.add_citation(Citation::new("https://b.com", "x", score()));
        pack.add_citation(Citation::new("https://a.com", "y", score()));
        pack.add_citation(Citation::new("https://a.com", "z", score()));
        assert_eq!(pack.cited_urls(), vec!["https://a.com".to_string(), "https://b.com".to_string()]);
    }
}
