//! Content-addressed artifact storage: raw bytes on disk/SQLite blob,
//! deduplicated by content hash. See `04-core-domain-model.md` `ArtifactRef`.

use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::OptionalExtension;

use sakha_core::{ArtifactId, ArtifactKind, ArtifactRef, SakhaError, SakhaResult};

use crate::db::Database;

/// Stores and retrieves artifact bytes referenced by `ArtifactRef`.
#[async_trait]
pub trait ArtifactStore: Send + Sync {
    async fn put(&self, kind: ArtifactKind, bytes: Vec<u8>, label: Option<String>) -> SakhaResult<ArtifactRef>;
    async fn get(&self, artifact_ref: &ArtifactRef) -> SakhaResult<Option<Vec<u8>>>;
    async fn delete(&self, artifact_ref: &ArtifactRef) -> SakhaResult<()>;
}

/// An in-memory `ArtifactStore` keyed by content hash for dedup, matching the
/// hashing scheme in `sakha_core::artifact::content_hash_hex`.
#[derive(Default)]
pub struct InMemoryArtifactStore {
    blobs: std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>,
}

impl InMemoryArtifactStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ArtifactStore for InMemoryArtifactStore {
    async fn put(&self, kind: ArtifactKind, bytes: Vec<u8>, label: Option<String>) -> SakhaResult<ArtifactRef> {
        let hash = sakha_core::artifact::content_hash_hex(&bytes);
        let byte_len = bytes.len() as u64;
        self.blobs.lock().unwrap().insert(hash.clone(), bytes);
        let mut artifact = ArtifactRef::new(kind, hash, byte_len);
        if let Some(label) = label {
            artifact = artifact.with_label(label);
        }
        Ok(artifact)
    }

    async fn get(&self, artifact_ref: &ArtifactRef) -> SakhaResult<Option<Vec<u8>>> {
        Ok(self.blobs.lock().unwrap().get(&artifact_ref.content_hash).cloned())
    }

    async fn delete(&self, artifact_ref: &ArtifactRef) -> SakhaResult<()> {
        self.blobs.lock().unwrap().remove(&artifact_ref.content_hash);
        Ok(())
    }
}

fn kind_as_str(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Text => "text",
        ArtifactKind::Json => "json",
        ArtifactKind::Diff => "diff",
        ArtifactKind::ToolOutput => "tool_output",
        ArtifactKind::ModelResponse => "model_response",
        ArtifactKind::Log => "log",
        ArtifactKind::Binary => "binary",
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn kind_from_str(s: &str) -> SakhaResult<ArtifactKind> {
    Ok(match s {
        "text" => ArtifactKind::Text,
        "json" => ArtifactKind::Json,
        "diff" => ArtifactKind::Diff,
        "tool_output" => ArtifactKind::ToolOutput,
        "model_response" => ArtifactKind::ModelResponse,
        "log" => ArtifactKind::Log,
        "binary" => ArtifactKind::Binary,
        other => return Err(SakhaError::integrity("sakha-memory", format!("unknown artifact kind: {other}"))),
    })
}

/// SQLite-backed `ArtifactStore`. Rows are keyed by content hash (the
/// `artifacts.content_hash` primary key), so `put`-ing identical bytes twice
/// dedups to one row: the second `put` overwrites metadata (kind/label) but
/// the hash — and therefore the returned `ArtifactRef.content_hash` — is
/// unchanged, matching `InMemoryArtifactStore`'s dedup contract.
pub struct SqliteArtifactStore {
    db: Arc<Database>,
}

impl SqliteArtifactStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl ArtifactStore for SqliteArtifactStore {
    async fn put(&self, kind: ArtifactKind, bytes: Vec<u8>, label: Option<String>) -> SakhaResult<ArtifactRef> {
        let hash = sakha_core::artifact::content_hash_hex(&bytes);
        let byte_len = bytes.len() as u64;

        // Reuse the existing artifact_id if this content hash already exists,
        // so repeated `put`s of identical bytes are true no-ops for identity.
        let existing_id: Option<String> = self.db.with_conn(|conn| {
            conn.query_row(
                "SELECT artifact_id FROM artifacts WHERE content_hash = ?1",
                rusqlite::params![hash],
                |row| row.get(0),
            )
            .optional()
        })?;

        let artifact_id = match &existing_id {
            Some(id) => id.clone(),
            None => ArtifactId::new().to_string(),
        };

        self.db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO artifacts (content_hash, artifact_id, kind, byte_len, label, bytes, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(content_hash) DO UPDATE SET
                    kind = excluded.kind,
                    label = excluded.label",
                rusqlite::params![
                    hash,
                    artifact_id,
                    kind_as_str(kind),
                    byte_len as i64,
                    label,
                    bytes,
                    sakha_core::time::now_utc().to_rfc3339(),
                ],
            )
        })?;

        let mut artifact = ArtifactRef::new(kind, hash, byte_len);
        artifact.id = artifact_id
            .parse::<uuid::Uuid>()
            .map(ArtifactId::from_uuid)
            .unwrap_or_else(|_| ArtifactId::new());
        if let Some(label) = label {
            artifact = artifact.with_label(label);
        }
        Ok(artifact)
    }

    async fn get(&self, artifact_ref: &ArtifactRef) -> SakhaResult<Option<Vec<u8>>> {
        self.db.with_conn(|conn| {
            conn.query_row(
                "SELECT bytes FROM artifacts WHERE content_hash = ?1",
                rusqlite::params![artifact_ref.content_hash],
                |row| row.get(0),
            )
            .optional()
        })
    }

    async fn delete(&self, artifact_ref: &ArtifactRef) -> SakhaResult<()> {
        self.db.with_conn(|conn| {
            conn.execute(
                "DELETE FROM artifacts WHERE content_hash = ?1",
                rusqlite::params![artifact_ref.content_hash],
            )
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod sqlite_tests {
    use super::*;
    use crate::migrations::run_migrations;

    #[tokio::test]
    async fn dedups_identical_bytes_by_content_hash() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        run_migrations(&db).unwrap();
        let store = SqliteArtifactStore::new(db);

        let a = store.put(ArtifactKind::Text, b"same content".to_vec(), None).await.unwrap();
        let b = store.put(ArtifactKind::Text, b"same content".to_vec(), Some("label".into())).await.unwrap();
        assert_eq!(a.content_hash, b.content_hash);

        let count: i64 = store
            .db
            .with_conn(|conn| conn.query_row("SELECT COUNT(*) FROM artifacts", [], |row| row.get(0)))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn get_returns_stored_bytes() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        run_migrations(&db).unwrap();
        let store = SqliteArtifactStore::new(db);
        let artifact = store.put(ArtifactKind::Log, b"log line".to_vec(), None).await.unwrap();
        let bytes = store.get(&artifact).await.unwrap();
        assert_eq!(bytes, Some(b"log line".to_vec()));
    }

    #[tokio::test]
    async fn delete_removes_artifact() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        run_migrations(&db).unwrap();
        let store = SqliteArtifactStore::new(db);
        let artifact = store.put(ArtifactKind::Text, b"to delete".to_vec(), None).await.unwrap();
        store.delete(&artifact).await.unwrap();
        assert!(store.get(&artifact).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn kind_roundtrips_through_all_variants() {
        for kind in [
            ArtifactKind::Text,
            ArtifactKind::Json,
            ArtifactKind::Diff,
            ArtifactKind::ToolOutput,
            ArtifactKind::ModelResponse,
            ArtifactKind::Log,
            ArtifactKind::Binary,
        ] {
            assert_eq!(kind_from_str(kind_as_str(kind)).unwrap(), kind);
        }
    }
}
