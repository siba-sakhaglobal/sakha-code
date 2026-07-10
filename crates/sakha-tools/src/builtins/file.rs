//! Built-in file tools: `file.read`, `file.write`, `file.patch`,
//! `file.delete`, `file.move`, `file.list`.
//!
//! See `modules/08-files-git-worktrees.md`: workspace-relative path
//! resolution with traversal prevention, backup snapshot on write, unified
//! diff patch application with conflict detection.

use async_trait::async_trait;
use sakha_core::{SakhaError, SakhaResult};
use sakha_security::PermissionKind;

use crate::fs_util::resolve_in_workspace;
use crate::schema::validate_against_schema;
use crate::tool::{
    IdempotencyPolicy, Tool, ToolContext, ToolInputSchema, ToolName, ToolOutputSchema,
    ToolPermissionSpec, ToolPlan, ToolResult, ToolSpec, ToolSummary, ValidatedInput,
};

const MODULE: &str = "sakha-tools::file";

/// Maximum bytes returned inline by `file.read` before truncation.
const MAX_READ_BYTES: usize = 512 * 1024;

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

// ---------------------------------------------------------------------
// file.read
// ---------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct FileReadTool;

#[async_trait]
impl Tool for FileReadTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("file.read"),
            description: "Read a file's contents from the workspace".to_string(),
            input_schema: ToolInputSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "offset": {"type": "integer"},
                    "limit": {"type": "integer"}
                },
                "required": ["path"]
            })),
            output_schema: ToolOutputSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "content": {"type": "string"},
                    "truncated": {"type": "boolean"},
                    "byte_len": {"type": "integer"}
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
        Ok(ValidatedInput(input))
    }

    async fn plan(&self, input: &ValidatedInput, _context: &ToolContext) -> SakhaResult<ToolPlan> {
        let path = input.0.get("path").and_then(|p| p.as_str()).unwrap_or_default();
        Ok(ToolPlan {
            summary: format!("read {path}"),
            affected_paths: vec![path.to_string()],
            is_destructive: false,
        })
    }

    async fn execute(&self, input: &ValidatedInput, context: &ToolContext) -> SakhaResult<ToolResult> {
        let path_str = input
            .0
            .get("path")
            .and_then(|p| p.as_str())
            .ok_or_else(|| SakhaError::invalid_input(MODULE, "missing path"))?;
        let resolved = resolve_in_workspace(&context.workspace_root, path_str)?;

        let bytes = tokio::fs::read(&resolved)
            .await
            .map_err(|e| SakhaError::invalid_input(MODULE, format!("read failed: {e}")))?;

        let byte_len = bytes.len();
        let truncated = byte_len > MAX_READ_BYTES;
        let slice = if truncated { &bytes[..MAX_READ_BYTES] } else { &bytes[..] };
        let content = String::from_utf8_lossy(slice).to_string();

        Ok(ToolResult::success(serde_json::json!({
            "content": content,
            "truncated": truncated,
            "byte_len": byte_len,
        })))
    }

    fn summarize(&self, result: &ToolResult) -> ToolSummary {
        default_summary(result)
    }
}

// ---------------------------------------------------------------------
// file.write
// ---------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct FileWriteTool;

