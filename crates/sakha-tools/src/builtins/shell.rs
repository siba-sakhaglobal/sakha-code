//! Built-in `shell.run` tool: executes a command under a timeout with
//! captured/truncated stdout+stderr. See
//! `modules/09-terminal-process-automation.md`.

use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use sakha_core::{SakhaError, SakhaResult};
use sakha_security::PermissionKind;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::schema::validate_against_schema;
use crate::tool::{
    IdempotencyPolicy, Tool, ToolContext, ToolInputSchema, ToolName, ToolOutputSchema,
    ToolPermissionSpec, ToolPlan, ToolResult, ToolSpec, ToolSummary, ValidatedInput,
};

const MODULE: &str = "sakha-tools::shell";

/// Default command timeout when the caller doesn't specify one.
const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// Hard cap on captured output per stream, to avoid unbounded memory growth
/// and huge model-visible output (module 09 "Output over threshold must be
/// summarized/compressed").
const MAX_CAPTURED_BYTES: usize = 200 * 1024;

/// Environment variable name fragments that mark a variable as a secret and
/// therefore excluded from the sanitized child environment. Case-insensitive
/// substring match, per module 09 "Commands inherit sanitized environment by
/// default" / "Never pass secret values to model-visible output".
const SECRET_MARKERS: &[&str] = &["SECRET", "TOKEN", "PASSWORD", "API_KEY", "APIKEY", "PRIVATE_KEY"];

fn is_secret_env_var(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    SECRET_MARKERS.iter().any(|marker| upper.contains(marker))
}

/// Reads up to `max_bytes` from `reader`, returning the captured text and
/// whether it was truncated.
async fn capture_stream<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    max_bytes: usize,
) -> (String, bool) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut truncated = false;
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                if buf.len() + n > max_bytes {
                    let remaining = max_bytes.saturating_sub(buf.len());
                    buf.extend_from_slice(&chunk[..remaining.min(n)]);
                    truncated = true;
                    // Keep draining so the child doesn't block on a full pipe,
                    // but stop accumulating.
                    while reader.read(&mut chunk).await.unwrap_or(0) > 0 {}
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            Err(_) => break,
        }
    }
    (String::from_utf8_lossy(&buf).to_string(), truncated)
}

/// Runs a shell command with a timeout, capturing (and capping) stdout and
/// stderr, and a sanitized environment.
#[derive(Debug, Default)]
pub struct ShellRunTool;

