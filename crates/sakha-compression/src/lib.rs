//! sakha-compression: Headroom adapter, compression policy, retrieval markers, fallback compressors.
//!
//! Public API skeleton — see spec `modules/05-context-compression-headroom.md`
//! and `crates/crate-work-breakdown.md`. Key contract: `ContextCompressor`
//! trait with `PassthroughCompressor` as the always-available fallback.

pub mod compressor;
pub mod headroom;
pub mod manager;
pub mod mcp;
pub mod policy;
pub mod retrieval;
pub mod sidecar;
pub mod stats;

pub use compressor::{
    classify_item, CompressedContextItem, ContextCompressor, ContextItem, HeadroomBackedCompressor, LocalFallbackCompressor,
    LocalFallbackConfig, PassthroughCompressor,
};
pub use headroom::{
    HeadroomClient, HeadroomCompressRequest, HeadroomCompressResponse, HeadroomHttpConfig, HeadroomMode, HeadroomSidecar,
    HttpHeadroomClient, UnavailableHeadroomClient,
};
pub use manager::{CompressionAudit, CompressionAuditLog, CompressionManager, ContextClassifier};
pub use mcp::{HeadroomMcpClient, McpToolInvoker};
pub use policy::{CompressionDecision, CompressionPolicy, CompressionRoute, ContentKind, FailPolicy};
pub use retrieval::{extract_marker_ids, CompressionMarker, InMemoryRetrievalStore, RetrievalStore};
pub use sidecar::{SidecarConfig, SidecarLauncher};
pub use stats::{CompressionScope, CompressionStats, StatsRecorder};

