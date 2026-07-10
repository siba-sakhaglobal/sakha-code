//! Built-in git tools: `git.status`, `git.diff`, `git.apply_patch`,
//! `git.branch`, `git.worktree`, `git.commit`. Shells out to the `git`
//! binary as a subprocess. See `modules/08-files-git-worktrees.md`.

use async_trait::async_trait;
use sakha_core::{SakhaError, SakhaResult};
use sakha_security::PermissionKind;
use tokio::process::Command;

use crate::schema::validate_against_schema;
use crate::tool::{
    IdempotencyPolicy, Tool, ToolContext, ToolInputSchema, ToolName, ToolOutputSchema,
    ToolPermissionSpec, ToolPlan, ToolResult, ToolSpec, ToolSummary, ValidatedInput,
};

const MODULE: &str = "sakha-tools::git";

fn default_summary(result: &ToolResult) -> ToolSummary {
    let text = match &result.error_message {
        Some(msg) => msg.clone(),
        None => result.output_json.to_string(),
    };
    ToolSummary {
        text,
        truncated: false,
    }
}

/// Runs `git <args>` in `cwd`, returning normalized stdout/stderr/exit code.
/// Never panics: subprocess spawn failures become `SakhaError::invalid_input`.
async fn run_git(cwd: &std::path::Path, args: &[&str]) -> SakhaResult<(i32, String, String)> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .await
        .map_err(|e| SakhaError::invalid_input(MODULE, format!("failed to run git: {e}")))?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Ok((exit_code, stdout, stderr))
}

fn result_from_git(exit_code: i32, stdout: String, stderr: String) -> ToolResult {
    if exit_code == 0 {
        ToolResult::success(serde_json::json!({
            "exit_code": exit_code,
            "stdout": stdout,
            "stderr": stderr,
        }))
    } else {
        ToolResult {
            status: crate::tool::ToolCallStatus::Failed,
            output_json: serde_json::json!({
                "exit_code": exit_code,
                "stdout": stdout,
                "stderr": stderr,
            }),
            raw_output_ref: None,
            error_message: Some(format!("git exited with code {exit_code}: {stderr}")),
        }
    }
}

macro_rules! simple_git_tool {
    ($struct_name:ident, $tool_name:literal, $description:literal, $permission:expr, $idempotency:expr, $build_args:expr) => {
        #[derive(Debug, Default)]
        pub struct $struct_name;

        #[async_trait]
        impl Tool for $struct_name {
            fn spec(&self) -> ToolSpec {
                ToolSpec {
                    name: ToolName::new($tool_name),
                    description: $description.to_string(),
                    input_schema: ToolInputSchema(serde_json::json!({"type": "object"})),
                    output_schema: ToolOutputSchema(serde_json::json!({
                        "type": "object",
                        "properties": {
                            "exit_code": {"type": "integer"},
                            "stdout": {"type": "string"},
                            "stderr": {"type": "string"}
                        }
                    })),
                    permission_spec: ToolPermissionSpec {
                        required: vec![$permission],
                    },
                    idempotency_policy: $idempotency,
                }
            }

            fn validate(&self, input: serde_json::Value) -> SakhaResult<ValidatedInput> {
                validate_against_schema(MODULE, &input, &self.spec().input_schema.0)?;
                Ok(ValidatedInput(input))
            }

            async fn plan(&self, _input: &ValidatedInput, _context: &ToolContext) -> SakhaResult<ToolPlan> {
                Ok(ToolPlan {
                    summary: $tool_name.to_string(),
                    affected_paths: Vec::new(),
                    is_destructive: false,
                })
            }

            async fn execute(&self, input: &ValidatedInput, context: &ToolContext) -> SakhaResult<ToolResult> {
                let build_args: fn(&serde_json::Value) -> SakhaResult<Vec<String>> = $build_args;
                let args = build_args(&input.0)?;
                let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                let (exit_code, stdout, stderr) = run_git(&context.workspace_root, &args_ref).await?;
                Ok(result_from_git(exit_code, stdout, stderr))
            }

            fn summarize(&self, result: &ToolResult) -> ToolSummary {
                default_summary(result)
            }
        }
    };
}

simple_git_tool!(
    GitStatusTool,
    "git.status",
    "Show git working tree status",
    PermissionKind::FileRead,
    IdempotencyPolicy::Idempotent,
    |_input| Ok(vec!["status".to_string(), "--porcelain=v1".to_string(), "--branch".to_string()])
);

