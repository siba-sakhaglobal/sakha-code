//! `ResearchStore`: deduplicated research memory (sources, evidence)
//! persisted across a goal's lifetime. See spec `modules/11-web-search-research.md`.

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use sakha_core::{GoalId, SakhaError, SakhaResult};

use crate::db::Database;

/// A single deduplicated research source record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchSourceRecord {
    pub url: String,
    pub title: Option<String>,
    pub fetched_at: chrono::DateTime<chrono::Utc>,
    pub goal_id: Option<GoalId>,
    pub content_summary: Option<String>,
}

/// Stores and deduplicates research sources by URL.
#[async_trait]
pub trait ResearchStore: Send + Sync {
    async fn upsert_source(&self, source: ResearchSourceRecord) -> SakhaResult<()>;
    async fn get_source(&self, url: &str) -> SakhaResult<Option<ResearchSourceRecord>>;
    async fn list_sources_for_goal(&self, goal_id: GoalId) -> SakhaResult<Vec<ResearchSourceRecord>>;
}

/// In-memory `ResearchStore` for tests and as a safe default.
#[derive(Default)]
pub struct InMemoryResearchStore {
    sources: std::sync::Mutex<std::collections::HashMap<String, ResearchSourceRecord>>,
}

impl InMemoryResearchStore {
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
impl ResearchStore for InMemoryResearchStore {
    async fn upsert_source(&self, source: ResearchSourceRecord) -> SakhaResult<()> {
        let mut sources = self.sources.lock().map_err(|_| poison_err("upsert_source"))?;
        sources.insert(source.url.clone(), source);
        Ok(())
    }

    async fn get_source(&self, url: &str) -> SakhaResult<Option<ResearchSourceRecord>> {
        let sources = self.sources.lock().map_err(|_| poison_err("get_source"))?;
        Ok(sources.get(url).cloned())
    }

    async fn list_sources_for_goal(&self, goal_id: GoalId) -> SakhaResult<Vec<ResearchSourceRecord>> {
        let sources = self.sources.lock().map_err(|_| poison_err("list_sources_for_goal"))?;
        Ok(sources.values().filter(|s| s.goal_id == Some(goal_id)).cloned().collect())
    }
}

/// SQLite-backed `ResearchStore`. Sources are deduplicated by URL (primary
/// key); re-fetching a known URL overwrites its title/summary/fetched_at,
/// per the `upsert_source` contract.
pub struct SqliteResearchStore {
    db: Arc<Database>,
}

impl SqliteResearchStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

fn row_to_source(
    url: String,
    title: Option<String>,
    fetched_at: String,
    goal_id: Option<String>,
    content_summary: Option<String>,
) -> SakhaResult<ResearchSourceRecord> {
    Ok(ResearchSourceRecord {
        url,
        title,
        fetched_at: chrono::DateTime::parse_from_rfc3339(&fetched_at)
            .map_err(|e| SakhaError::integrity("sakha-memory", "bad fetched_at").with_cause(e))?
            .with_timezone(&chrono::Utc),
        goal_id: goal_id
            .map(|g| GoalId::from_str(&g).map_err(|e| SakhaError::integrity("sakha-memory", "bad goal id").with_cause(e)))
            .transpose()?,
        content_summary,
    })
}

#[async_trait]
impl ResearchStore for SqliteResearchStore {
    async fn upsert_source(&self, source: ResearchSourceRecord) -> SakhaResult<()> {
        self.db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO research_sources (url, title, fetched_at, goal_id, content_summary)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(url) DO UPDATE SET
                    title = excluded.title,
                    fetched_at = excluded.fetched_at,
                    goal_id = excluded.goal_id,
                    content_summary = excluded.content_summary",
                rusqlite::params![
                    source.url,
                    source.title,
                    source.fetched_at.to_rfc3339(),
                    source.goal_id.map(|g| g.to_string()),
                    source.content_summary,
                ],
            )
        })?;
        Ok(())
    }

    async fn get_source(&self, url: &str) -> SakhaResult<Option<ResearchSourceRecord>> {
        let row: Option<(String, Option<String>, String, Option<String>, Option<String>)> = self.db.with_conn(|conn| {
            conn.query_row(
                "SELECT url, title, fetched_at, goal_id, content_summary FROM research_sources WHERE url = ?1",
                rusqlite::params![url],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .optional()
        })?;
        row.map(|(url, title, fetched_at, goal_id, content_summary)| {
            row_to_source(url, title, fetched_at, goal_id, content_summary)
        })
        .transpose()
    }

    async fn list_sources_for_goal(&self, goal_id: GoalId) -> SakhaResult<Vec<ResearchSourceRecord>> {
        let rows: Vec<(String, Option<String>, String, Option<String>, Option<String>)> = self.db.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT url, title, fetched_at, goal_id, content_summary FROM research_sources
                 WHERE goal_id = ?1 ORDER BY fetched_at ASC",
            )?;
            let rows = stmt.query_map(rusqlite::params![goal_id.to_string()], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
            })?;
            rows.collect()
        })?;
        rows.into_iter()
            .map(|(url, title, fetched_at, goal_id, content_summary)| {
                row_to_source(url, title, fetched_at, goal_id, content_summary)
            })
            .collect()
    }
}

#[cfg(test)]
mod sqlite_tests {
    use super::*;
    use crate::migrations::run_migrations;

    #[tokio::test]
    async fn upsert_deduplicates_by_url() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        run_migrations(&db).unwrap();
        let store = SqliteResearchStore::new(db);
        let goal = GoalId::new();

        store
            .upsert_source(ResearchSourceRecord {
                url: "https://example.com".into(),
                title: Some("first".into()),
                fetched_at: sakha_core::time::now_utc(),
                goal_id: Some(goal),
                content_summary: None,
            })
            .await
            .unwrap();
        store
            .upsert_source(ResearchSourceRecord {
                url: "https://example.com".into(),
                title: Some("updated".into()),
                fetched_at: sakha_core::time::now_utc(),
                goal_id: Some(goal),
                content_summary: Some("summary".into()),
            })
            .await
            .unwrap();

        let sources = store.list_sources_for_goal(goal).await.unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].title.as_deref(), Some("updated"));
        assert_eq!(sources[0].content_summary.as_deref(), Some("summary"));
    }

    #[tokio::test]
    async fn get_source_returns_none_when_absent() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        run_migrations(&db).unwrap();
        let store = SqliteResearchStore::new(db);
        assert!(store.get_source("https://nope.example").await.unwrap().is_none());
    }
}
