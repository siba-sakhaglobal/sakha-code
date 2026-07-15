# Module 21: Prompt System

## Purpose

Own how every prompt is assembled: system prompt, tool specs, memory, skills, and per-loop policy text. No other module should concatenate prompt strings directly.

## Responsibilities

- Deterministic system-prompt assembly from ordered sections (identity, safety, workspace facts, tool usage rules, loop policy, memory digest).
- Prompt templates with typed placeholders (no ad-hoc `format!` in call sites).
- Skill packs: named, versioned prompt fragments loadable from `~/.sakha/skills/` and workspace `.sakha/skills/`.
- Cache-friendly layout: stable prefix first, volatile context last, so provider prompt caching works.
- Token estimation per section so the context planner (module 06) can trim by priority.
- Prompt audit: every assembled prompt is recorded as an artifact ref (compressed via module 05).

## Interfaces

```rust
trait PromptAssembler {
    fn assemble(&self, req: &PromptRequest) -> Result<AssembledPrompt, SakhaError>;
}

struct PromptRequest { session, agent_role, tools, memory_digest, loop_policy, user_input }
struct AssembledPrompt { sections: Vec<PromptSection>, token_estimate: u32, cache_prefix_len: u32 }
struct PromptSection { id: SectionId, priority: u8, content: String, volatile: bool }
struct SkillPack { name, version, sections, required_tools }
```

## Failure Modes

- Missing skill pack → warn and continue without it; never fail the turn.
- Token estimate over budget → return `PromptOverBudget` so the planner trims, never silently truncate mid-section.
- Template placeholder unresolved → hard error at assembly time (catch in tests, not production).

## Test Requirements

- Golden tests: same `PromptRequest` → byte-identical prompt.
- Section ordering and priority trimming.
- Skill pack load/override precedence (workspace overrides user overrides builtin).
- Cache prefix stability across turns with unchanged config.

## Crate Mapping

Lives in `sakha-context` (see crate breakdown) as `prompt/` submodule.
