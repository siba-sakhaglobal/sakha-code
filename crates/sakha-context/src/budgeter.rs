//! `ContextBudgeter`: trims a context bundle (and, for the prompt system, an
//! assembled prompt's sections) to fit within a token budget by priority.
//! See spec `06-context-memory-state.md` step "Apply budget" and
//! `modules/21-prompt-system.md` "Token estimate over budget -> return
//! `PromptOverBudget` so the planner trims, never silently truncate
//! mid-section."

use serde::{Deserialize, Serialize};

use sakha_core::{SakhaError, SakhaResult};

use crate::planner::ContextBundle;
use crate::prompt::PromptSection;

/// Configuration for how aggressively to trim context.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BudgeterConfig {
    pub max_tokens: u64,
    /// Minimum priority (0-255, higher = more important) below which items
    /// are dropped first, regardless of whether they'd still fit in budget.
    pub min_priority_to_keep: u8,
}

impl Default for BudgeterConfig {
    fn default() -> Self {
        Self { max_tokens: 32_000, min_priority_to_keep: 0 }
    }
}

/// Trims `ContextBundle`s (and prompt sections) to fit a token budget,
/// dropping lowest-priority items first.
pub struct ContextBudgeter {
    pub config: BudgeterConfig,
}

impl ContextBudgeter {
    pub fn new(config: BudgeterConfig) -> Self {
        Self { config }
    }

    /// Returns a trimmed copy of `bundle` whose total token estimate is
    /// within `self.config.max_tokens`, preferring to keep the
    /// highest-priority items. Items whose priority is below
    /// `min_priority_to_keep` are dropped even if the budget would still fit
    /// them, per `BudgeterConfig::min_priority_to_keep`.
    pub fn trim(&self, mut bundle: ContextBundle) -> SakhaResult<ContextBundle> {
        bundle.items.sort_by(|a, b| b.priority.cmp(&a.priority));
        let mut total = 0u64;
        let mut kept = Vec::new();
        for item in bundle.items {
            if item.priority < self.config.min_priority_to_keep {
                continue;
            }
            if total + item.token_estimate > self.config.max_tokens {
                continue;
            }
            total += item.token_estimate;
            kept.push(item);
        }
        bundle.items = kept;
        bundle.token_estimate = total;
        Ok(bundle)
    }

    /// Trims a set of assembled `PromptSection`s to fit `max_tokens`,
    /// dropping whole lowest-priority sections first — never truncating
    /// mid-section, per module 21's failure-mode contract. Original relative
    /// order among kept sections is preserved (so volatile/cache-prefix
    /// ordering from `PromptAssembler::assemble` survives trimming).
    ///
    /// Returns `Err(SakhaError::budget(..))` if even the single
    /// highest-priority section alone exceeds `max_tokens` — that case can't
    /// be resolved by dropping other sections, so the caller must reduce
    /// content rather than have this silently return an empty prompt.
    pub fn trim_sections(&self, sections: Vec<PromptSection>, max_tokens: u32) -> SakhaResult<Vec<PromptSection>> {
        if sections.is_empty() {
            return Ok(sections);
        }

        let mut indexed: Vec<(usize, u32)> =
            sections.iter().enumerate().map(|(i, s)| (i, section_tokens(s))).collect();
        // Sort by priority descending (stable, so ties keep original order)
        // to decide *which* sections survive; we still emit in original order.
        indexed.sort_by(|a, b| sections[b.0].priority.cmp(&sections[a.0].priority));

        let top_priority_tokens = indexed.first().map(|(_, t)| *t).unwrap_or(0);
        if top_priority_tokens > max_tokens {
            return Err(SakhaError::budget(
                "sakha-context",
                format!(
                    "highest-priority prompt section alone ({top_priority_tokens} tokens) exceeds budget ({max_tokens} tokens)"
                ),
            ));
        }

        let mut total = 0u32;
        let mut keep = vec![false; sections.len()];
        for (idx, tokens) in indexed {
            if total + tokens > max_tokens {
                continue;
            }
            total += tokens;
            keep[idx] = true;
        }

        Ok(sections
            .into_iter()
            .enumerate()
            .filter_map(|(i, s)| if keep[i] { Some(s) } else { None })
            .collect())
    }
}

fn section_tokens(section: &PromptSection) -> u32 {
    (section.content.len() as u32) / 4
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attribution::{AttributionRecord, ContextSource, TrustLevel};
    use crate::planner::ContextBundleItem;
    use crate::prompt::SectionId;

    fn item(content: &str, priority: u8) -> ContextBundleItem {
        ContextBundleItem::new(
            content,
            priority,
            AttributionRecord { source: ContextSource::UserInput, trust_level: TrustLevel::Trusted, raw_artifact: None },
        )
    }

    #[test]
    fn min_priority_to_keep_drops_low_priority_even_if_it_fits() {
        let budgeter = ContextBudgeter::new(BudgeterConfig { max_tokens: 1000, min_priority_to_keep: 50 });
        let bundle = ContextBundle::from_items(vec![item("keep", 100), item("drop", 10)]);
        let trimmed = budgeter.trim(bundle).unwrap();
        assert_eq!(trimmed.items.len(), 1);
        assert_eq!(trimmed.items[0].content, "keep");
    }

    fn section(id: &str, priority: u8, content: &str) -> PromptSection {
        PromptSection { id: SectionId::new(id), priority, content: content.into(), volatile: false }
    }

    #[test]
    fn trim_sections_drops_lowest_priority_whole_sections() {
        let budgeter = ContextBudgeter::new(BudgeterConfig::default());
        let sections = vec![
            section("identity", 255, "you are sakha"), // ~3 tokens
            section("low", 10, &"x".repeat(400)),       // ~100 tokens
        ];
        let trimmed = budgeter.trim_sections(sections, 10).unwrap();
        assert_eq!(trimmed.len(), 1);
        assert_eq!(trimmed[0].id, SectionId::new("identity"));
    }

    #[test]
    fn trim_sections_preserves_original_order_among_survivors() {
        let budgeter = ContextBudgeter::new(BudgeterConfig::default());
        let sections = vec![section("a", 200, "aa"), section("b", 100, "bb"), section("c", 150, "cc")];
        let trimmed = budgeter.trim_sections(sections, 1000).unwrap();
        let ids: Vec<String> = trimmed.iter().map(|s| s.id.0.clone()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn trim_sections_never_truncates_mid_section() {
        let budgeter = ContextBudgeter::new(BudgeterConfig::default());
        let original = "keep me whole".to_string();
        let sections = vec![section("only", 255, &original)];
        let trimmed = budgeter.trim_sections(sections, 1000).unwrap();
        assert_eq!(trimmed[0].content, original);
    }

    #[test]
    fn trim_sections_errors_when_top_priority_section_exceeds_budget() {
        let budgeter = ContextBudgeter::new(BudgeterConfig::default());
        let sections = vec![section("huge", 255, &"x".repeat(4000))];
        let result = budgeter.trim_sections(sections, 10);
        assert!(result.is_err());
    }
}
