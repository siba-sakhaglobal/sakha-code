//! `sakha session list/show/resume`: session inspection, backed by
//! `sakha-memory::SessionStore`. Also retains `sakha goal create/status` from
//! the original skeleton (spec `modules/13-ui-cli-tui-web-desktop.md` "CLI
//! Commands" lists both `sakha goal create/status` and general session
//! inspection).

use clap::{Args, Subcommand};
use serde::Serialize;

use sakha_memory::{InMemorySessionStore, SessionRecord, SessionStatus, SessionStore};

use crate::output::{print_error, print_output, OutputFormat};

#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// `sakha goal create`
    GoalCreate(GoalCreateArgs),
    /// `sakha goal status`
    GoalStatus(GoalStatusArgs),
    /// `sakha session list`
    List(SessionListArgs),
    /// `sakha session show <session_id>`
    Show(SessionShowArgs),
    /// `sakha session resume <session_id>`
    Resume(SessionResumeArgs),
}

#[derive(Debug, Args)]
pub struct GoalCreateArgs {
    pub objective: String,
}

#[derive(Debug, Args)]
pub struct GoalStatusArgs {
    pub goal_id: String,
}

#[derive(Debug, Args)]
pub struct SessionListArgs {
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Args)]
pub struct SessionShowArgs {
    pub session_id: String,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Args)]
pub struct SessionResumeArgs {
    pub session_id: String,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Serialize)]
struct SessionView {
    id: String,
    status: String,
    created_at: String,
    updated_at: String,
}

impl From<&SessionRecord> for SessionView {
    fn from(record: &SessionRecord) -> Self {
        Self {
            id: record.id.to_string(),
            status: format!("{:?}", record.status).to_lowercase(),
            created_at: record.created_at.to_rfc3339(),
            updated_at: record.updated_at.to_rfc3339(),
        }
    }
}

pub fn execute(command: SessionCommand) -> i32 {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("failed to start async runtime: {err}");
            return 1;
        }
    };
    rt.block_on(execute_async(command))
}

/// Sessions created via `sakha chat` currently live in that process's
/// in-memory store; `session list/show/resume` here operate against a fresh
/// in-memory store (empty on every invocation) until a durable `db_path` is
/// configured and threaded through, at which point this swaps to
/// `SqliteSessionStore`. This is documented rather than silently misleading:
/// an empty list / not-found result is expected and correct for the current
/// wiring, never a crash.
async fn execute_async(command: SessionCommand) -> i32 {
    match command {
        SessionCommand::GoalCreate(args) => {
            let goal_id = sakha_core::GoalId::new();
            println!("created goal {goal_id} (objective: {})", args.objective);
            println!("note: goal persistence requires sakha-loop wiring (not yet implemented)");
            0
        }
        SessionCommand::GoalStatus(args) => {
            eprintln!("sakha goal status: goal store not yet wired (goal_id: {})", args.goal_id);
            1
        }
        SessionCommand::List(args) => {
            let store = InMemorySessionStore::new();
            let sessions: Vec<SessionView> = list_all(&store).await.iter().map(SessionView::from).collect();
            print_output(&sessions, args.output);
            0
        }
        SessionCommand::Show(args) => {
            let store = InMemorySessionStore::new();
            let id = match args.session_id.parse::<sakha_core::SessionId>() {
                Ok(id) => id,
                Err(_) => {
                    print_error(&format!("invalid session id: {}", args.session_id), args.output);
                    return 2;
                }
            };
            match store.get_session(id).await {
                Ok(Some(record)) => {
                    print_output(&SessionView::from(&record), args.output);
                    0
                }
                Ok(None) => {
                    print_error(&format!("session not found: {id}"), args.output);
                    3
                }
                Err(err) => {
                    print_error(&err.to_string(), args.output);
                    1
                }
            }
        }
        SessionCommand::Resume(args) => {
            let store = InMemorySessionStore::new();
            let id = match args.session_id.parse::<sakha_core::SessionId>() {
                Ok(id) => id,
                Err(_) => {
                    print_error(&format!("invalid session id: {}", args.session_id), args.output);
                    return 2;
                }
            };
            match store.get_session(id).await {
                Ok(Some(mut record)) => {
                    record.status = SessionStatus::Active;
                    let _ = store.update_session_status(id, SessionStatus::Active).await;
                    print_output(&SessionView::from(&record), args.output);
                    0
                }
                Ok(None) => {
                    print_error(&format!("session not found: {id}"), args.output);
                    3
                }
                Err(err) => {
                    print_error(&err.to_string(), args.output);
                    1
                }
            }
        }
    }
}

/// `InMemorySessionStore` does not expose a `list_all`; this helper is a
/// placeholder that returns an empty list until a durable listing API lands.
/// Kept as its own function so the swap to a real `list_sessions()` (once
/// added to `SessionStore`) is a one-line change at the call site.
async fn list_all(_store: &InMemorySessionStore) -> Vec<SessionRecord> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_create_parses() {
        use clap::Parser;
        #[derive(Debug, Parser)]
        struct TestCli {
            #[command(subcommand)]
            cmd: SessionCommand,
        }
        let cli = TestCli::try_parse_from(["sakha", "goal-create", "ship it"]).unwrap();
        match cli.cmd {
            SessionCommand::GoalCreate(args) => assert_eq!(args.objective, "ship it"),
            _ => panic!("expected GoalCreate"),
        }
    }

    #[test]
    fn session_list_returns_success() {
        let code = execute(SessionCommand::List(SessionListArgs { output: OutputFormat::Json }));
        assert_eq!(code, 0);
    }

    #[test]
    fn session_show_missing_returns_not_found() {
        let code = execute(SessionCommand::Show(SessionShowArgs {
            session_id: sakha_core::SessionId::new().to_string(),
            output: OutputFormat::Json,
        }));
        assert_eq!(code, 3);
    }

    #[test]
    fn session_show_invalid_id_returns_config_error() {
        let code = execute(SessionCommand::Show(SessionShowArgs {
            session_id: "not-a-uuid".into(),
            output: OutputFormat::Json,
        }));
        assert_eq!(code, 2);
    }
}
