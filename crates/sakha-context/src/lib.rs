//! sakha-context: context planning, budgeting, attribution, and prompt assembly.
//!
//! Public API skeleton — see spec `06-context-memory-state.md`,
//! `modules/21-prompt-system.md`, and `crates/crate-work-breakdown.md`. Key
//! contract: `PromptAssembler` (module 21) is the only place prompt strings
//! get concatenated.

pub mod attribution;
pub mod budgeter;
pub mod planner;
pub mod prompt;

pub use attribution::{AttributionRecord, ContextSource, TrustLevel};
pub use budgeter::{BudgeterConfig, ContextBudgeter};
pub use planner::{
    estimate_tokens, ContextBuildRequest, ContextBundle, ContextBundleItem, ContextPlanner,
    ContextSourceFeed, DefaultContextPlanner, MinimalContextPlanner,
};
pub use prompt::{
    AssembledPrompt, DefaultPromptAssembler, PromptAssembler, PromptRequest, PromptSection,
    PromptTemplate, SectionId, SkillPack, SkillPackLoadWarning, SkillPackLoader, SkillPackOrigin,
};

pub fn crate_name() -> &'static str {
    "sakha-context"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn minimal_planner_returns_user_input_as_context_item() {
        let planner = MinimalContextPlanner;
        let bundle = planner
            .build_context(ContextBuildRequest::new(sakha_core::SessionId::new(), "hello"))
            .await
            .unwrap();
        assert_eq!(bundle.items.len(), 1);
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

    /// End-to-end golden test: context planning feeds into prompt assembly
    /// (via a memory digest built from the context bundle) the same way
    /// `sakha-agent` will wire them, and the result is byte-identical across
    /// repeated runs with the same inputs.
    #[tokio::test]
    async fn end_to_end_context_to_prompt_is_deterministic() {
        let planner = MinimalContextPlanner;
        let request = ContextBuildRequest::new(sakha_core::SessionId::new(), "add a retry to the http client");
        let bundle = planner.build_context(request.clone()).await.unwrap();

        let memory_digest = bundle.items.iter().map(|i| i.content.as_str()).collect::<Vec<_>>().join("\n");

        let assembler = DefaultPromptAssembler;
        let prompt_req = PromptRequest {
            session_id: sakha_core::SessionId::new(),
            agent_role: "coding-agent".into(),
            tools: vec!["file.read".into(), "file.write".into()],
            memory_digest: Some(memory_digest.clone()),
            loop_policy: None,
            user_input: request.user_input.clone(),
            skills: Vec::new(),
            max_tokens: None,
        };

        let a = assembler.assemble(&prompt_req).unwrap();
        let b = assembler.assemble(&prompt_req).unwrap();
        assert_eq!(a.render(), b.render());
        assert!(a.render().contains("add a retry to the http client"));
    }

    /// Golden test combining skill pack precedence with prompt assembly:
    /// registering the same skill at builtin/user/workspace origins and
    /// resolving it must surface the workspace version's sections in the
    /// assembled prompt.
    #[test]
    fn skill_pack_precedence_flows_into_assembled_prompt() {
        let mut loader = SkillPackLoader::new();
        loader.register(
            SkillPackOrigin::Builtin,
            SkillPack {
                name: "review".into(),
                version: "builtin".into(),
                sections: vec![PromptSection {
                    id: SectionId::new("policy"),
                    priority: 100,
                    content: "builtin review policy".into(),
                    volatile: false,
                }],
                required_tools: vec![],
            },
        );
        loader.register(
            SkillPackOrigin::Workspace,
            SkillPack {
                name: "review".into(),
                version: "workspace".into(),
                sections: vec![PromptSection {
                    id: SectionId::new("policy"),
                    priority: 100,
                    content: "workspace review policy".into(),
                    volatile: false,
                }],
                required_tools: vec![],
            },
        );

        let resolved = loader.resolve("review").unwrap().unwrap().clone();
        assert_eq!(resolved.version, "workspace");

        let assembler = DefaultPromptAssembler;
        let req = PromptRequest {
            session_id: sakha_core::SessionId::new(),
            agent_role: "coding-agent".into(),
            tools: vec![],
            memory_digest: None,
            loop_policy: None,
            user_input: "review this diff".into(),
            skills: vec![resolved],
            max_tokens: None,
        };
        let assembled = assembler.assemble(&req).unwrap();
        assert!(assembled.render().contains("workspace review policy"));
        assert!(!assembled.render().contains("builtin review policy"));
    }
}
