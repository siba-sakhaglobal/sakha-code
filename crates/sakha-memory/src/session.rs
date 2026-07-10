//! `SessionStore`: persists and resumes `Session`/`Turn` records. See
//! `04-core-domain-model.md` "Session" and "Turn".

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use sakha_core::{
    GoalId, ProviderProfileId, SakhaError, SakhaResult, SessionId, TurnId, WorkspaceId,
};

use crate::db::Database;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Paused,
    Completed,
    Blocked,
}

impl SessionStatus {
    fn as_str(self) -> &'static str {
        match self {
            SessionStatus::Active => "active",
            SessionStatus::Paused => "paused",
            SessionStatus::Completed => "completed",
            SessionStatus::Blocked => "blocked",
        }
    }

    fn parse(s: &str) -> SakhaResult<Self> {
        match s {
            "active" => Ok(SessionStatus::Active),
            "paused" => Ok(SessionStatus::Paused),
            "completed" => Ok(SessionStatus::Completed),
            "blocked" => Ok(SessionStatus::Blocked),
            other => Err(SakhaError::integrity("sakha-memory", format!("unknown session status: {other}"))),
        }
    }
}

/// Mirrors `04-core-domain-model.md` `Session`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: SessionId,
    pub workspace_id: WorkspaceId,
    pub goal_id: Option<GoalId>,
    pub status: SessionStatus,
    pub provider_profile: Option<ProviderProfileId>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Mirrors `04-core-domain-model.md` `Turn` (minus large nested collections,
/// which live in their own tables/records).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRecord {
    pub id: TurnId,
    pub session_id: SessionId,
    pub input_text: String,
    pub output_text: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Persists session/turn history and supports resuming a session by id.
#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn create_session(&self, session: SessionRecord) -> SakhaResult<()>;
    async fn get_session(&self, id: SessionId) -> SakhaResult<Option<SessionRecord>>;
    async fn update_session_status(&self, id: SessionId, status: SessionStatus) -> SakhaResult<()>;
    async fn append_turn(&self, turn: TurnRecord) -> SakhaResult<()>;
    async fn list_turns(&self, session_id: SessionId) -> SakhaResult<Vec<TurnRecord>>;
}

/// In-memory `SessionStore`, used for tests and as a safe default before the
/// SQLite-backed implementation lands.
#[derive(Default)]
pub struct InMemorySessionStore {
    sessions: std::sync::Mutex<std::collections::HashMap<SessionId, SessionRecord>>,
    turns: std::sync::Mutex<Vec<TurnRecord>>,
}

impl InMemorySessionStore {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Maps a poisoned-mutex error to a non-panicking `SakhaError`. A poisoned
/// lock means some other task panicked while holding it; rather than
/// propagating that panic to every future caller, surface it as a fatal
/// (but catchable) error so normal request-handling flow never panics.
fn poison_err(source_module: &'static str, what: &str) -> SakhaError {
    SakhaError::fatal(source_module, format!("{what}: lock poisoned by a prior panic"))
}

#[async_trait]
impl SessionStore for InMemorySessionStore {
    async fn create_session(&self, session: SessionRecord) -> SakhaResult<()> {
        let mut sessions = self.sessions.lock().map_err(|_| poison_err("sakha-memory", "create_session"))?;
        sessions.insert(session.id, session);
        Ok(())
    }

    async fn get_session(&self, id: SessionId) -> SakhaResult<Option<SessionRecord>> {
        let sessions = self.sessions.lock().map_err(|_| poison_err("sakha-memory", "get_session"))?;
        Ok(sessions.get(&id).cloned())
    }

    async fn update_session_status(&self, id: SessionId, status: SessionStatus) -> SakhaResult<()> {
        let mut sessions = self.sessions.lock().map_err(|_| poison_err("sakha-memory", "update_session_status"))?;
        if let Some(session) = sessions.get_mut(&id) {
            session.status = status;
        }
        Ok(())
    }

    async fn append_turn(&self, turn: TurnRecord) -> SakhaResult<()> {
        let mut turns = self.turns.lock().map_err(|_| poison_err("sakha-memory", "append_turn"))?;
        turns.push(turn);
        Ok(())
    }