simple_git_tool!(
    GitDiffTool,
    "git.diff",
    "Show git diff for the working tree or a specific path",
    PermissionKind::FileRead,
    IdempotencyPolicy::Idempotent,
    |input: &serde_json::Value| {
        let mut args = vec!["diff".to_string()];
        if let Some(staged) = input.get("staged").and_then(|v| v.as_bool()) {
            if staged {
                args.push("--staged".to_string());
            }
        }
        if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
            args.push("--".to_string());
            args.push(path.to_string());
        }
        Ok(args)
    }
);

/// Applies a unified diff via `git apply`. Writes the diff to a temp file
/// first (rather than piping through stdin) so `git apply` error messages
/// reference a real path, which is easier to debug.
#[derive(Debug, Default)]
pub struct GitApplyPatchTool;

#[async_trait]
impl Tool for GitApplyPatchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("git.apply_patch"),
            description: "Apply a unified diff patch via `git apply`".to_string(),
            input_schema: ToolInputSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "diff": {"type": "string"},
                    "check_only": {"type": "boolean"}
                },
                "required": ["diff"]
            })),
            output_schema: ToolOutputSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "exit_code": {"type": "integer"},
                    "stdout": {"type": "string"},
                    "stderr": {"type": "string"}
                }
            })),
            permission_spec: ToolPermissionSpec {
                required: vec![PermissionKind::FileWrite],
            },
            idempotency_policy: IdempotencyPolicy::NotIdempotent,
        }
    }

    fn validate(&self, input: serde_json::Value) -> SakhaResult<ValidatedInput> {
        validate_against_schema(MODULE, &input, &self.spec().input_schema.0)?;
        Ok(ValidatedInput(input))
    }

    async fn plan(&self, _input: &ValidatedInput, _context: &ToolContext) -> SakhaResult<ToolPlan> {
        Ok(ToolPlan {
            summary: "git apply_patch".to_string(),
            affected_paths: Vec::new(),
            is_destructive: true,
        })
    }

    async fn execute(&self, input: &ValidatedInput, context: &ToolContext) -> SakhaResult<ToolResult> {
        let diff = input
            .0
            .get("diff")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SakhaError::invalid_input(MODULE, "missing diff"))?;
        let check_only = input.0.get("check_only").and_then(|v| v.as_bool()).unwrap_or(false);

        let unique = format!(
            "sakha-patch-{}-{}.diff",
            std::process::id(),
            sakha_core::ToolCallId::new()
        );
        let patch_path = std::env::temp_dir().join(unique);
        tokio::fs::write(&patch_path, diff)
            .await
            .map_err(|e| SakhaError::invalid_input(MODULE, format!("failed to write patch file: {e}")))?;

        let patch_path_str = patch_path.to_string_lossy().to_string();
        let mut args = vec!["apply".to_string()];
        if check_only {
            args.push("--check".to_string());
        }
        args.push(patch_path_str);

        let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let (exit_code, stdout, stderr) = run_git(&context.workspace_root, &args_ref).await?;
        let _ = tokio::fs::remove_file(&patch_path).await;
        Ok(result_from_git(exit_code, stdout, stderr))
    }

    fn summarize(&self, result: &ToolResult) -> ToolSummary {
        default_summary(result)
    }
}

simple_git_tool!(
    GitBranchTool,
    "git.branch",
    "Create or list git branches",
    PermissionKind::FileWrite,
    IdempotencyPolicy::NotIdempotent,
    |input: &serde_json::Value| {
        let mut args = vec!["branch".to_string()];
        if let Some(name) = input.get("name").and_then(|v| v.as_str()) {
            args.push(name.to_string());
        }
        Ok(args)
    }
);

simple_git_tool!(
    GitWorktreeTool,
    "git.worktree",
    "Manage git worktrees (add/remove/list)",
    PermissionKind::FileWrite,
    IdempotencyPolicy::NotIdempotent,
    |input: &serde_json::Value| {
        let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("list");
        let mut args = vec!["worktree".to_string(), action.to_string()];
        match action {
            "add" => {
                let path = input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SakhaError::invalid_input(MODULE, "worktree add requires 'path'"))?;
                args.push(path.to_string());
                if let Some(branch) = input.get("branch").and_then(|v| v.as_str()) {
                    args.push(branch.to_string());
                }
            }
            "remove" => {
                let path = input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SakhaError::invalid_input(MODULE, "worktree remove requires 'path'"))?;
                if input.get("force").and_then(|v| v.as_bool()).unwrap_or(false) {
                    args.push("--force".to_string());
                }
                args.push(path.to_string());
            }
            _ => {}
        }
        Ok(args)
    }
);

