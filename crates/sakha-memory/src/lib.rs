//! sakha-memory: SQLite-backed session, goal/plan, handoff, and research memory stores.
//!
//! Public API skeleton — see spec `modules/06-context-memory-state.md` and
//! `crates/crate-work-breakdown.md`. Store traits are async over `rusqlite`
//! (blocking calls are expected to be dispatched to a blocking thread pool
//! by callers per the spec threading model); in-memory implementations are
//! provided as safe defaults for tests and early wiring.

pub mod artifact_store;
pub mod db;
pub mod handoff;
pub mod memory;
pub mod migrations;
pub mod research;
pub mod session;

pub use artifact_store::{ArtifactStore, InMemoryArtifactStore, SqliteArtifactStore};
pub use db::Database;
pub use handoff::{HandoffArtifact, HandoffStore, InMemoryHandoffStore, SqliteHandoffStore};
pub use memory::{
    InMemoryMemoryStore, MemoryHit, MemoryKind, MemoryQuery, MemoryRecord, MemoryStore,
    RelevanceScore, SqliteMemoryStore, Summary,
};
pub use migrations::{all_migrations, current_version, run_migrations, Migration};
pub use research::{InMemoryResearchStore, ResearchSourceRecord, ResearchStore, SqliteResearchStore};
pub use session::{
    InMemorySessionStore, SessionRecord, SessionStatus, SessionStore, SqliteSessionStore, TurnRecord,
};

