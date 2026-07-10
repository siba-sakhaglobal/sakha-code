//! Schema migrations for the SQLite state store. See spec
//! `04-core-domain-model.md` "Persistence Tables" and
//! `06-context-memory-state.md` "Storage".
//!
//! Migrations are tracked via SQLite's built-in `user_version` pragma rather
//! than a separate metadata table, per the task spec. Each `Migration` is a
//! forward-only SQL batch; `run_migrations` applies any migration whose
//! `version` is greater than the database's current `user_version`, in
//! ascending order, and is idempotent (safe to call on every startup).

use sakha_core::{SakhaError, SakhaResult};

use crate::db::Database;

/// A single forward-only migration step.
pub struct Migration {
    pub version: u32,
    pub description: &'static str,
    pub sql: &'static str,
}

/// The ordered list of migrations to bring a fresh database up to date.
///
/// Tables (subset of `04-core-domain-model.md` "Persistence Tables" relevant
/// to this crate): `sessions`, `turns`, `events`, `artifacts`, `handoffs`,
/// `research_sources`, `memory_records`.
pub fn all_migrations() -> Vec<Migration> {
    vec![Migration {
        version: 1,
        description: "create sessions, turns, events, artifacts, handoffs, research_sources, memory_records",
        sql: r#"
            CREATE TABLE IF NOT EXISTS sessions (
                id              TEXT PRIMARY KEY,
                workspace_id    TEXT NOT NULL,
                goal_id         TEXT,
                status          TEXT NOT NULL,
                provider_profile TEXT,
                created_at      TEXT NOT NULL,
                updated_at      TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS turns (
                id           TEXT PRIMARY KEY,
                session_id   TEXT NOT NULL REFERENCES sessions(id),
                input_text   TEXT NOT NULL,
                output_text  TEXT,
                created_at   TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_turns_session_id ON turns(session_id);

            CREATE TABLE IF NOT EXISTS events (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id   TEXT NOT NULL,
                turn_id      TEXT,
                seq          INTEGER NOT NULL,
                kind         TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                created_at   TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_events_session_id ON events(session_id, seq);

            CREATE TABLE IF NOT EXISTS artifacts (
                content_hash TEXT PRIMARY KEY,
                artifact_id  TEXT NOT NULL,
                kind         TEXT NOT NULL,
                byte_len     INTEGER NOT NULL,
                label        TEXT,
                bytes        BLOB NOT NULL,
                created_at   TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS handoffs (
                goal_id                TEXT PRIMARY KEY,
                objective              TEXT NOT NULL,
                current_status         TEXT NOT NULL,
                completed_work_json    TEXT NOT NULL,
                pending_work_json      TEXT NOT NULL,
                files_changed_json     TEXT NOT NULL,
                commands_run_json      TEXT NOT NULL,
                test_results_json      TEXT NOT NULL,
                decisions_made_json    TEXT NOT NULL,
                blockers_json          TEXT NOT NULL,
                next_suggested_action  TEXT NOT NULL,
                compression_notes      TEXT,
                written_at             TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS research_sources (
                url             TEXT PRIMARY KEY,
                title           TEXT,
                fetched_at      TEXT NOT NULL,
                goal_id         TEXT,
                content_summary TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_research_sources_goal_id ON research_sources(goal_id);

            CREATE TABLE IF NOT EXISTS memory_records (
                id           TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL,
                kind         TEXT NOT NULL,
                text         TEXT NOT NULL,
                created_at   TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_memory_records_workspace_kind ON memory_records(workspace_id, kind);
        "#,
    }]
}

/// Applies all pending migrations to `db`, tracked via the SQLite
/// `user_version` pragma. Idempotent: safe to call on every startup — only
/// migrations with `version > current_version` are applied, and each
/// migration's DDL uses `IF NOT EXISTS` guards as a second layer of safety.
pub fn run_migrations(db: &Database) -> SakhaResult<()> {
    let current = current_version(db)?;
    let mut migrations = all_migrations();
    migrations.sort_by_key(|m| m.version);

    for migration in migrations.into_iter().filter(|m| m.version > current) {
        db.with_conn(|conn| conn.execute_batch(migration.sql)).map_err(|e| {
            SakhaError::integrity(
                "sakha-memory",
                format!("migration {} ({}) failed", migration.version, migration.description),
            )
            .with_cause(e.to_string())
        })?;
        set_version(db, migration.version)?;
    }
    Ok(())
}

/// Returns the current schema version recorded in the database's
/// `user_version` pragma, or 0 for a fresh database.
pub fn current_version(db: &Database) -> SakhaResult<u32> {
    db.with_conn(|conn| conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0)))
        .map(|v| v as u32)
}

fn set_version(db: &Database, version: u32) -> SakhaResult<()> {
    db.with_conn(|conn| conn.execute_batch(&format!("PRAGMA user_version = {version};")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_database_has_version_zero_before_migrations_run() {
        // `Database::open_in_memory`/`open` run migrations eagerly, so this
        // test opens a database file directly without migrating to observe
        // the true pre-migration state.
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open_without_migrations(dir.path().join("fresh.sqlite3")).unwrap();
        assert_eq!(current_version(&db).unwrap(), 0);
    }

    #[test]
    fn running_migrations_sets_version_to_latest() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open_without_migrations(dir.path().join("migrate.sqlite3")).unwrap();
        run_migrations(&db).unwrap();
        let latest = all_migrations().into_iter().map(|m| m.version).max().unwrap();
        assert_eq!(current_version(&db).unwrap(), latest);
    }

    #[test]
    fn running_migrations_twice_is_idempotent() {
        let db = Database::open_in_memory().unwrap();
        run_migrations(&db).unwrap();
        run_migrations(&db).unwrap();
        let latest = all_migrations().into_iter().map(|m| m.version).max().unwrap();
        assert_eq!(current_version(&db).unwrap(), latest);
    }

    #[test]
    fn migration_creates_expected_tables() {
        let db = Database::open_in_memory().unwrap();
        run_migrations(&db).unwrap();
        let names: Vec<String> = db
            .with_conn(|conn| {
                let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                rows.collect()
            })
            .unwrap();
        for expected in [
            "sessions",
            "turns",
            "events",
            "artifacts",
            "handoffs",
            "research_sources",
            "memory_records",
        ] {
            assert!(names.iter().any(|n| n == expected), "missing table {expected}");
        }
    }
}
