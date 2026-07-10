//! SQLite connection management. See spec `06-context-memory-state.md` "Storage".

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;
use sakha_core::{SakhaError, SakhaResult};

/// Owns a `rusqlite::Connection` behind a mutex so stores can be shared
/// across async tasks (blocking calls are expected to run in a blocking
/// thread pool at the call site per spec threading model).
pub struct Database {
    conn: Mutex<Connection>,
    path: Option<PathBuf>,
}

impl Database {
    /// Opens (or creates) a SQLite database file at `path` and runs any
    /// pending migrations, bringing the schema up to date.
    pub fn open(path: impl AsRef<Path>) -> SakhaResult<Self> {
        let path_buf = path.as_ref().to_path_buf();
        let conn = Connection::open(&path_buf)
            .map_err(|e| SakhaError::integrity("sakha-memory", "failed to open database").with_cause(e))?;
        let db = Self { conn: Mutex::new(conn), path: Some(path_buf) };
        crate::migrations::run_migrations(&db)?;
        Ok(db)
    }

    /// Opens an in-memory database and runs any pending migrations. Useful
    /// for tests and ephemeral sessions.
    pub fn open_in_memory() -> SakhaResult<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| SakhaError::integrity("sakha-memory", "failed to open in-memory database").with_cause(e))?;
        let db = Self { conn: Mutex::new(conn), path: None };
        crate::migrations::run_migrations(&db)?;
        Ok(db)
    }

    /// Opens a database file without running migrations. Used by tests that
    /// want to inspect a pre-migration database, or by callers that want to
    /// control migration timing explicitly.
    pub fn open_without_migrations(path: impl AsRef<Path>) -> SakhaResult<Self> {
        let path_buf = path.as_ref().to_path_buf();
        let conn = Connection::open(&path_buf)
            .map_err(|e| SakhaError::integrity("sakha-memory", "failed to open database").with_cause(e))?;
        Ok(Self { conn: Mutex::new(conn), path: Some(path_buf) })
    }

    /// The filesystem path backing this database, if any (`None` for
    /// in-memory databases).
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Runs `f` with exclusive access to the underlying connection.
    pub fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> SakhaResult<T> {
        let conn = self.conn.lock().map_err(|_| SakhaError::integrity("sakha-memory", "db mutex poisoned"))?;
        f(&conn).map_err(|e| SakhaError::integrity("sakha-memory", "sqlite operation failed").with_cause(e))
    }
}
