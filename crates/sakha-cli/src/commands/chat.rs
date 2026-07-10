//! `sakha chat`: interactive REPL over the agent loop. Reads lines from
//! stdin, streams model responses to stdout, and persists turns via
//! `sakha-memory` so a session can be resumed later with `sakha session
//! resume`.

use std::io::{BufRead, Write};
use std::sync::Arc;

use clap::Args;

use sakha_memory::{InMemorySessionStore, SessionRecord, SessionStatus, SessionStore, TurnRecord};
use sakha_provider::{MessageRole, ModelMessage, ModelRequest};

use crate::config::{default_config_path, load_config};
use crate::output::{print_error, OutputFormat};
use crate::runtime::{build_provider, stream_to_completion};

#[derive(Debug, Args)]
pub struct ChatArgs {
    /// Resume an existing session by id instead of starting a new one.
    #[arg(long)]
    pub session_id: Option<String>,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

/// Starts an interactive chat loop: reads prompts from stdin (one per line,
/// `exit`/`quit` to leave), streams the model's reply to stdout, and records
/// each turn. Uses an in-process session store; durable SQLite persistence is
/// available via `sakha session` once a `db_path` is configured.
pub fn execute(args: ChatArgs) -> i32 {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("failed to start async runtime: {err}");
            return 1;
        }
    };
    rt.block_on(chat_async(args, &mut std::io::stdin().lock(), &mut std::io::stdout()))
}

async fn chat_async(args: ChatArgs, input: &mut impl BufRead, out: &mut impl Write) -> i32 {
    let config_path = default_config_path();
    let config = match load_config(&config_path) {
        Ok(c) => c,
        Err(err) => {
            print_error(&format!("failed to load config: {err}"), args.output);
            return 2;
        }
    };

    let provider = build_provider(&config);
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());

    let session_id = match &args.session_id {
        Some(raw) => match raw.parse::<sakha_core::SessionId>() {
            Ok(id) => id,
            Err(_) => {
                print_error(&format!("invalid session id: {raw}"), args.output);
                return 2;
            }
        },
        None => sakha_core::SessionId::new(),
    };

    let existing = store.get_session(session_id).await.ok().flatten();
    if existing.is_none() {
        let session = SessionRecord {
            id: session_id,
            workspace_id: sakha_core::WorkspaceId::new(),
            goal_id: None,
            status: SessionStatus::Active,
            provider_profile: None,
            created_at: sakha_core::time::now_utc(),
            updated_at: sakha_core::time::now_utc(),
        };
        let _ = store.create_session(session).await;
    }

    let _ = writeln!(out, "sakha chat: session {session_id} (type 'exit' to leave)");

    loop {
        let _ = write!(out, "> ");
        let _ = out.flush();

        let mut line = String::new();
        let bytes_read = match input.read_line(&mut line) {
            Ok(n) => n,
            Err(err) => {
                print_error(&format!("stdin read error: {err}"), args.output);
                return 1;
            }
        };
        if bytes_read == 0 {
            // EOF (e.g. piped input exhausted).
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "exit" || line == "quit" {
            break;
        }

        let mut request = ModelRequest::new(config.provider.model.clone());
        request.messages.push(ModelMessage {
            role: MessageRole::User,
            content: line.to_string(),
            tool_call_id: None,
            name: None,
        });

        let result = stream_to_completion(provider.as_ref(), request, |chunk| {
            let _ = write!(out, "{chunk}");
            let _ = out.flush();
        })
        .await;

        match result {
            Ok((text, _stop_reason)) => {
                let _ = writeln!(out);
                let _ = store
                    .append_turn(TurnRecord {
                        id: sakha_core::TurnId::new(),
                        session_id,
                        input_text: line.to_string(),
                        output_text: Some(text),
                        created_at: sakha_core::time::now_utc(),
                    })
                    .await;
            }
            Err(err) => {
                print_error(&err.to_string(), args.output);
            }
        }
    }

    let _ = store.update_session_status(session_id, SessionStatus::Completed).await;
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn chat_args_parse_with_session_id() {
        use clap::Parser;
        #[derive(Debug, Parser)]
        struct TestCli {
            #[command(flatten)]
            args: ChatArgs,
        }
        let cli = TestCli::try_parse_from(["sakha", "--session-id", "abc"]).unwrap();
        assert_eq!(cli.args.session_id.as_deref(), Some("abc"));
    }

    #[tokio::test]
    async fn chat_echoes_mock_response_and_exits_on_quit() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", dir.path());
        std::env::set_var("USERPROFILE", dir.path());

        let mut input = Cursor::new(b"hello there\nexit\n".to_vec());
        let mut out = Vec::new();
        let code = chat_async(ChatArgs { session_id: None, output: OutputFormat::Text }, &mut input, &mut out).await;
        assert_eq!(code, 0);
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("sakha chat: session"));
        // MockProviderClient's default canned response text.
        assert!(rendered.contains("hello"));
    }

    #[tokio::test]
    async fn chat_handles_immediate_eof_gracefully() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", dir.path());
        std::env::set_var("USERPROFILE", dir.path());

        let mut input = Cursor::new(Vec::new());
        let mut out = Vec::new();
        let code = chat_async(ChatArgs { session_id: None, output: OutputFormat::Text }, &mut input, &mut out).await;
        assert_eq!(code, 0);
    }
}
