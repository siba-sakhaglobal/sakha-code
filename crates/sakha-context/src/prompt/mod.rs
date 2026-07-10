//! Prompt assembly submodule: see `modules/21-prompt-system.md`.

pub mod assembler;
pub mod skills;
pub mod templates;

pub use assembler::{AssembledPrompt, DefaultPromptAssembler, PromptAssembler, PromptRequest, PromptSection, SectionId};
pub use skills::{SkillPack, SkillPackLoadWarning, SkillPackLoader, SkillPackOrigin};
pub use templates::PromptTemplate;
