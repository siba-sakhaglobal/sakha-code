//! `HandoffStore`: durable handoff artifacts for goals/loops. See spec
//! `06-context-memory-state.md` "Handoff Artifact Required Fields".

use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use sakha_core::{GoalId, SakhaError, SakhaResult};

use crate::db::Database;

/// Required fields for a handoff artifact per spec, sufficient for a fresh
/// agent to continue the work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffArtifact {
    pub goal_id: GoalId,
    pub objective: String,
    pub current_status: String,
    pub completed_work: Vec<String>,
    pub pending_work: Vec<String>,
    pub files_changed: Vec<String>,
    pub commands_run: Vec<String>,
    pub test_results: Vec<String>,
    pub decisions_made: Vec<String>,
    pub blockers: Vec<String>,
    pub next_suggested_action: String,
    pub compression_notes: Option<String>,
    pub written_at: chrono::DateTime<chrono::Utc>,
}

impl HandoffArtifact {
    pub fn new(goal_id: GoalId, objective: impl Into<String>) -> Self {
        Self {
            goal_id,
            objective: objective.into(),
            current_status: String::new(),
            completed_work: Vec::new(),
            pending_work: Vec::new(),
            files_changed: Vec::new(),
            commands_run: Vec::new(),
            test_results: Vec::new(),
            decisions_made: Vec::new(),
            blockers: Vec::new(),
            next_suggested_action: String::new(),
            compression_notes: None,
            written_at: sakha_core::time::now_utc(),
        }
    }
}

/// Reads/writes handoff artifacts keyed by goal id.
#[async_trait]
pub trait HandoffStore: Send + Sync {
    async fn write_handoff(&self, goal_id: GoalId, handoff: HandoffArtifact) -> SakhaResult<()>;
    async fn load_handoff(&self, goal_id: GoalId) -> SakhaResult<Option<HandoffArtifact>>;
}

/// In-memory `HandoffStore` for tests and as a safe default.
#[derive(Default)]
pub struct InMemoryHandoffStore {
    handoffs: std::sync::Mutex<std::collections::HashMap<GoalId, HandoffArtifact>>,
}

impl InMemoryHandoffStore {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Maps a poisoned-mutex error to a non-panicking `SakhaError`. A poisoned
/// lock means some other task panicked while holding it; rather than
/// propagating that panic to every future caller, surface it as a fatal
/// (but catchable) error so normal request-handling flow never panics.
fn poison_err(what: &str) -> SakhaError {
    SakhaError::fatal("sakha-memory", format!("{what}: lock poisoned by a prior panic"))
}

#[async_trait]
impl HandoffStore for InMemoryHandoffStore {
    async fn write_handoff(&self, goal_id: GoalId, handoff: HandoffArtifact) -> SakhaResult<()> {
        let mut handoffs = self.handoffs.lock().map_err(|_| poison_err("write_handoff"))?;
        handoffs.insert(goal_id, handoff);
        Ok(())
    }