#[async_trait]
impl Tool for FileWriteTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("file.write"),
            description: "Write (create or overwrite) a file in the workspace".to_string(),
            input_schema: ToolInputSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "content": {"type": "string"}
                },
                "required": ["path", "content"]
            })),
            output_schema: ToolOutputSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "bytes_written": {"type": "integer"},
                    "backup_created": {"type": "boolean"}
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

    async fn plan(&self, input: &ValidatedInput, _context: &ToolContext) -> SakhaResult<ToolPlan> {
        let path = input.0.get("path").and_then(|p| p.as_str()).unwrap_or_default();
        Ok(ToolPlan {
            summary: format!("write {path}"),
            affected_paths: vec![path.to_string()],
            is_destructive: true,
        })
    }

    async fn execute(&self, input: &ValidatedInput, context: &ToolContext) -> SakhaResult<ToolResult> {
        let path_str = input
            .0
            .get("path")
            .and_then(|p| p.as_str())
            .ok_or_else(|| SakhaError::invalid_input(MODULE, "missing path"))?;
        let content = input
            .0
            .get("content")
            .and_then(|c| c.as_str())
            .ok_or_else(|| SakhaError::invalid_input(MODULE, "missing content"))?;

        let resolved = resolve_in_workspace(&context.workspace_root, path_str)?;

        let mut backup_created = false;
        if resolved.exists() {
            let backup_path = backup_path_for(&resolved);
            if let Ok(existing) = tokio::fs::read(&resolved).await {
                if tokio::fs::write(&backup_path, existing).await.is_ok() {
                    backup_created = true;
                }
            }
        }

        if let Some(parent) = resolved.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| SakhaError::invalid_input(MODULE, format!("create_dir_all failed: {e}")))?;
        }

        tokio::fs::write(&resolved, content.as_bytes())
            .await
            .map_err(|e| SakhaError::invalid_input(MODULE, format!("write failed: {e}")))?;

        Ok(ToolResult::success(serde_json::json!({
            "bytes_written": content.len(),
            "backup_created": backup_created,
        })))
    }

    fn summarize(&self, result: &ToolResult) -> ToolSummary {
        default_summary(result)
    }
}

fn backup_path_for(path: &std::path::Path) -> std::path::PathBuf {
    let mut backup = path.as_os_str().to_os_string();
    backup.push(".sakha-bak");
    std::path::PathBuf::from(backup)
}

// ---------------------------------------------------------------------
// file.patch
// ---------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct FilePatchTool;

#[async_trait]
impl Tool for FilePatchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("file.patch"),
            description: "Apply a unified diff patch to a file in the workspace".to_string(),
            input_schema: ToolInputSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "diff": {"type": "string"}
                },
                "required": ["path", "diff"]
            })),
            output_schema: ToolOutputSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "applied": {"type": "boolean"},
                    "conflict": {"type": "boolean"}
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

    async fn plan(&self, input: &ValidatedInput, _context: &ToolContext) -> SakhaResult<ToolPlan> {
        let path = input.0.get("path").and_then(|p| p.as_str()).unwrap_or_default();
        Ok(ToolPlan {
            summary: format!("patch {path}"),
            affected_paths: vec![path.to_string()],
            is_destructive: true,
        })
    }

    async fn execute(&self, input: &ValidatedInput, context: &ToolContext) -> SakhaResult<ToolResult> {
        let path_str = input
            .0
            .get("path")
            .and_then(|p| p.as_str())
            .ok_or_else(|| SakhaError::invalid_input(MODULE, "missing path"))?;
        let diff_text = input
            .0
            .get("diff")
            .and_then(|d| d.as_str())
            .ok_or_else(|| SakhaError::invalid_input(MODULE, "missing diff"))?;

        let resolved = resolve_in_workspace(&context.workspace_root, path_str)?;

        let original = tokio::fs::read_to_string(&resolved)
            .await
            .map_err(|e| SakhaError::invalid_input(MODULE, format!("read failed: {e}")))?;

        match crate::patch::apply_unified_diff(&original, diff_text) {
            Ok(patched) => {
                // Backup snapshot before write, per patch-safety rules.
                let backup_path = backup_path_for(&resolved);
                let _ = tokio::fs::write(&backup_path, original.as_bytes()).await;

                tokio::fs::write(&resolved, patched.as_bytes())
                    .await
                    .map_err(|e| SakhaError::invalid_input(MODULE, format!("write failed: {e}")))?;

                Ok(ToolResult::success(serde_json::json!({
                    "applied": true,
                    "conflict": false,
                })))
            }
            Err(conflict) => Ok(ToolResult {
                status: crate::tool::ToolCallStatus::Failed,
                output_json: serde_json::json!({
                    "applied": false,
                    "conflict": true,
                }),
                raw_output_ref: None,
                error_message: Some(format!("patch conflict: {conflict}")),
            }),
        }
    }

    fn summarize(&self, result: &ToolResult) -> ToolSummary {
        default_summary(result)
    }
}

