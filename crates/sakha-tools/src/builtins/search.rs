//! Built-in `search.ripgrep` tool for fast in-workspace text search.
//! Implemented with `walkdir` + `regex` (no dependency on an actual
//! `ripgrep` binary), honoring simple ignore rules (`.git`, common build
//! directories) per `modules/09-terminal-process-automation.md` style
//! ignore behavior.

use async_trait::async_trait;
use regex::Regex;
use sakha_core::{SakhaError, SakhaResult};
use sakha_security::PermissionKind;

use crate::fs_util::resolve_in_workspace;
use crate::schema::validate_against_schema;
use crate::tool::{
    IdempotencyPolicy, Tool, ToolContext, ToolInputSchema, ToolName, ToolOutputSchema,
    ToolPermissionSpec, ToolPlan, ToolResult, ToolSpec, ToolSummary, ValidatedInput,
};

const MODULE: &str = "sakha-tools::search";

/// Directory names always excluded from search, regardless of `.gitignore`.
const DEFAULT_IGNORED_DIRS: &[&str] = &[".git", "node_modules", "target", ".venv", "dist", "build"];

/// Maximum number of matches returned, to keep output bounded.
const MAX_MATCHES: usize = 500;

#[derive(Debug, Default)]
pub struct SearchRipgrepTool;

#[async_trait]
impl Tool for SearchRipgrepTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("search.ripgrep"),
            description: "Search workspace files for a regex pattern".to_string(),
            input_schema: ToolInputSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string"},
                    "path": {"type": "string"},
                    "case_insensitive": {"type": "boolean"},
                    "max_matches": {"type": "integer"}
                },
                "required": ["pattern"]
            })),
            output_schema: ToolOutputSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "matches": {"type": "array"},
                    "truncated": {"type": "boolean"}
                }
            })),
            permission_spec: ToolPermissionSpec {
                required: vec![PermissionKind::FileRead],
            },
            idempotency_policy: IdempotencyPolicy::Idempotent,
        }
    }

    fn validate(&self, input: serde_json::Value) -> SakhaResult<ValidatedInput> {
        validate_against_schema(MODULE, &input, &self.spec().input_schema.0)?;
        let pattern = input.get("pattern").and_then(|p| p.as_str()).unwrap_or("");
        if pattern.is_empty() {
            return Err(SakhaError::invalid_input(MODULE, "pattern must not be empty"));
        }
        Regex::new(pattern)
            .map_err(|e| SakhaError::invalid_input(MODULE, format!("invalid regex pattern: {e}")))?;
        Ok(ValidatedInput(input))
    }

    async fn plan(&self, input: &ValidatedInput, _context: &ToolContext) -> SakhaResult<ToolPlan> {
        let pattern = input.0.get("pattern").and_then(|p| p.as_str()).unwrap_or_default();
        Ok(ToolPlan {
            summary: format!("search for /{pattern}/"),
            affected_paths: Vec::new(),
            is_destructive: false,
        })
    }

    async fn execute(&self, input: &ValidatedInput, context: &ToolContext) -> SakhaResult<ToolResult> {
        let pattern_str = input
            .0
            .get("pattern")
            .and_then(|p| p.as_str())
            .ok_or_else(|| SakhaError::invalid_input(MODULE, "missing pattern"))?;
        let case_insensitive = input.0.get("case_insensitive").and_then(|c| c.as_bool()).unwrap_or(false);
        let max_matches = input
            .0
            .get("max_matches")
            .and_then(|m| m.as_u64())
            .map(|m| m as usize)
            .unwrap_or(MAX_MATCHES)
            .min(MAX_MATCHES);

        let regex = regex::RegexBuilder::new(pattern_str)
            .case_insensitive(case_insensitive)
            .build()
            .map_err(|e| SakhaError::invalid_input(MODULE, format!("invalid regex pattern: {e}")))?;

        let search_root = match input.0.get("path").and_then(|p| p.as_str()) {
            Some(path) => resolve_in_workspace(&context.workspace_root, path)?,
            None => context.workspace_root.clone(),
        };

        let root = search_root.clone();
        let matches = tokio::task::spawn_blocking(move || search_directory(&root, &regex, max_matches))
            .await
            .map_err(|e| SakhaError::invalid_input(MODULE, format!("search task failed: {e}")))??;

        let truncated = matches.len() >= max_matches;
        let matches_json: Vec<serde_json::Value> = matches
            .into_iter()
            .map(|m| {
                serde_json::json!({
                    "path": m.path,
                    "line_number": m.line_number,
                    "line": m.line,
                })
            })
            .collect();

        Ok(ToolResult::success(serde_json::json!({
            "matches": matches_json,
            "truncated": truncated,
        })))
    }

    fn summarize(&self, result: &ToolResult) -> ToolSummary {
        let count = result
            .output_json
            .get("matches")
            .and_then(|m| m.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        ToolSummary {
            text: format!("{count} match(es)"),
            truncated: result
                .output_json
                .get("truncated")
                .and_then(|t| t.as_bool())
                .unwrap_or(false),
        }
    }
}

