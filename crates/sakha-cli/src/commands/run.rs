//! `sakha run <prompt>`: non-interactive single-shot run. Streams the model
//! response to stdout and exits with a code reflecting outcome, per spec
//! "Non-interactive run exit codes".

use clap::Args;
use std::io::Write;

use sakha_memory::{SessionRecord, SessionStatus, TurnRecord};
use sakha_provider::{MessageRole, ModelMessage, ModelRequest, StopReason};

use crate::config::{default_config_path, load_config};
use crate::output::{print_error, OutputFormat};
use crate::runtime::{build_provider, build_session_store, build_tool_registry, run_agent_turn};
use crate::skills::{build_startup_system_messages, discover_skills};

#[derive(Debug, Args)]
pub struct RunArgs {
    /// The prompt/task for the agent to execute.
    pub prompt: String,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,

    /// Override the model name from config for this run.
    #[arg(long)]
    pub model: Option<String>,

    /// Inject a discovered skill's full instructions as a system message up
    /// front, skipping the `skill.activate` round-trip. Must name a skill
    /// discovered under the standard tiers (see `sakha skills list`).
    #[arg(long)]
    pub skill: Option<String>,
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
    let registry = build_tool_registry();

    // Persist the run as a session (spec M1: "usage recorded, session
    // persisted") — SQLite-backed when `db_path` is configured, in-memory
    // otherwise, same store `chat`/`session list` use.
    let store = match build_session_store(&config) {
        Ok(store) => store,
        Err(err) => {
            print_error(&format!("failed to open session store: {err}"), args.output);
            return exit_code::CONFIG_ERROR;
        }
    };
    let session_id = sakha_core::SessionId::new();
    let _ = store
        .create_session(SessionRecord {
            id: session_id,
            workspace_id: sakha_core::WorkspaceId::new(),
            goal_id: None,
            status: SessionStatus::Active,
            provider_profile: None,
            created_at: sakha_core::time::now_utc(),
            updated_at: sakha_core::time::now_utc(),
        })
        .await;

    let cwd = std::env::current_dir().ok();
    let (skills, _warnings) = discover_skills(cwd.as_deref());
    let system_messages = match build_startup_system_messages(&skills, args.skill.as_deref()) {
        Ok(messages) => messages,
        Err(err) => {
            print_error(&err, args.output);
            return exit_code::CONFIG_ERROR;
        }
    };

    let mut request = ModelRequest::new(config.provider.model.clone());
    for message in system_messages {
        request.messages.push(ModelMessage {
            tool_calls: Vec::new(),
            role: MessageRole::System,
            content: message,
            tool_call_id: None,
            name: None,
        });
    }
    request.messages.push(ModelMessage { tool_calls: Vec::new(),
        role: MessageRole::User,
        content: args.prompt.clone(),
        tool_call_id: None,
        name: None,
    });

    let output = args.output;
    let text_mode = matches!(output, OutputFormat::Text);
    let result = run_agent_turn(
        provider.as_ref(),
        &registry,
        request,
        |chunk| {
            if text_mode {
                print!("{chunk}");
                let _ = std::io::stdout().flush();
            }
        },
        |tool_name, succeeded| {
            if text_mode {
                eprintln!("[tool] {tool_name}: {}", if succeeded { "ok" } else { "failed" });
            }
        },
    )
    .await;

    match result {
        Ok((text, stop_reason)) => {
            let _ = store
                .append_turn(TurnRecord {
                    id: sakha_core::TurnId::new(),
                    session_id,
                    input_text: args.prompt.clone(),
                    output_text: Some(text.clone()),
                    created_at: sakha_core::time::now_utc(),
                })
                .await;
            let _ = store.update_session_status(session_id, SessionStatus::Completed).await;
            match output {
                OutputFormat::Text => {
                    println!();
                }
                OutputFormat::Json => {
                    let payload = serde_json::json!({
                        "prompt": args.prompt,
                        "response": text,
                        "stop_reason": stop_reason,
                        "session_id": session_id.to_string(),
                    });
                    match serde_json::to_string_pretty(&payload) {
                        Ok(json) => println!("{json}"),
                        Err(err) => {
                            print_error(&format!("failed to serialize run output: {err}"), output);
                            return exit_code::MODEL_ERROR;
                        }
                    }
                }
            }
            match stop_reason {
                StopReason::Error | StopReason::ContentFilter => exit_code::MODEL_ERROR,
                _ => exit_code::SUCCESS,
            }
        }
        Err(err) => {
            let _ = store.update_session_status(session_id, SessionStatus::Blocked).await;
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
        // on the host by pointing the config directory at an empty temp dir
        // for the duration of this test.
        let _home = crate::test_support::TempHome::new();
        let code = execute(RunArgs { prompt: "hello".into(), output: OutputFormat::Json, model: None, skill: None });
        assert_eq!(code, exit_code::SUCCESS);
    }

    #[test]
    fn parses_skill_flag() {
        let cli = TestCli::try_parse_from(["sakha", "do it", "--skill", "review"]).unwrap();
        assert_eq!(cli.args.skill.as_deref(), Some("review"));
    }

    #[test]
    fn run_with_unknown_skill_flag_fails_fast() {
        let _home = crate::test_support::TempHome::new();
        let code = execute(RunArgs { prompt: "hello".into(), output: OutputFormat::Json, model: None, skill: Some("does-not-exist".into()) });
        assert_eq!(code, exit_code::CONFIG_ERROR);
    }
}