pub fn crate_name() -> &'static str {
    "sakha-memory"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_brings_fresh_database_to_latest_version() {
        let db = Database::open_in_memory().unwrap();
        run_migrations(&db).unwrap();
        let latest = all_migrations().into_iter().map(|m| m.version).max().unwrap();
        let version = current_version(&db).unwrap();
        assert_eq!(version, latest);
    }

    #[tokio::test]
    async fn session_resumes_via_get_after_create() {
        let store = InMemorySessionStore::new();
        let session = SessionRecord {
            id: sakha_core::SessionId::new(),
            workspace_id: sakha_core::WorkspaceId::new(),
            goal_id: None,
            status: SessionStatus::Active,
            provider_profile: None,
            created_at: sakha_core::time::now_utc(),
            updated_at: sakha_core::time::now_utc(),
        };
        let id = session.id;
        store.create_session(session).await.unwrap();
        let resumed = store.get_session(id).await.unwrap();
        assert!(resumed.is_some());
    }

    #[tokio::test]
    async fn artifact_dedup_by_content_hash() {
        let store = InMemoryArtifactStore::new();
        let a = store
            .put(sakha_core::ArtifactKind::Text, b"same content".to_vec(), None)
            .await
            .unwrap();
        let b = store
            .put(sakha_core::ArtifactKind::Text, b"same content".to_vec(), None)
            .await
            .unwrap();
        assert_eq!(a.content_hash, b.content_hash);
    }

    #[tokio::test]
    async fn research_source_dedup_by_url() {
        let store = InMemoryResearchStore::new();
        let goal = sakha_core::GoalId::new();
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
                content_summary: None,
            })
            .await
            .unwrap();
        let sources = store.list_sources_for_goal(goal).await.unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].title.as_deref(), Some("updated"));
    }

    #[tokio::test]
    async fn handoff_round_trips() {
        let store = InMemoryHandoffStore::new();
        let goal = sakha_core::GoalId::new();
        let handoff = HandoffArtifact::new(goal, "ship feature x");
        store.write_handoff(goal, handoff).await.unwrap();
        let loaded = store.load_handoff(goal).await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().objective, "ship feature x");
    }

    /// Session resume test using a tempfile-backed SQLite database, per the
    /// task spec. Simulates a process restart: writes a session and turns
    /// through one `Database` handle, drops it, reopens the same file path
    /// through a fresh `Database`/`SqliteSessionStore`, and confirms the
    /// session and its turn history resume intact.
    #[tokio::test]
    async fn session_resumes_from_tempfile_database_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("sakha-memory-test.sqlite3");

        let session_id = sakha_core::SessionId::new();
        let workspace_id = sakha_core::WorkspaceId::new();
        let goal_id = sakha_core::GoalId::new();

        // First "process": create the DB, write a session and two turns.
        {
            let db = std::sync::Arc::new(Database::open(&db_path).unwrap());
            let store = SqliteSessionStore::new(db);
            let session = SessionRecord {
                id: session_id,
                workspace_id,
                goal_id: Some(goal_id),
                status: SessionStatus::Active,
                provider_profile: None,
                created_at: sakha_core::time::now_utc(),
                updated_at: sakha_core::time::now_utc(),
            };
            store.create_session(session).await.unwrap();
            store
                .append_turn(TurnRecord {
                    id: sakha_core::TurnId::new(),
                    session_id,
                    input_text: "hello".into(),
                    output_text: Some("hi there".into()),
                    created_at: sakha_core::time::now_utc(),
                })
                .await
                .unwrap();
            store.update_session_status(session_id, SessionStatus::Paused).await.unwrap();
        }

        // Second "process": reopen the same file path and resume.
        {
            let db = std::sync::Arc::new(Database::open(&db_path).unwrap());
            let store = SqliteSessionStore::new(db);
            let resumed = store.get_session(session_id).await.unwrap().expect("session persisted across reopen");
            assert_eq!(resumed.id, session_id);
            assert_eq!(resumed.workspace_id, workspace_id);
            assert_eq!(resumed.goal_id, Some(goal_id));
            assert_eq!(resumed.status, SessionStatus::Paused);

            let turns = store.list_turns(session_id).await.unwrap();
            assert_eq!(turns.len(), 1);
            assert_eq!(turns[0].input_text, "hello");
            assert_eq!(turns[0].output_text.as_deref(), Some("hi there"));
        }
    }

    /// A handoff written by one loop iteration must carry every field a
    /// fresh agent needs to continue without re-deriving context, per spec
    /// "Handoff Artifact Required Fields".
    #[tokio::test]
    async fn handoff_is_sufficient_for_a_fresh_agent_to_continue() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("handoff-test.sqlite3");
        let db = std::sync::Arc::new(Database::open(&db_path).unwrap());
        let store = SqliteHandoffStore::new(db);
        let goal = sakha_core::GoalId::new();

        let mut handoff = HandoffArtifact::new(goal, "migrate auth to OAuth2");
        handoff.current_status = "in progress".into();
        handoff.completed_work.push("added oauth2 client".into());
        handoff.pending_work.push("wire up refresh token flow".into());
        handoff.files_changed.push("src/auth/oauth2.rs".into());
        handoff.commands_run.push("cargo test -p sakha-security".into());
        handoff.test_results.push("12 passed, 0 failed".into());
        handoff.decisions_made.push("use PKCE flow".into());
        handoff.blockers.push("waiting on client secret rotation".into());
        handoff.next_suggested_action = "implement refresh token handler".into();
        handoff.compression_notes = Some("full detail retained, no compression applied".into());
        store.write_handoff(goal, handoff).await.unwrap();

        let loaded = store.load_handoff(goal).await.unwrap().expect("handoff present");
        // Every required field per spec is non-empty/populated.
        assert!(!loaded.objective.is_empty());
        assert!(!loaded.current_status.is_empty());
        assert!(!loaded.completed_work.is_empty());
        assert!(!loaded.pending_work.is_empty());
        assert!(!loaded.files_changed.is_empty());
        assert!(!loaded.commands_run.is_empty());
        assert!(!loaded.test_results.is_empty());
        assert!(!loaded.decisions_made.is_empty());
        assert!(!loaded.blockers.is_empty());
        assert!(!loaded.next_suggested_action.is_empty());
        assert!(loaded.compression_notes.is_some());
    }
}