struct SearchMatch {
    path: String,
    line_number: usize,
    line: String,
}

/// Walks `root`, skipping known-noisy directories, and collects up to
/// `max_matches` regex matches. Runs on a blocking thread since `walkdir` is
/// synchronous.
fn search_directory(root: &std::path::Path, regex: &Regex, max_matches: usize) -> SakhaResult<Vec<SearchMatch>> {
    let mut matches = Vec::new();

    let walker = walkdir::WalkDir::new(root).into_iter().filter_entry(|entry| {
        if entry.file_type().is_dir() {
            let name = entry.file_name().to_string_lossy();
            return !DEFAULT_IGNORED_DIRS.contains(&name.as_ref());
        }
        true
    });

    for entry in walker {
        if matches.len() >= max_matches {
            break;
        }
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }

        // Read as raw bytes and lossily convert to UTF-8 rather than using
        // `read_to_string` directly: a file that is mostly-text but contains
        // a handful of invalid UTF-8 byte sequences (common with generated
        // files, logs, or mixed-encoding sources) would otherwise be skipped
        // entirely instead of having its still-valid lines searched.
        // Genuinely unreadable/binary files are still skipped on I/O error.
        let bytes = match std::fs::read(entry.path()) {
            Ok(b) => b,
            Err(_) => continue, // skip unreadable files (permissions, etc.)
        };
        let content = String::from_utf8_lossy(&bytes).into_owned();

        let display_path = entry.path().to_string_lossy().to_string();
        for (idx, line) in content.lines().enumerate() {
            if regex.is_match(line) {
                matches.push(SearchMatch {
                    path: display_path.clone(),
                    line_number: idx + 1,
                    line: line.to_string(),
                });
                if matches.len() >= max_matches {
                    break;
                }
            }
        }
    }

    Ok(matches)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn finds_matches_in_workspace_files() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("a.txt"), "hello world\nfoo bar\n").await.unwrap();
        tokio::fs::write(dir.path().join("b.txt"), "no match here\n").await.unwrap();

        let context = ToolContext::new(dir.path());
        let tool = SearchRipgrepTool;
        let input = tool.validate(serde_json::json!({"pattern": "hello"})).unwrap();
        let result = tool.execute(&input, &context).await.unwrap();
        let matches = result.output_json["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["line_number"], 1);
    }

    #[tokio::test]
    async fn ignores_git_directory() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::create_dir_all(dir.path().join(".git")).await.unwrap();
        tokio::fs::write(dir.path().join(".git").join("config"), "hello secret").await.unwrap();
        tokio::fs::write(dir.path().join("a.txt"), "hello visible").await.unwrap();

        let context = ToolContext::new(dir.path());
        let tool = SearchRipgrepTool;
        let input = tool.validate(serde_json::json!({"pattern": "hello"})).unwrap();
        let result = tool.execute(&input, &context).await.unwrap();
        let matches = result.output_json["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert!(matches[0]["path"].as_str().unwrap().contains("a.txt"));
    }

    #[test]
    fn validate_rejects_invalid_regex() {
        let tool = SearchRipgrepTool;
        let result = tool.validate(serde_json::json!({"pattern": "("}));
        assert!(result.is_err());
    }

    #[test]
    fn validate_rejects_empty_pattern() {
        let tool = SearchRipgrepTool;
        let result = tool.validate(serde_json::json!({"pattern": ""}));
        assert!(result.is_err());
    }
}
