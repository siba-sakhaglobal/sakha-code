//! `sakha run <prompt>`: non-interactive single-shot run. Streams the model
//! response to stdout and exits with a code reflecting outcome, per spec
//! "Non-interactive run exit codes".

use clap::Args;
use std::io::Write;

use sakha_provider::{MessageRole, ModelMessage, ModelRequest, StopReason};

use crate::config::{default_config_path, load_config};
use crate::output::{print_error, OutputFormat};
use crate::runtime::{build_provider, stream_to_completion};

#[derive(Debug, Args)]
pub struct RunArgs {
    /// The prompt/task for the agent to execute.
    pub prompt: String,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,

    /// Override the model name from config for this run.
    #[arg(long)]
    pub model: Option<String>,
}

/// Exit codes for `sakha run`, per spec "proper exit codes".
pub mod exit_code {
    pub const SUCCESS: i32 = 0;
    pub const MODEL_ERROR: i32 = 1;
    pub const CONFIG_ERROR: i32 = 2;
}

/// Executes a non-interactive run: loads config, builds the configured
/// provider (Mock by default, so this always works offline), streams the
/// response to stdout, and returns a process exit code.
pub fn execute(args: RunArgs) -> i32 {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(err) => {
            print_error(&format!("failed to start async runtime: {err}"), args.output);
            return exit_code::CONFIG_ERROR;
        }
    };
    rt.block_on(run_async(args))
}

async fn run_async(args: RunArgs) -> i32 {
    let config_path = default_config_path();
    let mut config = match load_config(&config_path) {
        Ok(c) => c,
        Err(err) => {
            print_error(&format!("failed to load config: {err}"), args.output);
            return exit_code::CONFIG_ERROR;
        }
    };
    if let Some(model) = &args.model {
        config.provider.model = model.clone();
    }

    let provider = build_provider(&config);

    let mut request = ModelRequest::new(config.provider.model.clone());
    request.messages.push(ModelMessage {
        role: MessageRole::User,
        content: args.prompt.clone(),
        tool_call_id: None,
        name: None,
    });

    let output = args.output;
    let text_mode = matches!(output, OutputFormat::Text);
    let result = stream_to_completion(provider.as_ref(), request, |chunk| {
        if text_mode {
            print!("{chunk}");
            let _ = std::io::stdout().flush();
        }
    })
    .await;

    match result {
        Ok((text, stop_reason)) => {
            match output {
                OutputFormat::Text => {
                    println!();
                }
                OutputFormat::Json => {
                    let payload = serde_json::json!({
                        "prompt": args.prompt,
                        "response": text,
                        "stop_reason": stop_reason,
                    });
                    println!("{}", serde_json::to_string_pretty(&payload).unwrap_or_default());
                }
            }
            match stop_reason {
                StopReason::Error | StopReason::ContentFilter => exit_code::MODEL_ERROR,
                _ => exit_code::SUCCESS,
            }
        }
        Err(err) => {
            print_error(&err.to_string(), output);
            exit_code::MODEL_ERROR
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Debug, Parser)]
    struct TestCli {
        #[command(flatten)]
        args: RunArgs,
    }

    #[test]
    fn parses_prompt_and_defaults_to_text_output() {
        let cli = TestCli::try_parse_from(["sakha", "fix the bug"]).unwrap();
        assert_eq!(cli.args.prompt, "fix the bug");
        assert_eq!(cli.args.output, OutputFormat::Text);
    }

    #[test]
    fn parses_json_output_flag() {
        let cli = TestCli::try_parse_from(["sakha", "do it", "--output", "json"]).unwrap();
        assert_eq!(cli.args.output, OutputFormat::Json);
    }

    #[test]
    fn run_with_mock_provider_exits_success() {
        // Force a mock-provider run regardless of any real ~/.sakha/config.toml
        // on the host by pointing the home directory at an empty temp dir for
        // the duration of this test. `dirs::home_dir()` reads `USERPROFILE` on
        // Windows and `HOME` elsewhere, so override both.
        let dir = tempfile::tempdir().unwrap();
        let _guard_home = EnvVarGuard::set("HOME", dir.path());
        let _guard_profile = EnvVarGuard::set("USERPROFILE", dir.path());
        let code = execute(RunArgs { prompt: "hello".into(), output: OutputFormat::Json, model: None });
        assert_eq!(code, exit_code::SUCCESS);
    }

    /// Minimal RAII env-var guard so tests don't leak env overrides into
    /// other tests running in the same process.
    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<std::path::Path>) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value.as_ref());
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }
}
