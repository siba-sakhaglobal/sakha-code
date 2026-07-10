//! Retrieval markers and store: reversible compression requires that
//! compressed content embed a stable marker the model can use to request the
//! original back. See spec "Reversible CCR Requirements".

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use sakha_core::{ArtifactRef, SakhaResult};

/// Current UTC time as unix seconds. Local helper since `sakha-core::time`
/// only exposes `DateTime<Utc>`, not a raw unix-seconds accessor.
fn now_unix() -> u64 {
    sakha_core::time::now_utc().timestamp().clamp(0, i64::MAX) as u64
}

/// A stable, embeddable marker id pointing at a raw artifact behind a
/// compressed item. Markers are embedded verbatim in compressed content
/// (e.g. `[[ccr:<uuid>]]`) so a model can request the original back through
/// a retrieval tool. `created_at_unix` and `retrieval_count` support the
/// store-raw-for-days policy field and the retrieval-rate metric.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompressionMarker {
    pub id: String,
    pub raw_artifact: ArtifactRef,
    pub created_at_unix: u64,
}

impl CompressionMarker {
    pub fn new(raw_artifact: ArtifactRef) -> Self {
        Self {
            id: format!("ccr:{}", raw_artifact.id),
            raw_artifact,
            created_at_unix: now_unix(),
        }
    }

    /// Renders the marker as an inline token safe to embed in compressed
    /// text, e.g. `[[ccr:<id>]]`. `extract_marker_ids` finds these again.
    pub fn as_inline_token(&self) -> String {
        format!("[[{}]]", self.id)
    }
}

/// Scans `text` for `[[ccr:...]]` marker tokens and returns their ids in
/// order of appearance. Used by retrieval tools to find which markers a
/// piece of compressed content references.
pub fn extract_marker_ids(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[ccr:") {
        let after = &rest[start + 2..];
        if let Some(end) = after.find("]]") {
            ids.push(after[..end].to_string());
            rest = &after[end + 2..];
        } else {
            break;
        }
    }
    ids
}

/// Abstraction over marker -> raw artifact lookup, backed by SQLite +
/// artifact store in `sakha-memory`. Async since real impls hit disk.
#[async_trait]
pub trait RetrievalStore: Send + Sync {
    async fn put(&self, marker: CompressionMarker) -> SakhaResult<()>;
    async fn get(&self, marker_id: &str) -> SakhaResult<Option<CompressionMarker>>;
    /// Removes markers older than `now_unix - max_age_secs`. Supports the
    /// `store_raw_for_days` policy field. Returns the number of markers
    /// evicted. Default no-op for stores that don't implement retention.
    async fn evict_older_than(&self, _max_age_secs: u64) -> SakhaResult<u64> {
        Ok(0)
    }
    /// Total number of markers currently stored.
    async fn len(&self) -> SakhaResult<usize> {
        Ok(0)
    }
}

/// An in-memory `RetrievalStore` for tests and as a safe default before a
/// SQLite-backed store is wired in.
#[derive(Debug, Default)]
pub struct InMemoryRetrievalStore {
    markers: std::sync::Mutex<std::collections::HashMap<String, CompressionMarker>>,
}

impl InMemoryRetrievalStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl RetrievalStore for InMemoryRetrievalStore {
    async fn put(&self, marker: CompressionMarker) -> SakhaResult<()> {
        self.markers.lock().unwrap().insert(marker.id.clone(), marker);
        Ok(())
    }

    async fn get(&self, marker_id: &str) -> SakhaResult<Option<CompressionMarker>> {
        Ok(self.markers.lock().unwrap().get(marker_id).cloned())
    }

    async fn evict_older_than(&self, max_age_secs: u64) -> SakhaResult<u64> {
        let now = now_unix();
        let mut markers = self.markers.lock().unwrap();
        let before = markers.len();
        markers.retain(|_, marker| now.saturating_sub(marker.created_at_unix) <= max_age_secs);
        Ok((before - markers.len()) as u64)
    }

    async fn len(&self) -> SakhaResult<usize> {
        Ok(self.markers.lock().unwrap().len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact() -> ArtifactRef {
        ArtifactRef::new(sakha_core::ArtifactKind::Text, "hash", 10)
    }

    #[test]
    fn extract_marker_ids_finds_all_tokens_in_order() {
        let marker1 = CompressionMarker::new(artifact());
        let marker2 = CompressionMarker::new(artifact());
        let text = format!(
            "start {} middle {} end",
            marker1.as_inline_token(),
            marker2.as_inline_token()
        );
        let ids = extract_marker_ids(&text);
        assert_eq!(ids, vec![marker1.id.clone(), marker2.id.clone()]);
    }

    #[test]
    fn extract_marker_ids_returns_empty_for_plain_text() {
        assert!(extract_marker_ids("no markers here").is_empty());
    }

    #[tokio::test]
    async fn evict_older_than_removes_stale_markers() {
        let store = InMemoryRetrievalStore::new();
        let mut marker = CompressionMarker::new(artifact());
        marker.created_at_unix = 0; // far in the past
        store.put(marker.clone()).await.unwrap();
        let evicted = store.evict_older_than(1).await.unwrap();
        assert_eq!(evicted, 1);
        assert!(store.get(&marker.id).await.unwrap().is_none());
    }
}