    async fn load_handoff(&self, goal_id: GoalId) -> SakhaResult<Option<HandoffArtifact>> {
        let handoffs = self.handoffs.lock().map_err(|_| poison_err("load_handoff"))?;
        Ok(handoffs.get(&goal_id).cloned())
    }
}

/// SQLite-backed `HandoffStore`. One handoff row per goal; writing again for
/// the same goal overwrites the previous handoff (a goal has exactly one
/// "latest" handoff, per spec `load_handoff(goal_id) -> HandoffArtifact`).
pub struct SqliteHandoffStore {
    db: Arc<Database>,
}

impl SqliteHandoffStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

fn json_list(items: &[String]) -> String {
    serde_json::to_string(items).unwrap_or_else(|_| "[]".to_string())
}

fn parse_json_list(s: &str) -> SakhaResult<Vec<String>> {
    serde_json::from_str(s).map_err(|e| SakhaError::integrity("sakha-memory", "bad handoff list json").with_cause(e))
}

#[async_trait]
impl HandoffStore for SqliteHandoffStore {
    async fn write_handoff(&self, goal_id: GoalId, handoff: HandoffArtifact) -> SakhaResult<()> {
        self.db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO handoffs (
                    goal_id, objective, current_status, completed_work_json, pending_work_json,
                    files_changed_json, commands_run_json, test_results_json, decisions_made_json,
                    blockers_json, next_suggested_action, compression_notes, written_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT(goal_id) DO UPDATE SET
                    objective = excluded.objective,
                    current_status = excluded.current_status,
                    completed_work_json = excluded.completed_work_json,
                    pending_work_json = excluded.pending_work_json,
                    files_changed_json = excluded.files_changed_json,
                    commands_run_json = excluded.commands_run_json,
                    test_results_json = excluded.test_results_json,
                    decisions_made_json = excluded.decisions_made_json,
                    blockers_json = excluded.blockers_json,
                    next_suggested_action = excluded.next_suggested_action,
                    compression_notes = excluded.compression_notes,
                    written_at = excluded.written_at",
                rusqlite::params![
                    goal_id.to_string(),
                    handoff.objective,
                    handoff.current_status,
                    json_list(&handoff.completed_work),
                    json_list(&handoff.pending_work),
                    json_list(&handoff.files_changed),
                    json_list(&handoff.commands_run),
                    json_list(&handoff.test_results),
                    json_list(&handoff.decisions_made),
                    json_list(&handoff.blockers),
                    handoff.next_suggested_action,
                    handoff.compression_notes,
                    handoff.written_at.to_rfc3339(),
                ],
            )
        })?;
        Ok(())
    }

    async fn load_handoff(&self, goal_id: GoalId) -> SakhaResult<Option<HandoffArtifact>> {
        #[allow(clippy::type_complexity)]
        let row: Option<(
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            String,
        )> = self.db.with_conn(|conn| {
            conn.query_row(
                "SELECT objective, current_status, completed_work_json, pending_work_json,
                        files_changed_json, commands_run_json, test_results_json, decisions_made_json,
                        blockers_json, next_suggested_action, compression_notes, written_at
                 FROM handoffs WHERE goal_id = ?1",
                rusqlite::params![goal_id.to_string()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                        row.get(10)?,
                        row.get(11)?,
                    ))
                },
            )
            .optional()
        })?;

        let Some((
            objective,
            current_status,
            completed_work_json,
            pending_work_json,
            files_changed_json,
            commands_run_json,
            test_results_json,
            decisions_made_json,
            blockers_json,
            next_suggested_action,
            compression_notes,
            written_at,
        )) = row
        else {
            return Ok(None);
        };

        Ok(Some(HandoffArtifact {
            goal_id,
            objective,
            current_status,
            completed_work: parse_json_list(&completed_work_json)?,
            pending_work: parse_json_list(&pending_work_json)?,
            files_changed: parse_json_list(&files_changed_json)?,
            commands_run: parse_json_list(&commands_run_json)?,
            test_results: parse_json_list(&test_results_json)?,
            decisions_made: parse_json_list(&decisions_made_json)?,
            blockers: parse_json_list(&blockers_json)?,
            next_suggested_action,
            compression_notes,
            written_at: chrono::DateTime::parse_from_rfc3339(&written_at)
                .map_err(|e| SakhaError::integrity("sakha-memory", "bad written_at").with_cause(e))?
                .with_timezone(&chrono::Utc),
        }))
    }
}

#[cfg(test)]
mod sqlite_tests {
    use super::*;
    use crate::migrations::run_migrations;

    #[tokio::test]
    async fn handoff_round_trips_through_sqlite() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        run_migrations(&db).unwrap();
        let store = SqliteHandoffStore::new(db);
        let goal = GoalId::new();

        let mut handoff = HandoffArtifact::new(goal, "ship feature x");
        handoff.completed_work.push("wrote schema".into());
        handoff.pending_work.push("write tests".into());
        handoff.files_changed.push("src/handoff.rs".into());
        handoff.blockers.push("none".into());
        handoff.next_suggested_action = "run cargo test".into();

        store.write_handoff(goal, handoff.clone()).await.unwrap();
        let loaded = store.load_handoff(goal).await.unwrap().expect("handoff present");
        assert_eq!(loaded.objective, "ship feature x");
        assert_eq!(loaded.completed_work, vec!["wrote schema".to_string()]);
        assert_eq!(loaded.next_suggested_action, "run cargo test");
    }

    #[tokio::test]
    async fn writing_again_overwrites_previous_handoff() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        run_migrations(&db).unwrap();
        let store = SqliteHandoffStore::new(db);
        let goal = GoalId::new();

        store.write_handoff(goal, HandoffArtifact::new(goal, "first")).await.unwrap();
        store.write_handoff(goal, HandoffArtifact::new(goal, "second")).await.unwrap();

        let loaded = store.load_handoff(goal).await.unwrap().unwrap();
        assert_eq!(loaded.objective, "second");
    }

    #[tokio::test]
    async fn missing_handoff_returns_none() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        run_migrations(&db).unwrap();
        let store = SqliteHandoffStore::new(db);
        let loaded = store.load_handoff(GoalId::new()).await.unwrap();
        assert!(loaded.is_none());
    }
}
