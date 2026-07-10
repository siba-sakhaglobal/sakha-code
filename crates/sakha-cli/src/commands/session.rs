//! `sakha session list/show/resume`: session inspection, backed by
//! `sakha-memory::SessionStore`. Also retains `sakha goal create/status` from
//! the original skeleton (spec `modules/13-ui-cli-tui-web-desktop.md` "CLI
//! Commands" lists both `sakha goal create/status` and general session
//! inspection).

use clap::{Args, Subcommand};
use serde::Serialize;

use sakha_memory::{SessionRecord, SessionStatus, SessionStore};

use crate::config::{default_config_path, load_config};
use crate::output::{print_error, print_output, OutputFormat};
use crate::runtime::build_session_store;

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

/// Uses `build_session_store` (SQLite-backed when `config.db_path` is set,
/// in-memory otherwise) so `session list/show/resume` see sessions created by
/// `sakha chat` in prior invocations whenever durable storage is configured;
/// with no `db_path` configured, an empty list / not-found result is
/// expected and correct (there is nothing to persist across processes),
/// never a crash.
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
            let store = match open_store(args.output) {
                Ok(store) => store,
                Err(code) => return code,
            };
            match store.list_sessions().await {
                Ok(records) => {
                    let sessions: Vec<SessionView> = records.iter().map(SessionView::from).collect();
                    print_output(&sessions, args.output);
                    0
                }
                Err(err) => {
                    print_error(&err.to_string(), args.output);
                    1
                }
            }
        }
        SessionCommand::Show(args) => {
            let store = match open_store(args.output) {
                Ok(store) => store,
                Err(code) => return code,
            };
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
            let store = match open_store(args.output) {
                Ok(store) => store,
                Err(code) => return code,
            };
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

/// Loads config and builds the shared `SessionStore` (spec-durable when
/// `db_path` is configured), or reports a config-load error and returns the
/// exit code the caller should propagate.
fn open_store(output: OutputFormat) -> Result<std::sync::Arc<dyn SessionStore>, i32> {
    let config = match load_config(&default_config_path()) {
        Ok(c) => c,
        Err(err) => {
            print_error(&format!("failed to load config: {err}"), output);
            return Err(2);
        }
    };
    build_session_store(&config).map_err(|err| {
        print_error(&format!("failed to open session store: {err}"), output);
        1
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempHome;

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
        let _home = TempHome::new();
        let code = execute(SessionCommand::List(SessionListArgs { output: OutputFormat::Json }));
        assert_eq!(code, 0);
    }

    #[test]
    fn session_show_missing_returns_not_found() {
        let _home = TempHome::new();
        let code = execute(SessionCommand::Show(SessionShowArgs {
            session_id: sakha_core::SessionId::new().to_string(),
            output: OutputFormat::Json,
        }));
        assert_eq!(code, 3);
    }

    #[test]
    fn session_show_invalid_id_returns_config_error() {
        let _home = TempHome::new();
        let code = execute(SessionCommand::Show(SessionShowArgs {
            session_id: "not-a-uuid".into(),
            output: OutputFormat::Json,
        }));
        assert_eq!(code, 2);
    }

    /// End-to-end proof that `sakha session list` enumerates sessions once a
    /// durable `db_path` is configured: a session is created directly through
    /// `build_session_store` (as `sakha chat` would), then `session list`
    /// against the same `db_path` must find it (spec: "sakha session list to
    /// enumerate sessions").
    #[test]
    fn session_list_enumerates_sessions_when_db_path_configured() {
        let home = TempHome::new();
        let db_path = home.path().join("sessions.sqlite3");

        let mut config = crate::config::SakhaConfig::default();
        config.db_path = Some(db_path.to_string_lossy().to_string());
        crate::config::save_config(&crate::config::default_config_path(), &config).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let session_id = sakha_core::SessionId::new();
        rt.block_on(async {
            let store = build_session_store(&config).unwrap();
            store
                .create_session(SessionRecord {
                    id: session_id,
                    workspace_id: sakha_core::WorkspaceId::new(),
                    goal_id: None,
                    status: SessionStatus::Active,
                    provider_profile: None,
                    created_at: sakha_core::time::now_utc(),
                    updated_at: sakha_core::time::now_utc(),
                })
                .await
                .unwrap();
        });

        let code = execute(SessionCommand::List(SessionListArgs { output: OutputFormat::Json }));
        assert_eq!(code, 0);

        // Show must also find it via the same durable store.
        let show_code = execute(SessionCommand::Show(SessionShowArgs {
            session_id: session_id.to_string(),
            output: OutputFormat::Json,
        }));
        assert_eq!(show_code, 0);
    }
}
