//! Log/output redaction: strips known secret patterns before they reach logs,
//! model context, or audit exports.

use regex::Regex;

/// Redacts known secret-shaped substrings (API keys, bearer tokens, private
/// keys, etc.) from arbitrary text. Best-effort pattern matching, not a
/// cryptographic guarantee — intended as defense in depth alongside the
/// `SecretStore` handle-not-value discipline.
pub struct Redactor {
    patterns: Vec<Regex>,
}

/// Default set of secret-shaped patterns. Kept as a function (not just inline
/// in `Default`) so `Redactor::with_patterns` callers can start from this set
/// and extend it.
fn default_patterns() -> Vec<&'static str> {
    vec![
        // OpenAI-style API keys.
        r"sk-[A-Za-z0-9]{20,}",
        // Generic bearer tokens.
        r"(?i)bearer\s+[A-Za-z0-9\-_.]{10,}",
        // AWS access key IDs.
        r"AKIA[0-9A-Z]{16}",
        // AWS secret access keys (heuristic: 40-char base64-ish after a hint word).
        r#"(?i)(aws_secret_access_key|secret_access_key)\s*[:=]\s*['"]?[A-Za-z0-9/+=]{40}['"]?"#,
        // GitHub personal access tokens / fine-grained tokens.
        r"gh[pousr]_[A-Za-z0-9]{20,}",
        // Slack tokens.
        r"xox[baprs]-[A-Za-z0-9-]{10,}",
        // Generic `key`/`token`/`secret`/`password` assignments with a quoted value.
        r#"(?i)(api[_-]?key|access[_-]?token|secret|password)\s*[:=]\s*['"][^'"\s]{8,}['"]"#,
        // PEM private key blocks.
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
        // JWTs.
        r"eyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}",
    ]
}

impl Default for Redactor {
    fn default() -> Self {
        Self::with_patterns(default_patterns())
    }
}

impl Redactor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a redactor from an explicit set of regex source patterns.
    /// Patterns that fail to compile are silently skipped rather than
    /// panicking, since this may be constructed from user-supplied policy
    /// config.
    pub fn with_patterns<I, S>(patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let compiled = patterns
            .into_iter()
            .filter_map(|p| Regex::new(p.as_ref()).ok())
            .collect();
        Self { patterns: compiled }
    }

    /// Adds an additional pattern (e.g. a project-specific secret shape) on
    /// top of the defaults. Invalid regex is silently ignored.
    pub fn with_extra_pattern(mut self, pattern: &str) -> Self {
        if let Ok(re) = Regex::new(pattern) {
            self.patterns.push(re);
        }
        self
    }

    /// Returns a copy of `text` with any matched secret patterns replaced by
    /// `[REDACTED]`.
    pub fn redact(&self, text: &str) -> String {
        let mut out = text.to_string();
        for pattern in &self.patterns {
            out = pattern.replace_all(&out, "[REDACTED]").into_owned();
        }
        out
    }

    /// Returns true if `text` contains anything that looks like a secret.
    pub fn contains_secret(&self, text: &str) -> bool {
        self.patterns.iter().any(|p| p.is_match(text))
    }

    /// Redacts a specific known secret value (exact match, not pattern-based)
    /// wherever it appears in `text`. Useful when a `SecretStore` value is
    /// known and must be scrubbed from tool output even if it doesn't match
    /// any generic shape.
    pub fn redact_known_value(&self, text: &str, secret_value: &str) -> String {
        if secret_value.is_empty() {
            return text.to_string();
        }
        text.replace(secret_value, "[REDACTED]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_known_api_key_shape() {
        let redactor = Redactor::new();
        let text = "key=sk-abcdefghijklmnopqrstuvwxyz";
        assert!(redactor.contains_secret(text));
        assert!(!redactor.redact(text).contains("sk-abcdefghijklmnopqrstuvwxyz"));
    }

    #[test]
    fn redacts_bearer_token() {
        let redactor = Redactor::new();
        let text = "Authorization: Bearer abc123DEF456ghi789";
        let redacted = redactor.redact(text);
        assert!(!redacted.contains("abc123DEF456ghi789"));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_aws_access_key() {
        let redactor = Redactor::new();
        let text = "AWS_KEY=AKIAABCDEFGHIJKLMNOP";
        assert!(redactor.contains_secret(text));
    }

    #[test]
    fn leaves_non_secret_text_untouched() {
        let redactor = Redactor::new();
        let text = "the quick brown fox jumps over the lazy dog";
        assert_eq!(redactor.redact(text), text);
        assert!(!redactor.contains_secret(text));
    }

    #[test]
    fn redacts_known_exact_value_even_without_shape_match() {
        let redactor = Redactor::new();
        let text = "the password is hunter2forsure";
        let redacted = redactor.redact_known_value(text, "hunter2forsure");
        assert!(!redacted.contains("hunter2forsure"));
    }
}
