//! `PromptAssembler`: the single place every prompt gets assembled. See spec
//! `modules/21-prompt-system.md` and `04-core-domain-model.md`. No other
//! module should concatenate prompt strings directly.

use serde::{Deserialize, Serialize};

use sakha_core::{SakhaError, SakhaResult, SessionId};

use super::skills::SkillPack;
use super::templates::PromptTemplate;

/// Stable identifier for a prompt section (identity, safety, workspace
/// facts, tool usage rules, loop policy, memory digest, ...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionId(pub String);

impl SectionId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// One ordered, prioritized block of the assembled prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptSection {
    pub id: SectionId,
    /// Higher priority sections are kept first when trimming under budget.
    pub priority: u8,
    pub content: String,
    /// Volatile sections (recent context) go last so stable-prefix caching
    /// works; non-volatile sections form the stable cache prefix.
    pub volatile: bool,
}

/// Input describing what a prompt needs to contain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptRequest {
    pub session_id: SessionId,
    pub agent_role: String,
    pub tools: Vec<String>,
    pub memory_digest: Option<String>,
    pub loop_policy: Option<String>,
    pub user_input: String,
    /// Already-resolved skill packs to fold into the stable prefix, in the
    /// order given (typically the order returned by
    /// `SkillPackLoader::resolve` for each skill the current agent role/loop
    /// needs). Resolution/precedence is `SkillPackLoader`'s job; the
    /// assembler only renders what it's handed, so `assemble` stays pure and
    /// golden-test-friendly.
    #[serde(default)]
    pub skills: Vec<SkillPack>,
    /// Optional hard cap on the assembled prompt's token estimate. When set
    /// and the fully-assembled prompt exceeds it, sections are trimmed by
    /// priority (dropping whole lowest-priority sections, never truncating
    /// mid-section) rather than silently truncated. `user_input` (priority
    /// 0, always present) is the last thing dropped since without it the
    /// prompt is meaningless.
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

/// The fully assembled prompt, ready to send as the model's system/user
/// messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssembledPrompt {
    pub sections: Vec<PromptSection>,
    pub token_estimate: u32,
    /// Length (in the rendered text) of the stable, cache-friendly prefix.
    pub cache_prefix_len: u32,
}

impl AssembledPrompt {
    /// Renders all sections in order into a single string, stable sections
    /// first (already guaranteed by construction order in `assemble`).
    pub fn render(&self) -> String {
        self.sections.iter().map(|s| s.content.as_str()).collect::<Vec<_>>().join("\n\n")
    }
}

/// Assembles prompts deterministically from ordered sections. See
/// `modules/21-prompt-system.md` interfaces.
pub trait PromptAssembler: Send + Sync {
    fn assemble(&self, req: &PromptRequest) -> SakhaResult<AssembledPrompt>;
}

/// A minimal deterministic assembler: identity + tools + skill packs +
/// memory digest + loop policy (stable prefix) followed by user input
/// (volatile).
#[derive(Debug, Default)]
pub struct DefaultPromptAssembler;

/// Priority bands for built-in section kinds. Skill pack sections keep
/// whatever priority they declare in their own `skill.toml`, but typically
/// fall in the 100-180 band (below identity/tools, above memory digest) —
/// callers are free to author skill packs outside that band deliberately.
mod priority {
    pub const IDENTITY: u8 = 255;
    pub const TOOLS: u8 = 200;
    pub const MEMORY_DIGEST: u8 = 150;
    pub const LOOP_POLICY: u8 = 140;
    pub const USER_INPUT: u8 = 0;
}

/// Built-in section templates, kept as typed `PromptTemplate`s rather than
/// ad-hoc `format!` at the call site, per module 21.
fn identity_template() -> PromptTemplate {
    PromptTemplate::new("identity", "You are the Sakha Coding Agent, acting as: {{agent_role}}.")
}

fn tools_template() -> PromptTemplate {
    PromptTemplate::new("tools", "Available tools: {{tool_list}}")
}