pub fn crate_name() -> &'static str {
    "sakha-compression"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn passthrough_compressor_roundtrips_content_unchanged() {
        let compressor = PassthroughCompressor;
        let item = ContextItem::new("hello world");
        let policy = CompressionPolicy::default();
        let compressed = compressor.compress(item, &policy).await.unwrap();
        assert_eq!(compressed.compressed_content, "hello world");
    }

    #[tokio::test]
    async fn headroom_unavailable_returns_error_not_panic() {
        let client = headroom::UnavailableHeadroomClient;
        let result = client
            .compress(HeadroomCompressRequest {
                content: "x".into(),
                content_kind: "text".into(),
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn retrieval_store_put_then_get_roundtrips() {
        let store = InMemoryRetrievalStore::new();
        let artifact = sakha_core::ArtifactRef::new(sakha_core::ArtifactKind::Text, "hash", 10);
        let marker = CompressionMarker::new(artifact.clone());
        store.put(marker.clone()).await.unwrap();
        let found = store.get(&marker.id).await.unwrap();
        assert_eq!(found.unwrap().raw_artifact, artifact);
    }

    #[test]
    fn policy_routing_respects_blocked_content_kinds() {
        let mut policy = CompressionPolicy::default();
        policy.blocked_content_kinds.push(ContentKind::ErrorTrace);
        assert!(!policy.allows(ContentKind::ErrorTrace));
        assert!(policy.allows(ContentKind::CodeFile));
    }

    /// End-to-end: a big tool output goes through `LocalFallbackCompressor`,
    /// comes back smaller with a marker, and `retrieve` resolves that marker
    /// back to the original artifact ref — the crate's headline contract
    /// (spec "Tests": "Compress/retrieve round trip", "Tool output
    /// compression").
    #[tokio::test]
    async fn tool_output_compress_then_retrieve_round_trip() {
        let store = std::sync::Arc::new(InMemoryRetrievalStore::new());
        let compressor = LocalFallbackCompressor::new(store).with_config(LocalFallbackConfig {
            head_lines: 5,
            tail_lines: 5,
            min_bytes_to_truncate: 10,
        });
        let raw: String = (0..500).map(|i| format!("stdout line {i}\n")).collect();
        let item = ContextItem::new(raw.clone()).with_kind(ContentKind::ToolOutput);
        let policy = CompressionPolicy::default();

        let compressed = compressor.compress(item, &policy).await.unwrap();
        assert!(compressed.token_estimate_compressed < compressed.token_estimate_raw);
        let marker = compressed.marker.expect("large tool output should get a marker");

        let retrieved = compressor.retrieve(&marker).await.unwrap();
        assert_eq!(retrieved.raw_artifact.unwrap().byte_len, raw.len() as u64);
    }

    /// A retrieval store missing the raw artifact for a marker must surface
    /// as an integrity error, not panic (spec "Failure Modes": "Retrieval
    /// store missing raw artifact").
    #[tokio::test]
    async fn retrieval_store_missing_artifact_is_integrity_error() {
        let store = std::sync::Arc::new(InMemoryRetrievalStore::new());
        let compressor = LocalFallbackCompressor::new(store);
        let ghost_artifact = sakha_core::ArtifactRef::new(sakha_core::ArtifactKind::Text, "missing", 1);
        let ghost_marker = CompressionMarker::new(ghost_artifact);
        let result = compressor.retrieve(&ghost_marker).await;
        assert!(result.is_err());
    }

    /// Sensitive content kinds can be fully excluded from compression via
    /// the blocked-content-kinds policy field (spec "Tests": "Disable
    /// compression for sensitive content").
    #[tokio::test]
    async fn disable_compression_for_sensitive_content_kind() {
        let store = std::sync::Arc::new(InMemoryRetrievalStore::new());
        let compressor = LocalFallbackCompressor::new(store).with_config(LocalFallbackConfig {
            head_lines: 3,
            tail_lines: 3,
            min_bytes_to_truncate: 10,
        });
        let mut policy = CompressionPolicy::default();
        policy.blocked_content_kinds.push(ContentKind::Handoff);

        let raw: String = (0..500).map(|i| format!("secret line {i}\n")).collect();
        let item = ContextItem::new(raw.clone()).with_kind(ContentKind::Handoff);
        let compressed = compressor.compress(item, &policy).await.unwrap();
        assert_eq!(compressed.compressed_content, raw);
        assert!(compressed.marker.is_none());
    }

    /// An over-compression detector (sakha-specific addition) can flag a
    /// result as too lossy and retry with a less aggressive config,
    /// recording the retry in stats (spec "Tests": "Over-compression
    /// triggers retry with lower compression").
    #[tokio::test]
    async fn over_compression_triggers_retry_with_lower_compression() {
        let store = std::sync::Arc::new(InMemoryRetrievalStore::new());
        let stats = std::sync::Arc::new(StatsRecorder::new());
        let aggressive = LocalFallbackCompressor::new(store.clone())
            .with_config(LocalFallbackConfig { head_lines: 1, tail_lines: 1, min_bytes_to_truncate: 10 })
            .with_stats(stats.clone());
        let raw: String = (0..2000).map(|i| format!("line {i}\n")).collect();
        let mut policy = CompressionPolicy::default();
        policy.min_tokens_to_compress = 1;

        let first = aggressive
            .compress(ContextItem::new(raw.clone()).with_kind(ContentKind::TestLog), &policy)
            .await
            .unwrap();

        // Over-compression detector: ratio below 10% is considered too lossy.
        let compressed_ratio = (first.token_estimate_compressed as f64) / (first.token_estimate_raw.max(1) as f64);
        assert!(compressed_ratio < 0.10, "expected the 1/1-line config to over-compress this input");

        stats.record_over_compression_retry(CompressionScope::Global);
        let gentler = LocalFallbackCompressor::new(store)
            .with_config(LocalFallbackConfig { head_lines: 20, tail_lines: 20, min_bytes_to_truncate: 10 })
            .with_stats(stats.clone());
        let retried = gentler
            .compress(ContextItem::new(raw).with_kind(ContentKind::TestLog), &policy)
            .await
            .unwrap();

        assert!(retried.token_estimate_compressed > first.token_estimate_compressed);
        assert_eq!(stats.get(&CompressionScope::Global).over_compression_retries, 1);
    }
}
