//! Minimal glob matcher used for policy rule and command matching.
//!
//! Supports `*` (any run of characters, including none) and `?` (any single
//! character). No brace expansion, no character classes — kept intentionally
//! small since the only workspace-approved regex dependency is `regex`, and a
//! full glob crate is not in the pre-declared dependency set.

use regex::Regex;

/// Converts a glob pattern into an anchored regex pattern string.
fn glob_to_regex_pattern(glob: &str) -> String {
    let mut out = String::with_capacity(glob.len() * 2 + 2);
    out.push('^');
    for c in glob.chars() {
        match c {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            c if regex_syntax::is_meta_character(c) => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push('$');
    out
}

/// A minimal stand-in for `regex_syntax::is_meta_character` so we don't need
/// to pull in the `regex-syntax` crate directly (it is a transitive
/// dependency of `regex` but not part of the workspace-declared dependency
/// set to depend on directly).
mod regex_syntax {
    pub fn is_meta_character(c: char) -> bool {
        matches!(
            c,
            '\\' | '.' | '+' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$' | '*' | '?'
        )
    }
}

/// Returns true if `text` matches the glob `pattern`.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') && !pattern.contains('?') {
        return pattern == text;
    }
    match Regex::new(&glob_to_regex_pattern(pattern)) {
        Ok(re) => re.is_match(text),
        Err(_) => pattern == text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_literal() {
        assert!(glob_match("file.read", "file.read"));
        assert!(!glob_match("file.read", "file.write"));
    }

    #[test]
    fn matches_wildcard_suffix() {
        assert!(glob_match("file.*", "file.read"));
        assert!(glob_match("file.*", "file.write"));
        assert!(!glob_match("file.*", "shell.run_safe"));
    }

    #[test]
    fn matches_command_glob() {
        assert!(glob_match("shell.run_arbitrary:rm -rf *", "shell.run_arbitrary:rm -rf /tmp/x"));
        assert!(!glob_match("shell.run_arbitrary:rm -rf *", "shell.run_arbitrary:ls -la"));
    }

    #[test]
    fn star_alone_matches_everything() {
        assert!(glob_match("*", "anything.at.all"));
    }
}