// ---------------------------------------------------------------------
// file.delete
// ---------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct FileDeleteTool;

#[async_trait]
impl Tool for FileDeleteTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("file.delete"),
            description: "Delete a file in the workspace".to_string(),
            input_schema: ToolInputSchema(serde_json::json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"]
            })),
            output_schema: ToolOutputSchema(serde_json::json!({
                "type": "object",
                "properties": {"deleted": {"type": "boolean"}}
            })),
            permission_spec: ToolPermissionSpec {
                required: vec![PermissionKind::FileDelete],
            },
            idempotency_policy: IdempotencyPolicy::NotIdempotent,
        }
    }

    fn validate(&self, input: serde_json::Value) -> SakhaResult<ValidatedInput> {
        validate_against_schema(MODULE, &input, &self.spec().input_schema.0)?;
        Ok(ValidatedInput(input))
    }

    async fn plan(&self, input: &ValidatedInput, _context: &ToolContext) -> SakhaResult<ToolPlan> {
        let path = input.0.get("path").and_then(|p| p.as_str()).unwrap_or_default();
        Ok(ToolPlan {
            summary: format!("delete {path}"),
            affected_paths: vec![path.to_string()],
            is_destructive: true,
        })
    }

    async fn execute(&self, input: &ValidatedInput, context: &ToolContext) -> SakhaResult<ToolResult> {
        let path_str = input
            .0
            .get("path")
            .and_then(|p| p.as_str())
            .ok_or_else(|| SakhaError::invalid_input(MODULE, "missing path"))?;
        let resolved = resolve_in_workspace(&context.workspace_root, path_str)?;

        tokio::fs::remove_file(&resolved)
            .await
            .map_err(|e| SakhaError::invalid_input(MODULE, format!("delete failed: {e}")))?;

        Ok(ToolResult::success(serde_json::json!({"deleted": true})))
    }

    fn summarize(&self, result: &ToolResult) -> ToolSummary {
        default_summary(result)
    }
}

// ---------------------------------------------------------------------
// file.move
// ---------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct FileMoveTool;

#[async_trait]
impl Tool for FileMoveTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("file.move"),
            description: "Move or rename a file within the workspace".to_string(),
            input_schema: ToolInputSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "from": {"type": "string"},
                    "to": {"type": "string"}
                },
                "required": ["from", "to"]
            })),
            output_schema: ToolOutputSchema(serde_json::json!({
                "type": "object",
                "properties": {"moved": {"type": "boolean"}}
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

    async fn plan(&self, input: &ValidatedInput, _context: &ToolContext) -> SakhaResult<ToolPlan> {
        let from = input.0.get("from").and_then(|p| p.as_str()).unwrap_or_default();
        let to = input.0.get("to").and_then(|p| p.as_str()).unwrap_or_default();
        Ok(ToolPlan {
            summary: format!("move {from} -> {to}"),
            affected_paths: vec![from.to_string(), to.to_string()],
            is_destructive: true,
        })
    }

    async fn execute(&self, input: &ValidatedInput, context: &ToolContext) -> SakhaResult<ToolResult> {
        let from_str = input
            .0
            .get("from")
            .and_then(|p| p.as_str())
            .ok_or_else(|| SakhaError::invalid_input(MODULE, "missing from"))?;
        let to_str = input
            .0
            .get("to")
            .and_then(|p| p.as_str())
            .ok_or_else(|| SakhaError::invalid_input(MODULE, "missing to"))?;

        let from_resolved = resolve_in_workspace(&context.workspace_root, from_str)?;
        let to_resolved = resolve_in_workspace(&context.workspace_root, to_str)?;

        if let Some(parent) = to_resolved.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| SakhaError::invalid_input(MODULE, format!("create_dir_all failed: {e}")))?;
        }

        tokio::fs::rename(&from_resolved, &to_resolved)
            .await
            .map_err(|e| SakhaError::invalid_input(MODULE, format!("move failed: {e}")))?;

        Ok(ToolResult::success(serde_json::json!({"moved": true})))
    }

    fn summarize(&self, result: &ToolResult) -> ToolSummary {
        default_summary(result)
    }
}

