//! `MemoryStore`: general-purpose durable memory records (facts, decisions,
//! preferences) with search. See `04-core-domain-model.md` `MemoryStore`
//! entries and spec `06-context-memory-state.md` "Main APIs".

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use sakha_core::{SakhaError, SakhaResult, WorkspaceId};

use crate::db::Database;

/// Category of a memory record. See spec "Memory Types".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    ConversationHistory,
    UserPreference,
    ProjectFact,
    ArchitectureFact,
    Decision,
    OpenTask,
    CompletedTask,
    FailedAttempt,
    ToolTrace,
    WebResearchSource,
    CompressionMarker,
    LoopHandoff,
    EvalRegression,
}

impl MemoryKind {
    fn as_str(self) -> &'static str {
        match self {
            MemoryKind::ConversationHistory => "conversation_history",
            MemoryKind::UserPreference => "user_preference",
            MemoryKind::ProjectFact => "project_fact",
            MemoryKind::ArchitectureFact => "architecture_fact",
            MemoryKind::Decision => "decision",
            MemoryKind::OpenTask => "open_task",
            MemoryKind::CompletedTask => "completed_task",
            MemoryKind::FailedAttempt => "failed_attempt",
            MemoryKind::ToolTrace => "tool_trace",
            MemoryKind::WebResearchSource => "web_research_source",
            MemoryKind::CompressionMarker => "compression_marker",
            MemoryKind::LoopHandoff => "loop_handoff",
            MemoryKind::EvalRegression => "eval_regression",
        }
    }

    fn parse(s: &str) -> SakhaResult<Self> {
        Ok(match s {
            "conversation_history" => MemoryKind::ConversationHistory,
            "user_preference" => MemoryKind::UserPreference,
            "project_fact" => MemoryKind::ProjectFact,
            "architecture_fact" => MemoryKind::ArchitectureFact,
            "decision" => MemoryKind::Decision,
            "open_task" => MemoryKind::OpenTask,
            "completed_task" => MemoryKind::CompletedTask,
            "failed_attempt" => MemoryKind::FailedAttempt,
            "tool_trace" => MemoryKind::ToolTrace,
            "web_research_source" => MemoryKind::WebResearchSource,
            "compression_marker" => MemoryKind::CompressionMarker,
            "loop_handoff" => MemoryKind::LoopHandoff,
            "eval_regression" => MemoryKind::EvalRegression,
            other => return Err(SakhaError::integrity("sakha-memory", format!("unknown memory kind: {other}"))),
        })
    }
}

/// A single durable memory record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: uuid::Uuid,
    pub workspace_id: WorkspaceId,
    pub kind: MemoryKind,
    pub text: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl MemoryRecord {
    pub fn new(workspace_id: WorkspaceId, kind: MemoryKind, text: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            workspace_id,
            kind,
            text: text.into(),
            created_at: sakha_core::time::now_utc(),
        }
    }
}

/// A search query against memory records.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryQuery {
    pub workspace_id: Option<WorkspaceId>,
    pub kind: Option<MemoryKind>,
    pub text_contains: Option<String>,
    pub limit: Option<u32>,
}

/// A scored search hit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryHit {
    pub record: MemoryRecord,
    pub relevance: RelevanceScore,
}

/// A relevance score in `[0.0, 1.0]` for a search hit.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RelevanceScore(pub f32);

/// A point-in-time summary of a session, produced by `summarize_session`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub text: String,
    pub token_estimate: u32,
}

/// General-purpose durable memory storage and search.
#[async_trait]
pub trait MemoryStore: Send + Sync {
    async fn write_memory(&self, record: MemoryRecord) -> SakhaResult<()>;
    async fn search_memory(&self, query: MemoryQuery) -> SakhaResult<Vec<MemoryHit>>;
    async fn summarize_session(&self, session_id: sakha_core::SessionId) -> SakhaResult<Summary>;
}

/// In-memory `MemoryStore` for tests and as a safe default.
#[derive(Default)]
pub struct InMemoryMemoryStore {
    records: std::sync::Mutex<Vec<MemoryRecord>>,
}

impl InMemoryMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl MemoryStore for InMemoryMemoryStore {
    async fn write_memory(&self, record: MemoryRecord) -> SakhaResult<()> {
        self.records.lock().unwrap().push(record);
        Ok(())
    }

    async fn search_memory(&self, query: MemoryQuery) -> SakhaResult<Vec<MemoryHit>> {
        let records = self.records.lock().unwrap();
        let hits = records
            .iter()
            .filter(|r| {
                query.workspace_id.map(|w| w == r.workspace_id).unwrap_or(true)
                    && query.kind.map(|k| k == r.kind).unwrap_or(true)
                    && query
                        .text_contains
                        .as_ref()
                        .map(|needle| r.text.contains(needle.as_str()))
                        .unwrap_or(true)
            })
            .take(query.limit.unwrap_or(u32::MAX) as usize)
            .map(|r| MemoryHit {
                record: r.clone(),
                relevance: RelevanceScore(1.0),
            })
            .collect();
        Ok(hits)
    }

    async fn summarize_session(&self, _session_id: sakha_core::SessionId) -> SakhaResult<Summary> {
        Ok(Summary { text: String::new(), token_estimate: 0 })
    }
}

/// SQLite-backed `MemoryStore`. Search uses a simple substring/kind/workspace
/// filter (per `MemoryQuery`) with relevance scored by keyword overlap; a
/// vector/full-text index (per spec "Storage") can be layered on later
/// without changing this trait.
pub struct SqliteMemoryStore {
    db: Arc<Database>,
}

impl SqliteMemoryStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