impl PromptAssembler for DefaultPromptAssembler {
    fn assemble(&self, req: &PromptRequest) -> SakhaResult<AssembledPrompt> {
        if req.agent_role.trim().is_empty() {
            return Err(SakhaError::invalid_input("sakha-context", "agent_role must not be empty"));
        }

        let mut sections = Vec::new();
        let identity_content = identity_template().render(&[("agent_role", req.agent_role.as_str())])?;
        sections.push(PromptSection {
            id: SectionId::new("identity"),
            priority: priority::IDENTITY,
            content: identity_content,
            volatile: false,
        });
        if !req.tools.is_empty() {
            let tool_list = req.tools.join(", ");
            let tools_content = tools_template().render(&[("tool_list", tool_list.as_str())])?;
            sections.push(PromptSection {
                id: SectionId::new("tools"),
                priority: priority::TOOLS,
                content: tools_content,
                volatile: false,
            });
        }
        for skill in &req.skills {
            for section in &skill.sections {
                sections.push(PromptSection {
                    id: SectionId::new(format!("skill:{}:{}", skill.name, section.id.0)),
                    priority: section.priority,
                    content: section.content.clone(),
                    volatile: section.volatile,
                });
            }
        }
        if let Some(digest) = &req.memory_digest {
            sections.push(PromptSection {
                id: SectionId::new("memory_digest"),
                priority: priority::MEMORY_DIGEST,
                content: digest.clone(),
                volatile: false,
            });
        }
        if let Some(policy) = &req.loop_policy {
            sections.push(PromptSection {
                id: SectionId::new("loop_policy"),
                priority: priority::LOOP_POLICY,
                content: policy.clone(),
                volatile: false,
            });
        }
        sections.push(PromptSection {
            id: SectionId::new("user_input"),
            priority: priority::USER_INPUT,
            content: req.user_input.clone(),
            volatile: true,
        });

        let sections = match req.max_tokens {
            Some(max_tokens) => {
                let budgeter = crate::budgeter::ContextBudgeter::new(crate::budgeter::BudgeterConfig {
                    max_tokens: max_tokens as u64,
                    min_priority_to_keep: 0,
                });
                budgeter.trim_sections(sections, max_tokens)?
            }
            None => sections,
        };

        // `render()` joins section contents with a `"\n\n"` separator, so the
        // cache-friendly prefix of the rendered string must include the
        // separator bytes between non-volatile sections too — otherwise
        // `render()[..cache_prefix_len]` cuts a few bytes short of the true
        // stable prefix, which is what providers actually hash for prompt
        // caching. Only *between* non-volatile sections does a separator
        // belong to the prefix; the separator right before the first volatile
        // section is not part of the stable prefix.
        const SEPARATOR_LEN: u32 = 2; // "\n\n"
        let non_volatile_count = sections.iter().take_while(|s| !s.volatile).count();
        let cache_prefix_len: u32 = sections
            .iter()
            .take(non_volatile_count)
            .map(|s| s.content.len() as u32)
            .sum::<u32>()
            + non_volatile_count.saturating_sub(1) as u32 * SEPARATOR_LEN;
        let token_estimate: u32 = sections.iter().map(|s| (s.content.len() as u32) / 4).sum();

        Ok(AssembledPrompt { sections, token_estimate, cache_prefix_len })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::skills::SkillPack;

    fn base_request() -> PromptRequest {
        PromptRequest {
            session_id: SessionId::new(),
            agent_role: "coding-agent".into(),
            tools: vec!["file.read".into()],
            memory_digest: Some("project uses Rust".into()),
            loop_policy: None,
            user_input: "fix the bug".into(),
            skills: Vec::new(),
            max_tokens: None,
        }
    }

    #[test]
    fn same_request_produces_byte_identical_prompt() {
        let assembler = DefaultPromptAssembler;
        let req = base_request();
        let a = assembler.assemble(&req).unwrap();
        let b = assembler.assemble(&req).unwrap();
        assert_eq!(a.render(), b.render());
    }

    #[test]
    fn user_input_is_last_and_volatile() {
        let assembler = DefaultPromptAssembler;
        let req = base_request();
        let assembled = assembler.assemble(&req).unwrap();
        let last = assembled.sections.last().unwrap();
        assert!(last.volatile);
        assert_eq!(last.id, SectionId::new("user_input"));
    }

    #[test]
    fn cache_prefix_excludes_volatile_user_input() {
        let assembler = DefaultPromptAssembler;
        let req = base_request();
        let assembled = assembler.assemble(&req).unwrap();
        let full_len: u32 = assembled.sections.iter().map(|s| s.content.len() as u32).sum();
        assert!(assembled.cache_prefix_len < full_len);
    }

    #[test]
    fn empty_agent_role_is_hard_error() {
        let assembler = DefaultPromptAssembler;
        let mut req = base_request();
        req.agent_role = "".into();
        assert!(assembler.assemble(&req).is_err());
    }

    #[test]
    fn skill_pack_sections_are_included_in_stable_prefix() {
        let assembler = DefaultPromptAssembler;
        let mut req = base_request();
        req.skills.push(SkillPack {
            name: "review".into(),
            version: "1".into(),
            sections: vec![PromptSection {
                id: SectionId::new("review_policy"),
                priority: 120,
                content: "Review diffs for correctness before style.".into(),
                volatile: false,
            }],
            required_tools: vec![],
        });
        let assembled = assembler.assemble(&req).unwrap();
        let skill_section = assembled
            .sections
            .iter()
            .find(|s| s.id == SectionId::new("skill:review:review_policy"))
            .expect("skill section present");
        assert!(!skill_section.volatile);
        // Must appear before the volatile user_input section.
        let skill_idx = assembled.sections.iter().position(|s| s.id == skill_section.id).unwrap();
        let user_idx = assembled.sections.iter().position(|s| s.id == SectionId::new("user_input")).unwrap();
        assert!(skill_idx < user_idx);
    }

    #[test]
    fn missing_skill_does_not_error_assembly() {
        // Per spec: a caller that failed to resolve a skill pack simply omits
        // it from `req.skills` — assembly proceeds normally, never fails the
        // turn over a missing skill.
        let assembler = DefaultPromptAssembler;
        let req = base_request();
        assert!(req.skills.is_empty());
        assert!(assembler.assemble(&req).is_ok());
    }

    #[test]
    fn same_config_produces_stable_cache_prefix_across_turns() {
        let assembler = DefaultPromptAssembler;
        let mut turn1 = base_request();
        let mut turn2 = base_request();
        turn1.user_input = "first message".into();
        turn2.user_input = "a completely different second message".into();

        let a = assembler.assemble(&turn1).unwrap();
        let b = assembler.assemble(&turn2).unwrap();
        assert_eq!(a.cache_prefix_len, b.cache_prefix_len);

        let prefix_a = &a.render()[..a.cache_prefix_len as usize];
        let prefix_b = &b.render()[..b.cache_prefix_len as usize];
        assert_eq!(prefix_a, prefix_b);
    }

    #[test]
    fn max_tokens_trims_low_priority_sections_first() {
        let assembler = DefaultPromptAssembler;
        let mut req = base_request();
        req.memory_digest = Some("x".repeat(4000)); // ~1000 tokens, low priority
        req.max_tokens = Some(20);
        let assembled = assembler.assemble(&req).unwrap();
        assert!(assembled.sections.iter().all(|s| s.id != SectionId::new("memory_digest")));
        // identity (highest priority) always survives.
        assert!(assembled.sections.iter().any(|s| s.id == SectionId::new("identity")));
        assert!(assembled.token_estimate <= 20);
    }

    #[test]
    fn max_tokens_below_identity_alone_is_hard_error() {
        let assembler = DefaultPromptAssembler;
        let mut req = base_request();
        req.agent_role = "x".repeat(4000);
        req.max_tokens = Some(1);
        assert!(assembler.assemble(&req).is_err());
    }
}