simple_git_tool!(
    GitCommitTool,
    "git.commit",
    "Create a git commit with a message",
    PermissionKind::FileWrite,
    IdempotencyPolicy::NotIdempotent,
    |input: &serde_json::Value| {
        let message = input
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SakhaError::invalid_input(MODULE, "missing commit message"))?;
        let mut args = vec!["commit".to_string(), "-m".to_string(), message.to_string()];
        if input.get("all").and_then(|v| v.as_bool()).unwrap_or(false) {
            args.push("-a".to_string());
        }
        Ok(args)
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    async fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        run_git(dir.path(), &["init", "-q"]).await.unwrap();
        run_git(dir.path(), &["config", "user.email", "test@example.com"]).await.unwrap();
        run_git(dir.path(), &["config", "user.name", "Test User"]).await.unwrap();
        dir
    }

    #[tokio::test]
    async fn git_status_reports_clean_repo() {
        let dir = init_repo().await;
        let context = ToolContext::new(dir.path());
        let tool = GitStatusTool;
        let input = tool.validate(serde_json::json!({})).unwrap();
        let result = tool.execute(&input, &context).await.unwrap();
        assert!(matches!(result.status, crate::tool::ToolCallStatus::Succeeded));
    }

    #[tokio::test]
    async fn git_status_reports_untracked_file() {
        let dir = init_repo().await;
        tokio::fs::write(dir.path().join("a.txt"), "hello").await.unwrap();
        let context = ToolContext::new(dir.path());
        let tool = GitStatusTool;
        let input = tool.validate(serde_json::json!({})).unwrap();
        let result = tool.execute(&input, &context).await.unwrap();
        let stdout = result.output_json["stdout"].as_str().unwrap();
        assert!(stdout.contains("a.txt"));
    }

    #[tokio::test]
    async fn git_commit_creates_commit() {
        let dir = init_repo().await;
        tokio::fs::write(dir.path().join("a.txt"), "hello").await.unwrap();
        run_git(dir.path(), &["add", "."]).await.unwrap();

        let context = ToolContext::new(dir.path());
        let tool = GitCommitTool;
        let input = tool.validate(serde_json::json!({"message": "initial commit"})).unwrap();
        let result = tool.execute(&input, &context).await.unwrap();
        assert!(matches!(result.status, crate::tool::ToolCallStatus::Succeeded));
    }

    #[tokio::test]
    async fn git_branch_creates_branch() {
        let dir = init_repo().await;
        tokio::fs::write(dir.path().join("a.txt"), "hello").await.unwrap();
        run_git(dir.path(), &["add", "."]).await.unwrap();
        run_git(dir.path(), &["commit", "-q", "-m", "init"]).await.unwrap();

        let context = ToolContext::new(dir.path());
        let tool = GitBranchTool;
        let input = tool.validate(serde_json::json!({"name": "feature/x"})).unwrap();
        let result = tool.execute(&input, &context).await.unwrap();
        assert!(matches!(result.status, crate::tool::ToolCallStatus::Succeeded));

        let (_, stdout, _) = run_git(dir.path(), &["branch"]).await.unwrap();
        assert!(stdout.contains("feature/x"));
    }

    #[tokio::test]
    async fn git_apply_patch_applies_diff() {
        let dir = init_repo().await;
        tokio::fs::write(dir.path().join("a.txt"), "line1\nline2\nline3\n").await.unwrap();
        run_git(dir.path(), &["add", "."]).await.unwrap();
        run_git(dir.path(), &["commit", "-q", "-m", "init"]).await.unwrap();

        let diff = "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1,3 +1,3 @@\n line1\n-line2\n+line2-changed\n line3\n";

        let context = ToolContext::new(dir.path());
        let tool = GitApplyPatchTool;
        let input = tool.validate(serde_json::json!({"diff": diff})).unwrap();
        let result = tool.execute(&input, &context).await.unwrap();
        assert!(matches!(result.status, crate::tool::ToolCallStatus::Succeeded));

        let content = tokio::fs::read_to_string(dir.path().join("a.txt")).await.unwrap();
        assert!(content.contains("line2-changed"));
    }

    #[tokio::test]
    async fn git_diff_reports_no_changes_on_clean_repo() {
        let dir = init_repo().await;
        tokio::fs::write(dir.path().join("a.txt"), "hello").await.unwrap();
        run_git(dir.path(), &["add", "."]).await.unwrap();
        run_git(dir.path(), &["commit", "-q", "-m", "init"]).await.unwrap();

        let context = ToolContext::new(dir.path());
        let tool = GitDiffTool;
        let input = tool.validate(serde_json::json!({})).unwrap();
        let result = tool.execute(&input, &context).await.unwrap();
        assert!(matches!(result.status, crate::tool::ToolCallStatus::Succeeded));
        assert_eq!(result.output_json["stdout"], "");
    }
}
