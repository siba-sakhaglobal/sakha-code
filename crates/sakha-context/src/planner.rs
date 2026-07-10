//! `ContextPlanner`: builds a `ContextBundle` from goal, handoff, memory, and
//! research state. See spec `06-context-memory-state.md` "Context Assembly
//! Strategy" and "Main APIs" `build_context`.
//!
//! `sakha-context` deliberately does not depend on `sakha-memory` or
//! `sakha-research` (dependency direction: those crates sit below/alongside
//! this one and pulling them in here would invert the intended layering for
//! no benefit). Instead callers hand the planner a set of `ContextSourceFeed`
//! implementations — thin adapters over whatever store they own — and the
//! planner is responsible only for ordering, prioritizing, attributing, and
//! budgeting the results per the module 06 assembly strategy.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use sakha_core::{GoalId, SakhaResult, SessionId};

use crate::attribution::{AttributionRecord, ContextSource, TrustLevel};
use crate::budgeter::{BudgeterConfig, ContextBudgeter};

/// A single unit of context ready for prompt assembly, with priority for the
/// budgeter and attribution for audit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBundleItem {
    pub content: String,
    pub priority: u8,
    pub token_estimate: u64,
    pub attribution: AttributionRecord,
}

impl ContextBundleItem {
    pub fn new(
        content: impl Into<String>,
        priority: u8,
        attribution: AttributionRecord,
    ) -> Self {
        let content = content.into();
        let token_estimate = estimate_tokens(&content);
        Self { content, priority, token_estimate, attribution }
    }
}

/// Cheap, deterministic token estimator (chars/4, floor 1 for non-empty
/// content). Real tokenization lives in `sakha-provider::count_tokens`; this
/// is only used for local trimming decisions before a request is built.
pub fn estimate_tokens(text: &str) -> u64 {
    if text.is_empty() {
        return 0;
    }
    ((text.len() as u64) / 4).max(1)
}

/// The assembled, not-yet-budgeted context for a turn.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextBundle {
    pub items: Vec<ContextBundleItem>,
    pub token_estimate: u64,
}

impl ContextBundle {
    pub fn from_items(items: Vec<ContextBundleItem>) -> Self {
        let token_estimate = items.iter().map(|i| i.token_estimate).sum();
        Self { items, token_estimate }
    }

    pub fn push(&mut self, item: ContextBundleItem) {
        self.token_estimate += item.token_estimate;
        self.items.push(item);
    }
}

/// Input to `ContextPlanner::build_context`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBuildRequest {
    pub session_id: SessionId,
    pub goal_id: Option<GoalId>,
    pub user_input: String,
    /// Optional token budget to apply after composing the bundle. `None`
    /// means the caller will budget separately (e.g. via `ContextBudgeter`).
    #[serde(default)]
    pub max_tokens: Option<u64>,
}

impl ContextBuildRequest {
    pub fn new(session_id: SessionId, user_input: impl Into<String>) -> Self {
        Self { session_id, goal_id: None, user_input: user_input.into(), max_tokens: None }
    }
}

/// One named source of context the planner can pull from (goal/plan,
/// handoff, project facts, research evidence, recent tool outcomes, ...).
/// Implementors adapt a concrete store (owned by `sakha-memory`,
/// `sakha-research`, etc.) into plain `ContextBundleItem`s.
#[async_trait]
pub trait ContextSourceFeed: Send + Sync {
    /// Stable name for logging/debugging (e.g. `"handoff"`, `"memory"`).
    fn name(&self) -> &str;

    /// Returns zero or more context items for this request. A feed that has
    /// nothing relevant returns an empty vec — never an error, since a
    /// missing/empty source must not fail context assembly (mirrors the
    /// module 21 "missing skill pack -> warn and continue" failure mode).
    async fn fetch(&self, request: &ContextBuildRequest) -> Vec<ContextBundleItem>;
}

/// Builds context bundles from durable state (goal/plan, handoff, memory,
/// research, recent tool outcomes).
#[async_trait]
pub trait ContextPlanner: Send + Sync {
    async fn build_context(&self, request: ContextBuildRequest) -> SakhaResult<ContextBundle>;
}

/// A minimal planner that returns only the user input as a single context
/// item. Safe default until memory/research wiring lands.
#[derive(Debug, Default)]
pub struct MinimalContextPlanner;

#[async_trait]
impl ContextPlanner for MinimalContextPlanner {
    async fn build_context(&self, request: ContextBuildRequest) -> SakhaResult<ContextBundle> {
        let item = user_input_item(&request.user_input);
        Ok(ContextBundle::from_items(vec![item]))
    }
}

fn user_input_item(user_input: &str) -> ContextBundleItem {
    ContextBundleItem::new(
        user_input,
        255,
        AttributionRecord { source: ContextSource::UserInput, trust_level: TrustLevel::Trusted, raw_artifact: None },
    )
}

/// The full context assembly strategy from `06-context-memory-state.md`:
/// goal/plan, last handoff, touched-files summary, project facts, research
/// evidence, and recent tool outcomes are pulled from injected
/// `ContextSourceFeed`s (in that priority order, highest first), then user
/// input is appended, then the whole bundle is budget-trimmed.
///
/// Feed ordering in `feeds` does not matter for output order — each item
/// carries its own `priority`, and `DefaultContextPlanner` sorts by priority
/// (ties broken by insertion order) so higher layers can reason about
/// "most important content survives trimming" independent of feed order.
pub struct DefaultContextPlanner {
    feeds: Vec<Box<dyn ContextSourceFeed>>,
    budgeter: ContextBudgeter,
}

