//! Prompt templates with typed placeholders. No ad-hoc `format!` at call
//! sites per spec `modules/21-prompt-system.md`.

use serde::{Deserialize, Serialize};

use sakha_core::{SakhaError, SakhaResult};

/// A template string with `{{placeholder}}`-style slots, resolved via
/// `PromptTemplate::render`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplate {
    pub name: String,
    pub source: String,
}

impl PromptTemplate {
    pub fn new(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self { name: name.into(), source: source.into() }
    }

    /// Substitutes every `{{key}}` occurrence with its value from `values`.
    /// Returns a hard error if any placeholder remains unresolved, per spec
    /// "Template placeholder unresolved -> hard error at assembly time".
    pub fn render(&self, values: &[(&str, &str)]) -> SakhaResult<String> {
        let mut out = self.source.clone();
        for (key, value) in values {
            out = out.replace(&format!("{{{{{key}}}}}"), value);
        }
        if out.contains("{{") && out.contains("}}") {
            return Err(SakhaError::invalid_input(
                "sakha-context",
                format!("unresolved placeholder in template '{}'", self.name),
            ));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_substitutes_all_placeholders() {
        let template = PromptTemplate::new("greeting", "Hello, {{name}}!");
        let rendered = template.render(&[("name", "Sakha")]).unwrap();
        assert_eq!(rendered, "Hello, Sakha!");
    }

    #[test]
    fn render_errors_on_unresolved_placeholder() {
        let template = PromptTemplate::new("greeting", "Hello, {{name}}!");
        let result = template.render(&[]);
        assert!(result.is_err());
    }
}