// ---------------------------------------------------------------------
// file.list
// ---------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct FileListTool;

#[async_trait]
impl Tool for FileListTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("file.list"),
            description: "List files/directories under a workspace path".to_string(),
            input_schema: ToolInputSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "recursive": {"type": "boolean"}
                },
                "required": []
            })),
            output_schema: ToolOutputSchema(serde_json::json!({
                "type": "object",
                "properties": {"entries": {"type": "array"}}
            })),
            permission_spec: ToolPermissionSpec {
                required: vec![PermissionKind::FileRead],
            },
            idempotency_policy: IdempotencyPolicy::Idempotent,
        }
    }

    fn validate(&self, input: serde_json::Value) -> SakhaResult<ValidatedInput> {
        validate_against_schema(MODULE, &input, &self.spec().input_schema.0)?;
        Ok(ValidatedInput(input))
    }

    async fn plan(&self, _input: &ValidatedInput, _context: &ToolContext) -> SakhaResult<ToolPlan> {
        Ok(ToolPlan::default())
    }

    async fn execute(&self, input: &ValidatedInput, context: &ToolContext) -> SakhaResult<ToolResult> {
        let path_str = input.0.get("path").and_then(|p| p.as_str()).unwrap_or(".");
        let recursive = input.0.get("recursive").and_then(|r| r.as_bool()).unwrap_or(false);
        let resolved = resolve_in_workspace(&context.workspace_root, path_str)?;

        let mut entries = Vec::new();
        if recursive {
            for entry in walkdir::WalkDir::new(&resolved).into_iter().filter_map(|e| e.ok()) {
                if entry.path() == resolved {
                    continue;
                }
                entries.push(serde_json::json!({
                    "path": entry.path().to_string_lossy(),
                    "is_dir": entry.file_type().is_dir(),
                }));
            }
        } else {
            let mut read_dir = tokio::fs::read_dir(&resolved)
                .await
                .map_err(|e| SakhaError::invalid_input(MODULE, format!("list failed: {e}")))?;
            while let Some(entry) = read_dir
                .next_entry()
                .await
                .map_err(|e| SakhaError::invalid_input(MODULE, format!("list failed: {e}")))?
            {
                let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
                entries.push(serde_json::json!({
                    "path": entry.path().to_string_lossy(),
                    "is_dir": is_dir,
                }));
            }
        }

        Ok(ToolResult::success(serde_json::json!({"entries": entries})))
    }

    fn summarize(&self, result: &ToolResult) -> ToolSummary {
        default_summary(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn ctx(root: &std::path::Path) -> ToolContext {
        ToolContext::new(root)
    }

    #[tokio::test]
    async fn write_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let context = ctx(dir.path());

        let write_tool = FileWriteTool;
        let input = write_tool
            .validate(serde_json::json!({"path": "a.txt", "content": "hello"}))
            .unwrap();
        let result = write_tool.execute(&input, &context).await.unwrap();
        assert!(matches!(result.status, crate::tool::ToolCallStatus::Succeeded));

        let read_tool = FileReadTool;
        let input = read_tool.validate(serde_json::json!({"path": "a.txt"})).unwrap();
        let result = read_tool.execute(&input, &context).await.unwrap();
        assert_eq!(result.output_json["content"], "hello");
    }

    #[tokio::test]
    async fn write_creates_backup_on_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let context = ctx(dir.path());
        let tool = FileWriteTool;

        let input = tool
            .validate(serde_json::json!({"path": "a.txt", "content": "v1"}))
            .unwrap();
        tool.execute(&input, &context).await.unwrap();

        let input = tool
            .validate(serde_json::json!({"path": "a.txt", "content": "v2"}))
            .unwrap();
        let result = tool.execute(&input, &context).await.unwrap();
        assert_eq!(result.output_json["backup_created"], true);
        assert!(dir.path().join("a.txt.sakha-bak").exists());
    }

    #[tokio::test]
    async fn path_traversal_is_blocked() {
        let dir = tempfile::tempdir().unwrap();
        let context = ctx(dir.path());
        let tool = FileReadTool;
        let input = tool
            .validate(serde_json::json!({"path": "../../etc/passwd"}))
            .unwrap();
        let result = tool.execute(&input, &context).await;
        assert!(result.is_err());
    }

    #[test]
    fn schema_validation_rejects_missing_path() {
        let tool = FileReadTool;
        let result = tool.validate(serde_json::json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn schema_validation_rejects_wrong_type() {
        let tool = FileReadTool;
        let result = tool.validate(serde_json::json!({"path": 123}));
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn patch_applies_unified_diff() {
        let dir = tempfile::tempdir().unwrap();
        let context = ctx(dir.path());

        tokio::fs::write(dir.path().join("a.txt"), "line1\nline2\nline3\n")
            .await
            .unwrap();

        let diff = "--- a/a.txt\n+++ b/a.txt\n@@ -1,3 +1,3 @@\n line1\n-line2\n+line2-modified\n line3\n";

        let tool = FilePatchTool;
        let input = tool
            .validate(serde_json::json!({"path": "a.txt", "diff": diff}))
            .unwrap();
        let result = tool.execute(&input, &context).await.unwrap();
        assert!(matches!(result.status, crate::tool::ToolCallStatus::Succeeded));

        let content = tokio::fs::read_to_string(dir.path().join("a.txt")).await.unwrap();
        assert!(content.contains("line2-modified"));
    }

    #[tokio::test]
    async fn patch_conflict_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let context = ctx(dir.path());

        tokio::fs::write(dir.path().join("a.txt"), "completely different content\n")
            .await
            .unwrap();

        let diff = "--- a/a.txt\n+++ b/a.txt\n@@ -1,3 +1,3 @@\n line1\n-line2\n+line2-modified\n line3\n";

        let tool = FilePatchTool;
        let input = tool
            .validate(serde_json::json!({"path": "a.txt", "diff": diff}))
            .unwrap();
        let result = tool.execute(&input, &context).await.unwrap();
        assert!(matches!(result.status, crate::tool::ToolCallStatus::Failed));
        assert_eq!(result.output_json["conflict"], true);
    }

    #[tokio::test]
    async fn delete_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        let context = ctx(dir.path());
        tokio::fs::write(dir.path().join("a.txt"), "x").await.unwrap();

        let tool = FileDeleteTool;
        let input = tool.validate(serde_json::json!({"path": "a.txt"})).unwrap();
        let result = tool.execute(&input, &context).await.unwrap();
        assert!(matches!(result.status, crate::tool::ToolCallStatus::Succeeded));
        assert!(!dir.path().join("a.txt").exists());
    }

    #[tokio::test]
    async fn move_renames_file() {
        let dir = tempfile::tempdir().unwrap();
        let context = ctx(dir.path());
        tokio::fs::write(dir.path().join("a.txt"), "x").await.unwrap();

        let tool = FileMoveTool;
        let input = tool
            .validate(serde_json::json!({"from": "a.txt", "to": "b.txt"}))
            .unwrap();
        let result = tool.execute(&input, &context).await.unwrap();
        assert!(matches!(result.status, crate::tool::ToolCallStatus::Succeeded));
        assert!(!dir.path().join("a.txt").exists());
        assert!(dir.path().join("b.txt").exists());
    }

    #[tokio::test]
    async fn list_returns_entries() {
        let dir = tempfile::tempdir().unwrap();
        let context = ctx(dir.path());
        tokio::fs::write(dir.path().join("a.txt"), "x").await.unwrap();
        tokio::fs::write(dir.path().join("b.txt"), "y").await.unwrap();

        let tool = FileListTool;
        let input = tool.validate(serde_json::json!({})).unwrap();
        let result = tool.execute(&input, &context).await.unwrap();
        let entries = result.output_json["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn arc_dyn_tool_is_object_safe() {
        let _tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(FileReadTool),
            Arc::new(FileWriteTool),
            Arc::new(FilePatchTool),
            Arc::new(FileDeleteTool),
            Arc::new(FileMoveTool),
            Arc::new(FileListTool),
        ];
        assert_eq!(_tools.len(), 6);
    }
}