/// Crude relevance score: fraction of the query's whitespace-separated terms
/// that appear (case-insensitively) in the record text. Falls back to 1.0
/// when there is no text filter to score against.
fn score_relevance(text: &str, needle: Option<&str>) -> f32 {
    let Some(needle) = needle else { return 1.0 };
    let needle_lower = needle.to_lowercase();
    let terms: Vec<&str> = needle_lower.split_whitespace().collect();
    if terms.is_empty() {
        return 1.0;
    }
    let text_lower = text.to_lowercase();
    let matched = terms.iter().filter(|t| text_lower.contains(*t)).count();
    matched as f32 / terms.len() as f32
}

#[async_trait]
impl MemoryStore for SqliteMemoryStore {
    async fn write_memory(&self, record: MemoryRecord) -> SakhaResult<()> {
        self.db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO memory_records (id, workspace_id, kind, text, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET text = excluded.text",
                rusqlite::params![
                    record.id.to_string(),
                    record.workspace_id.to_string(),
                    record.kind.as_str(),
                    record.text,
                    record.created_at.to_rfc3339(),
                ],
            )
        })?;
        Ok(())
    }

    async fn search_memory(&self, query: MemoryQuery) -> SakhaResult<Vec<MemoryHit>> {
        let workspace_filter = query.workspace_id.map(|w| w.to_string());
        let kind_filter = query.kind.map(|k| k.as_str().to_string());
        let limit = query.limit.unwrap_or(u32::MAX) as i64;

        let rows: Vec<(String, String, String, String, String)> = self.db.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, workspace_id, kind, text, created_at FROM memory_records
                 WHERE (?1 IS NULL OR workspace_id = ?1)
                   AND (?2 IS NULL OR kind = ?2)
                 ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map(rusqlite::params![workspace_filter, kind_filter], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
            })?;
            rows.collect()
        })?;

        let mut hits = Vec::new();
        for (id, workspace_id, kind, text, created_at) in rows {
            if let Some(needle) = &query.text_contains {
                if !text.contains(needle.as_str()) {
                    continue;
                }
            }
            let relevance = score_relevance(&text, query.text_contains.as_deref());
            let record = MemoryRecord {
                id: uuid::Uuid::from_str(&id).map_err(|e| SakhaError::integrity("sakha-memory", "bad memory id").with_cause(e))?,
                workspace_id: WorkspaceId::from_str(&workspace_id)
                    .map_err(|e| SakhaError::integrity("sakha-memory", "bad workspace id").with_cause(e))?,
                kind: MemoryKind::parse(&kind)?,
                text,
                created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                    .map_err(|e| SakhaError::integrity("sakha-memory", "bad created_at").with_cause(e))?
                    .with_timezone(&chrono::Utc),
            };
            hits.push(MemoryHit { record, relevance: RelevanceScore(relevance) });
        }

        // Highest relevance first; stable by insertion (created_at desc) for ties.
        hits.sort_by(|a, b| b.relevance.0.partial_cmp(&a.relevance.0).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(limit.max(0) as usize);
        Ok(hits)
    }

    async fn summarize_session(&self, session_id: sakha_core::SessionId) -> SakhaResult<Summary> {
        let turns: Vec<(String, Option<String>)> = self.db.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT input_text, output_text FROM turns WHERE session_id = ?1 ORDER BY created_at ASC",
            )?;
            let rows = stmt.query_map(rusqlite::params![session_id.to_string()], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;
            rows.collect()
        })?;

        let mut text = String::new();
        for (input, output) in &turns {
            text.push_str("User: ");
            text.push_str(input);
            text.push('\n');
            if let Some(output) = output {
                text.push_str("Assistant: ");
                text.push_str(output);
                text.push('\n');
            }
        }
        // Rough token estimate: ~4 chars per token, matching common English
        // tokenization heuristics used elsewhere in the workspace's budgeting.
        let token_estimate = (text.chars().count() as u32).div_ceil(4);
        Ok(Summary { text, token_estimate })
    }
}

#[cfg(test)]
mod sqlite_tests {
    use super::*;
    use crate::migrations::run_migrations;

    #[tokio::test]
    async fn search_filters_by_workspace_and_kind() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        run_migrations(&db).unwrap();
        let store = SqliteMemoryStore::new(db);
        let ws = WorkspaceId::new();
        store
            .write_memory(MemoryRecord::new(ws, MemoryKind::ProjectFact, "uses rust and sqlite"))
            .await
            .unwrap();
        store
            .write_memory(MemoryRecord::new(ws, MemoryKind::Decision, "chose sqlite over postgres"))
            .await
            .unwrap();
        store
            .write_memory(MemoryRecord::new(WorkspaceId::new(), MemoryKind::ProjectFact, "other workspace fact"))
            .await
            .unwrap();

        let hits = store
            .search_memory(MemoryQuery { workspace_id: Some(ws), kind: Some(MemoryKind::ProjectFact), ..Default::default() })
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record.text, "uses rust and sqlite");
    }

    #[tokio::test]
    async fn search_relevance_ranks_matching_text_higher() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        run_migrations(&db).unwrap();
        let store = SqliteMemoryStore::new(db);
        let ws = WorkspaceId::new();
        store.write_memory(MemoryRecord::new(ws, MemoryKind::ProjectFact, "the sky is blue")).await.unwrap();
        store.write_memory(MemoryRecord::new(ws, MemoryKind::ProjectFact, "rust is a systems language")).await.unwrap();

        let hits = store
            .search_memory(MemoryQuery { workspace_id: Some(ws), text_contains: Some("rust".into()), ..Default::default() })
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].record.text.contains("rust"));
    }
}