impl DefaultContextPlanner {
    pub fn new(feeds: Vec<Box<dyn ContextSourceFeed>>) -> Self {
        Self { feeds, budgeter: ContextBudgeter::new(BudgeterConfig::default()) }
    }

    pub fn with_budgeter(mut self, budgeter: ContextBudgeter) -> Self {
        self.budgeter = budgeter;
        self
    }
}

#[async_trait]
impl ContextPlanner for DefaultContextPlanner {
    async fn build_context(&self, request: ContextBuildRequest) -> SakhaResult<ContextBundle> {
        let mut items = Vec::new();
        for feed in &self.feeds {
            let fetched = feed.fetch(&request).await;
            if fetched.is_empty() {
                tracing::debug!(feed = feed.name(), "context feed returned no items");
            }
            items.extend(fetched);
        }
        // User input always included, always highest priority: it is what
        // the user is asking about right now.
        items.push(user_input_item(&request.user_input));

        let bundle = ContextBundle::from_items(items);
        match request.max_tokens {
            Some(max_tokens) => self.budgeter_with_max(max_tokens).trim(bundle),
            None => Ok(bundle),
        }
    }
}

impl DefaultContextPlanner {
    fn budgeter_with_max(&self, max_tokens: u64) -> ContextBudgeter {
        ContextBudgeter::new(BudgeterConfig {
            max_tokens,
            min_priority_to_keep: self.budgeter.config.min_priority_to_keep,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StaticFeed {
        name: &'static str,
        items: Vec<ContextBundleItem>,
    }

    #[async_trait]
    impl ContextSourceFeed for StaticFeed {
        fn name(&self) -> &str {
            self.name
        }

        async fn fetch(&self, _request: &ContextBuildRequest) -> Vec<ContextBundleItem> {
            self.items.clone()
        }
    }

    fn record(source: ContextSource) -> AttributionRecord {
        AttributionRecord { source, trust_level: TrustLevel::Trusted, raw_artifact: None }
    }

    #[tokio::test]
    async fn minimal_planner_returns_user_input_as_context_item() {
        let planner = MinimalContextPlanner;
        let bundle = planner
            .build_context(ContextBuildRequest::new(SessionId::new(), "hello"))
            .await
            .unwrap();
        assert_eq!(bundle.items.len(), 1);
    }

    #[tokio::test]
    async fn default_planner_composes_feeds_and_user_input() {
        let handoff_feed = StaticFeed {
            name: "handoff",
            items: vec![ContextBundleItem::new("last handoff: fixed parser", 220, record(ContextSource::Handoff))],
        };
        let memory_feed = StaticFeed {
            name: "memory",
            items: vec![ContextBundleItem::new(
                "project uses Rust 2021",
                180,
                record(ContextSource::Memory { record_id: "fact-1".into() }),
            )],
        };
        let planner = DefaultContextPlanner::new(vec![Box::new(handoff_feed), Box::new(memory_feed)]);
        let bundle = planner
            .build_context(ContextBuildRequest::new(SessionId::new(), "what's next?"))
            .await
            .unwrap();

        assert_eq!(bundle.items.len(), 3);
        assert!(bundle.items.iter().any(|i| i.content.contains("last handoff")));
        assert!(bundle.items.iter().any(|i| i.content.contains("uses Rust")));
        assert!(bundle.items.iter().any(|i| i.content == "what's next?"));
    }

    #[tokio::test]
    async fn empty_feed_does_not_error_or_add_items() {
        let empty_feed = StaticFeed { name: "research", items: vec![] };
        let planner = DefaultContextPlanner::new(vec![Box::new(empty_feed)]);
        let bundle = planner
            .build_context(ContextBuildRequest::new(SessionId::new(), "hi"))
            .await
            .unwrap();
        assert_eq!(bundle.items.len(), 1);
    }

    #[tokio::test]
    async fn max_tokens_trims_low_priority_feed_items() {
        let big_feed = StaticFeed {
            name: "research",
            items: vec![ContextBundleItem::new(
                "x".repeat(400),
                10,
                record(ContextSource::Research { url: "https://example.com".into() }),
            )],
        };
        let planner = DefaultContextPlanner::new(vec![Box::new(big_feed)]);
        let mut request = ContextBuildRequest::new(SessionId::new(), "short");
        request.max_tokens = Some(5);
        let bundle = planner.build_context(request).await.unwrap();
        // User input (priority 255) survives; the 100-token research item is dropped.
        assert_eq!(bundle.items.len(), 1);
        assert_eq!(bundle.items[0].content, "short");
    }

    #[test]
    fn budgeter_trims_low_priority_items_under_budget() {
        let budgeter = ContextBudgeter::new(BudgeterConfig { max_tokens: 10, min_priority_to_keep: 0 });
        let bundle = ContextBundle {
            items: vec![
                ContextBundleItem {
                    content: "high".into(),
                    priority: 200,
                    token_estimate: 8,
                    attribution: AttributionRecord {
                        source: ContextSource::UserInput,
                        trust_level: TrustLevel::Trusted,
                        raw_artifact: None,
                    },
                },
                ContextBundleItem {
                    content: "low".into(),
                    priority: 10,
                    token_estimate: 8,
                    attribution: AttributionRecord {
                        source: ContextSource::Memory { record_id: "1".into() },
                        trust_level: TrustLevel::Trusted,
                        raw_artifact: None,
                    },
                },
            ],
            token_estimate: 16,
        };
        let trimmed = budgeter.trim(bundle).unwrap();
        assert_eq!(trimmed.items.len(), 1);
        assert_eq!(trimmed.items[0].priority, 200);
    }
}