#[async_trait]
impl Tool for ShellRunTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("shell.run"),
            description: "Run a shell command in the workspace".to_string(),
            input_schema: ToolInputSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"},
                    "timeout_secs": {"type": "integer"},
                    "cwd": {"type": "string"}
                },
                "required": ["command"]
            })),
            output_schema: ToolOutputSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "exit_code": {"type": "integer"},
                    "stdout": {"type": "string"},
                    "stderr": {"type": "string"},
                    "timed_out": {"type": "boolean"}
                }
            })),
            permission_spec: ToolPermissionSpec {
                required: vec![PermissionKind::ShellRunArbitrary],
            },
            idempotency_policy: IdempotencyPolicy::NotIdempotent,
        }
    }

    fn validate(&self, input: serde_json::Value) -> SakhaResult<ValidatedInput> {
        validate_against_schema(MODULE, &input, &self.spec().input_schema.0)?;
        let command = input.get("command").and_then(|c| c.as_str()).unwrap_or("");
        if command.trim().is_empty() {
            return Err(SakhaError::invalid_input(MODULE, "command must not be empty"));
        }
        Ok(ValidatedInput(input))
    }

    async fn plan(&self, input: &ValidatedInput, _context: &ToolContext) -> SakhaResult<ToolPlan> {
        let command = input.0.get("command").and_then(|c| c.as_str()).unwrap_or_default();
        Ok(ToolPlan {
            summary: format!("run: {command}"),
            affected_paths: Vec::new(),
            is_destructive: true,
        })
    }

    async fn execute(&self, input: &ValidatedInput, context: &ToolContext) -> SakhaResult<ToolResult> {
        let command_str = input
            .0
            .get("command")
            .and_then(|c| c.as_str())
            .ok_or_else(|| SakhaError::invalid_input(MODULE, "missing command"))?;
        let timeout_secs = input
            .0
            .get("timeout_secs")
            .and_then(|t| t.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_SECS);
        let cwd = input
            .0
            .get("cwd")
            .and_then(|c| c.as_str())
            .map(|c| context.workspace_root.join(c))
            .unwrap_or_else(|| context.workspace_root.clone());

        let mut command = build_shell_command(command_str);
        command.current_dir(&cwd);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        command.env_clear();
        for (key, value) in std::env::vars() {
            if !is_secret_env_var(&key) {
                command.env(key, value);
            }
        }

        let mut child = command
            .spawn()
            .map_err(|e| SakhaError::invalid_input(MODULE, format!("failed to spawn command: {e}")))?;

        let mut stdout = child.stdout.take();
        let mut stderr = child.stderr.take();

        let run_fut = async {
            let stdout_fut = async {
                if let Some(s) = stdout.take() {
                    capture_stream(s, MAX_CAPTURED_BYTES).await
                } else {
                    (String::new(), false)
                }
            };
            let stderr_fut = async {
                if let Some(s) = stderr.take() {
                    capture_stream(s, MAX_CAPTURED_BYTES).await
                } else {
                    (String::new(), false)
                }
            };
            let (stdout_res, stderr_res, status) =
                tokio::join!(stdout_fut, stderr_fut, child.wait());
            (stdout_res, stderr_res, status)
        };

        match tokio::time::timeout(Duration::from_secs(timeout_secs), run_fut).await {
            Ok((stdout_res, stderr_res, status)) => {
                let status = status
                    .map_err(|e| SakhaError::invalid_input(MODULE, format!("wait failed: {e}")))?;
                let exit_code = status.code().unwrap_or(-1);
                Ok(ToolResult::success(serde_json::json!({
                    "exit_code": exit_code,
                    "stdout": stdout_res.0,
                    "stdout_truncated": stdout_res.1,
                    "stderr": stderr_res.0,
                    "stderr_truncated": stderr_res.1,
                    "timed_out": false,
                })))
            }
            Err(_) => {
                // Timed out: kill the process (and its tree, best-effort).
                let _ = child.start_kill();
                let _ = child.wait().await;
                Ok(ToolResult {
                    status: crate::tool::ToolCallStatus::TimedOut,
                    output_json: serde_json::json!({
                        "exit_code": null,
                        "stdout": "",
                        "stderr": "",
                        "timed_out": true,
                    }),
                    raw_output_ref: None,
                    error_message: Some(format!("command timed out after {timeout_secs}s")),
                })
            }
        }
    }

    fn summarize(&self, result: &ToolResult) -> ToolSummary {
        if let Some(err) = &result.error_message {
            return ToolSummary {
                text: err.clone(),
                truncated: false,
            };
        }
        let exit_code = result.output_json.get("exit_code").cloned().unwrap_or(serde_json::Value::Null);
        ToolSummary {
            text: format!("exit_code={exit_code}"),
            truncated: false,
        }
    }
}

/// Builds a `Command` that runs `command_str` through the platform shell.
fn build_shell_command(command_str: &str) -> Command {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(command_str);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command_str);
        cmd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ToolContext {
        ToolContext::new(std::env::temp_dir())
    }

    #[tokio::test]
    async fn simple_command_succeeds() {
        let tool = ShellRunTool;
        let cmd = if cfg!(windows) { "echo hello" } else { "echo hello" };
        let input = tool.validate(serde_json::json!({"command": cmd})).unwrap();
        let result = tool.execute(&input, &ctx()).await.unwrap();
        assert!(matches!(result.status, crate::tool::ToolCallStatus::Succeeded));
        assert_eq!(result.output_json["exit_code"], 0);
        assert!(result.output_json["stdout"].as_str().unwrap().contains("hello"));
    }

    #[tokio::test]
    async fn timeout_kills_process() {
        let tool = ShellRunTool;
        let cmd = if cfg!(windows) {
            "ping -n 10 127.0.0.1 > NUL"
        } else {
            "sleep 10"
        };
        let input = tool
            .validate(serde_json::json!({"command": cmd, "timeout_secs": 1}))
            .unwrap();
        let start = std::time::Instant::now();
        let result = tool.execute(&input, &ctx()).await.unwrap();
        assert!(matches!(result.status, crate::tool::ToolCallStatus::TimedOut));
        assert!(start.elapsed() < Duration::from_secs(8));
    }

    #[tokio::test]
    async fn nonzero_exit_is_reported() {
        let tool = ShellRunTool;
        let cmd = if cfg!(windows) { "exit 3" } else { "exit 3" };
        let input = tool.validate(serde_json::json!({"command": cmd})).unwrap();
        let result = tool.execute(&input, &ctx()).await.unwrap();
        assert_eq!(result.output_json["exit_code"], 3);
    }

    #[test]
    fn validate_rejects_empty_command() {
        let tool = ShellRunTool;
        let result = tool.validate(serde_json::json!({"command": ""}));
        assert!(result.is_err());
    }

    #[test]
    fn validate_rejects_missing_command() {
        let tool = ShellRunTool;
        let result = tool.validate(serde_json::json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn secret_env_vars_are_flagged() {
        assert!(is_secret_env_var("API_KEY"));
        assert!(is_secret_env_var("my_secret_token"));
        assert!(!is_secret_env_var("PATH"));
    }
}
