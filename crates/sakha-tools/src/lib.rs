//! sakha-tools: tool trait, registry, executor, permission bridge, and built-in tools.
//!
//! See spec `modules/07-tool-system.md`, `modules/08-files-git-worktrees.md`,
//! `modules/09-terminal-process-automation.md`, and
//! `crates/crate-work-breakdown.md`. Key contract: `Tool` trait,
//! `ToolRegistry`, `ToolExecutor` driving
//! validate -> risk -> permission -> idempotency -> execute -> audit.

pub mod builtins;
pub mod executor;
pub mod fs_util;
pub mod patch;
pub mod permissions;
pub mod registry;
pub mod schema;
pub mod tool;

pub use executor::{ToolAuditRecord, ToolCall, ToolExecutor};
pub use permissions::PermissionBridge;
pub use registry::ToolRegistry;
pub use tool::{
    IdempotencyKey, IdempotencyPolicy, PermissionCheckRequest, PermissionChecker, Tool,
    ToolCallStatus, ToolContext, ToolInputSchema, ToolName, ToolOutputSchema, ToolPermissionSpec,
    ToolPlan, ToolResult, ToolSpec, ToolSummary, ValidatedInput,
};

/// Builds a `ToolRegistry` pre-populated with every built-in tool. Convenience
/// for hosts (agent/CLI/daemon) that just want the default tool set.
pub fn default_registry() -> ToolRegistry {
    use std::sync::Arc;
    let mut registry = ToolRegistry::new();

    registry.register(Arc::new(builtins::file::FileReadTool));
    registry.register(Arc::new(builtins::file::FileWriteTool));
    registry.register(Arc::new(builtins::file::FilePatchTool));
    registry.register(Arc::new(builtins::file::FileDeleteTool));
    registry.register(Arc::new(builtins::file::FileMoveTool));
    registry.register(Arc::new(builtins::file::FileListTool));

    registry.register(Arc::new(builtins::git::GitStatusTool));
    registry.register(Arc::new(builtins::git::GitDiffTool));
    registry.register(Arc::new(builtins::git::GitApplyPatchTool));
    registry.register(Arc::new(builtins::git::GitBranchTool));
    registry.register(Arc::new(builtins::git::GitWorktreeTool));
    registry.register(Arc::new(builtins::git::GitCommitTool));

    registry.register(Arc::new(builtins::shell::ShellRunTool));
    registry.register(Arc::new(builtins::search::SearchRipgrepTool));

    registry
}

pub fn crate_name() -> &'static str {
    "sakha-tools"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn registry_registers_and_looks_up_tools() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(builtins::search::SearchRipgrepTool));
        assert_eq!(registry.len(), 1);
        let found = registry.get(&ToolName::new("search.ripgrep"));
        assert!(found.is_some());
    }

    #[tokio::test]
    async fn executor_rejects_unknown_tool() {
        let registry = ToolRegistry::new();
        let policy = sakha_security::PermissionPolicy::new();
        let executor = ToolExecutor::new(registry, policy);
        let call = ToolCall {
            id: sakha_core::ToolCallId::new(),
            tool_name: ToolName::new("does.not.exist"),
            input_json: serde_json::json!({}),
        };
        let context = ToolContext::new(".");
        let result = executor.execute(call, &context).await;
        assert!(result.is_err());
    }

    #[test]
    fn schema_validation_rejects_bad_input() {
        let tool = builtins::file::FileReadTool;
        let result = tool.validate(serde_json::json!({"path": 123}));
        assert!(result.is_err());
    }

    #[test]
    fn default_registry_contains_all_builtins() {
        let registry = default_registry();
        for name in [
            "file.read",
            "file.write",
            "file.patch",
            "file.delete",
            "file.move",
            "file.list",
            "git.status",
            "git.diff",
            "git.apply_patch",
            "git.branch",
            "git.worktree",
            "git.commit",
            "shell.run",
            "search.ripgrep",
        ] {
            assert!(
                registry.get(&ToolName::new(name)).is_some(),
                "missing built-in tool: {name}"
            );
        }
    }
}