    async fn list_turns(&self, session_id: SessionId) -> SakhaResult<Vec<TurnRecord>> {
        let turns = self.turns.lock().map_err(|_| poison_err("sakha-memory", "list_turns"))?;
        Ok(turns.iter().filter(|t| t.session_id == session_id).cloned().collect())
    }
}

/// SQLite-backed `SessionStore`, used for durable session resume across
/// process restarts. Blocking `rusqlite` calls run synchronously on the
/// calling async task per the crate's threading model (see `Database`).
pub struct SqliteSessionStore {
    db: Arc<Database>,
}

impl SqliteSessionStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

fn opt_to_str<T: ToString>(v: &Option<T>) -> Option<String> {
    v.as_ref().map(|x| x.to_string())
}

fn parse_opt<T: FromStr>(s: Option<String>) -> Option<T> {
    s.and_then(|s| T::from_str(&s).ok())
}

#[async_trait]
impl SessionStore for SqliteSessionStore {
    async fn create_session(&self, session: SessionRecord) -> SakhaResult<()> {
        self.db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, workspace_id, goal_id, status, provider_profile, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET
                    workspace_id = excluded.workspace_id,
                    goal_id = excluded.goal_id,
                    status = excluded.status,
                    provider_profile = excluded.provider_profile,
                    updated_at = excluded.updated_at",
                rusqlite::params![
                    session.id.to_string(),
                    session.workspace_id.to_string(),
                    opt_to_str(&session.goal_id),
                    session.status.as_str(),
                    opt_to_str(&session.provider_profile),
                    session.created_at.to_rfc3339(),
                    session.updated_at.to_rfc3339(),
                ],
            )
        })?;
        Ok(())
    }

    async fn get_session(&self, id: SessionId) -> SakhaResult<Option<SessionRecord>> {
        let row: Option<(String, String, Option<String>, String, Option<String>, String, String)> = self
            .db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT id, workspace_id, goal_id, status, provider_profile, created_at, updated_at
                     FROM sessions WHERE id = ?1",
                    rusqlite::params![id.to_string()],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                        ))
                    },
                )
                .optional()
            })?;

        let Some((id, workspace_id, goal_id, status, provider_profile, created_at, updated_at)) = row else {
            return Ok(None);
        };

        Ok(Some(SessionRecord {
            id: SessionId::from_str(&id).map_err(|e| SakhaError::integrity("sakha-memory", "bad session id").with_cause(e))?,
            workspace_id: WorkspaceId::from_str(&workspace_id)
                .map_err(|e| SakhaError::integrity("sakha-memory", "bad workspace id").with_cause(e))?,
            goal_id: parse_opt(goal_id),
            status: SessionStatus::parse(&status)?,
            provider_profile: parse_opt(provider_profile),
            created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                .map_err(|e| SakhaError::integrity("sakha-memory", "bad created_at").with_cause(e))?
                .with_timezone(&chrono::Utc),
            updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)
                .map_err(|e| SakhaError::integrity("sakha-memory", "bad updated_at").with_cause(e))?
                .with_timezone(&chrono::Utc),
        }))
    }

    async fn update_session_status(&self, id: SessionId, status: SessionStatus) -> SakhaResult<()> {
        self.db.with_conn(|conn| {
            conn.execute(
                "UPDATE sessions SET status = ?1, updated_at = ?2 WHERE id = ?3",
                rusqlite::params![status.as_str(), sakha_core::time::now_utc().to_rfc3339(), id.to_string()],
            )
        })?;
        Ok(())
    }

    async fn append_turn(&self, turn: TurnRecord) -> SakhaResult<()> {
        self.db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO turns (id, session_id, input_text, output_text, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET output_text = excluded.output_text",
                rusqlite::params![
                    turn.id.to_string(),
                    turn.session_id.to_string(),
                    turn.input_text,
                    turn.output_text,
                    turn.created_at.to_rfc3339(),
                ],
            )
        })?;
        Ok(())
    }

    async fn list_turns(&self, session_id: SessionId) -> SakhaResult<Vec<TurnRecord>> {
        let rows: Vec<(String, String, String, Option<String>, String)> = self.db.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, input_text, output_text, created_at
                 FROM turns WHERE session_id = ?1 ORDER BY created_at ASC",
            )?;
            let rows = stmt.query_map(rusqlite::params![session_id.to_string()], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
            })?;
            rows.collect()
        })?;

        rows.into_iter()
            .map(|(id, session_id, input_text, output_text, created_at)| {
                Ok(TurnRecord {
                    id: TurnId::from_str(&id).map_err(|e| SakhaError::integrity("sakha-memory", "bad turn id").with_cause(e))?,
                    session_id: SessionId::from_str(&session_id)
                        .map_err(|e| SakhaError::integrity("sakha-memory", "bad session id").with_cause(e))?,
                    input_text,
                    output_text,
                    created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                        .map_err(|e| SakhaError::integrity("sakha-memory", "bad created_at").with_cause(e))?
                        .with_timezone(&chrono::Utc),
                })
            })
            .collect()
    }
}
