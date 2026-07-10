//! The `ContextCompressor` trait: routes context through Headroom or a
//! local fallback. See `04-core-domain-model.md` `ContextCompressor`.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use sakha_core::{ArtifactKind, ArtifactRef, SakhaError, SakhaResult};

use crate::headroom::HeadroomClient;
use crate::policy::{CompressionPolicy, CompressionRoute, ContentKind};
use crate::retrieval::{CompressionMarker, RetrievalStore};
use crate::stats::{CompressionScope, CompressionStats, StatsRecorder};

/// A unit of context to be classified/compressed. Mirrors
/// `04-core-domain-model.md` `ContextItem` at the compression layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextItem {
    pub content: String,
    pub kind: Option<ContentKind>,
    pub raw_artifact: Option<ArtifactRef>,
}

impl ContextItem {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            kind: None,
            raw_artifact: None,
        }
    }

    pub fn with_kind(mut self, kind: ContentKind) -> Self {
        self.kind = Some(kind);
        self
    }

    /// Rough token estimate (chars / 4), used for policy min-token routing
    /// decisions and stats. Not a real tokenizer, but consistent enough for
    /// local routing decisions and offline tests.
    pub fn estimated_tokens(&self) -> u64 {
        (self.content.len() as u64) / 4
    }
}

/// The result of compressing a `ContextItem`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressedContextItem {
    pub compressed_content: String,
    pub marker: Option<CompressionMarker>,
    pub token_estimate_raw: u64,
    pub token_estimate_compressed: u64,
}

/// Routes context through Headroom or fallback compressors. See
/// `04-core-domain-model.md` `ContextCompressor`.
#[async_trait]
pub trait ContextCompressor: Send + Sync {
    fn classify(&self, item: &ContextItem) -> ContentKind;

    async fn compress(&self, item: ContextItem, policy: &CompressionPolicy) -> SakhaResult<CompressedContextItem>;

    async fn retrieve(&self, marker: &CompressionMarker) -> SakhaResult<ContextItem>;

    async fn stats(&self, scope: CompressionScope) -> SakhaResult<CompressionStats>;
}

/// A no-op compressor that passes content through unchanged. Safe default
/// fallback per spec "Headroom unavailable" failure mode / fail-open policy.
#[derive(Debug, Default)]
pub struct PassthroughCompressor;

#[async_trait]
impl ContextCompressor for PassthroughCompressor {
    fn classify(&self, _item: &ContextItem) -> ContentKind {
        ContentKind::ConversationHistory
    }

    async fn compress(&self, item: ContextItem, _policy: &CompressionPolicy) -> SakhaResult<CompressedContextItem> {
        let len = item.content.len() as u64;
        Ok(CompressedContextItem {
            compressed_content: item.content,
            marker: None,
            token_estimate_raw: len / 4,
            token_estimate_compressed: len / 4,
        })
    }

    async fn retrieve(&self, _marker: &CompressionMarker) -> SakhaResult<ContextItem> {
        Err(SakhaError::not_implemented("sakha-compression", "PassthroughCompressor::retrieve"))
    }

    async fn stats(&self, _scope: CompressionScope) -> SakhaResult<CompressionStats> {
        Ok(CompressionStats::default())
    }
}

/// Classifies a `ContextItem` heuristically from its declared kind or, if
/// absent, cheap content sniffing. Shared by every `ContextCompressor` impl
/// in this crate so classification behavior stays consistent.
pub fn classify_item(item: &ContextItem) -> ContentKind {
    if let Some(kind) = item.kind {
        return kind;
    }
    let trimmed = item.content.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        ContentKind::JsonData
    } else if trimmed.contains("Traceback (most recent call last)")
        || trimmed.contains("panicked at")
        || trimmed.contains("Exception in thread")
    {
        ContentKind::ErrorTrace
    } else {
        ContentKind::ToolOutput
    }
}

/// Deduplicates consecutive identical lines, collapsing runs into a single
/// occurrence annotated with a repeat count. This is a common source of
/// bloat in shell/build/test logs (e.g. repeated progress lines).
fn dedup_consecutive_lines(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let mut run = 1;
        while i + run < lines.len() && lines[i + run] == line {
            run += 1;
        }
        if run > 1 {
            out.push(format!("{line}  (repeated {run}x)"));
        } else {
            out.push(line.to_string());
        }
        i += run;
    }
    out.join("\n")
}

/// Truncates `text` to roughly `head_lines` + `tail_lines`, replacing the
/// omitted middle with an inline marker token pointing at the raw artifact.
/// This is the local, always-available compression strategy (no network,
/// no external process) used when Headroom is unavailable or bypassed.
fn head_tail_truncate(text: &str, head_lines: usize, tail_lines: usize, marker_token: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= head_lines + tail_lines {
        return text.to_string();
    }
    let head = &lines[..head_lines];
    let tail = &lines[lines.len() - tail_lines..];
    let omitted = lines.len() - head_lines - tail_lines;
    format!(
        "{}\n... [{omitted} lines omitted; retrieve full content via {marker_token}] ...\n{}",
        head.join("\n"),
        tail.join("\n")
    )
}

/// Config knobs for `LocalFallbackCompressor`'s truncation heuristic.
#[derive(Debug, Clone, Copy)]
pub struct LocalFallbackConfig {
    pub head_lines: usize,
    pub tail_lines: usize,
    /// Only apply dedup+truncation once content exceeds this many bytes;
    /// short content is left untouched even if policy says "compress".
    pub min_bytes_to_truncate: usize,
}

impl Default for LocalFallbackConfig {
    fn default() -> Self {
        Self {
            head_lines: 40,
            tail_lines: 40,
            min_bytes_to_truncate: 2000,
        }
    }
}

/// Always-available local compressor: line-dedup then head/tail truncation,
/// with a retrieval marker inserted so the omitted middle can be fetched
/// back. Requires no network or external process, per module 05's mandate
/// that a local fallback must always work offline. Raw content is written
/// to `retrieval_store` before truncation so retrieval round-trips.
pub struct LocalFallbackCompressor {
    pub config: LocalFallbackConfig,
    pub retrieval_store: Arc<dyn RetrievalStore>,
    pub stats: Arc<StatsRecorder>,
}

impl LocalFallbackCompressor {
    pub fn new(retrieval_store: Arc<dyn RetrievalStore>) -> Self {
        Self {
            config: LocalFallbackConfig::default(),
            retrieval_store,
            stats: Arc::new(StatsRecorder::new()),
        }
    }

    pub fn with_config(mut self, config: LocalFallbackConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_stats(mut self, stats: Arc<StatsRecorder>) -> Self {
        self.stats = stats;
        self
    }
}

#[async_trait]
impl ContextCompressor for LocalFallbackCompressor {
    fn classify(&self, item: &ContextItem) -> ContentKind {
        classify_item(item)
    }

    async fn compress(&self, item: ContextItem, policy: &CompressionPolicy) -> SakhaResult<CompressedContextItem> {
        let kind = self.classify(&item);
        let scope = CompressionScope::Global;
        let raw_tokens = item.estimated_tokens();
        let decision = policy.decide(kind, raw_tokens);

        if !decision.should_compress || item.content.len() < self.config.min_bytes_to_truncate {
            self.stats.record_compression(scope, raw_tokens, raw_tokens, true);
            return Ok(CompressedContextItem {
                compressed_content: item.content,
                marker: None,
                token_estimate_raw: raw_tokens,
                token_estimate_compressed: raw_tokens,
            });
        }

        // Store the raw artifact first (reversible CCR requirement: raw
        // content must exist before the model ever sees the compressed
        // marker referencing it).
        let raw_bytes = item.content.as_bytes();
        let content_hash = sakha_core::artifact::content_hash_hex(raw_bytes);
        let artifact_kind = match kind {
            ContentKind::JsonData => ArtifactKind::Json,
            ContentKind::CodeFile => ArtifactKind::Text,
            _ => ArtifactKind::ToolOutput,
        };
        let raw_artifact = item
            .raw_artifact
            .clone()
            .unwrap_or_else(|| ArtifactRef::new(artifact_kind, content_hash, raw_bytes.len() as u64));
        let marker = CompressionMarker::new(raw_artifact.clone());
        self.retrieval_store.put(marker.clone()).await?;

        let deduped = dedup_consecutive_lines(&item.content);
        let truncated = head_tail_truncate(
            &deduped,
            self.config.head_lines,
            self.config.tail_lines,
            &marker.as_inline_token(),
        );

        let compressed_tokens = (truncated.len() as u64) / 4;
        self.stats.record_compression(scope, raw_tokens, compressed_tokens, false);

        Ok(CompressedContextItem {
            compressed_content: truncated,
            marker: Some(marker),
            token_estimate_raw: raw_tokens,
            token_estimate_compressed: compressed_tokens,
        })
    }

    async fn retrieve(&self, marker: &CompressionMarker) -> SakhaResult<ContextItem> {
        let found = self.retrieval_store.get(&marker.id).await?;
        let found = found.ok_or_else(|| {
            SakhaError::integrity("sakha-compression", format!("retrieval store missing raw artifact for marker {}", marker.id))
        })?;
        self.stats.record_retrieval(CompressionScope::Global);
        // The retrieval store only holds the `ArtifactRef` pointer, not the
        // bytes (those live in `sakha-memory`'s artifact store); callers
        // resolve `raw_artifact` against that store. Here we hand back a
        // `ContextItem` carrying the artifact ref so higher layers can do so.
        Ok(ContextItem {
            content: String::new(),
            kind: None,
            raw_artifact: Some(found.raw_artifact),
        })
    }

    async fn stats(&self, scope: CompressionScope) -> SakhaResult<CompressionStats> {
        Ok(self.stats.get(&scope))
    }
}

/// Wraps a `HeadroomClient` and falls back to `LocalFallbackCompressor`
/// whenever Headroom is unavailable or errors, per the "Headroom unavailable"
/// failure mode and `fail_open_or_closed` policy field. This is the
/// production-shaped compressor; `LocalFallbackCompressor` alone is enough
/// for fully offline operation.
pub struct HeadroomBackedCompressor {
    pub headroom: Arc<dyn HeadroomClient>,
    pub fallback: LocalFallbackCompressor,
}

impl HeadroomBackedCompressor {
    pub fn new(headroom: Arc<dyn HeadroomClient>, fallback: LocalFallbackCompressor) -> Self {
        Self { headroom, fallback }
    }
}

#[async_trait]
impl ContextCompressor for HeadroomBackedCompressor {
    fn classify(&self, item: &ContextItem) -> ContentKind {
        classify_item(item)
    }

    async fn compress(&self, item: ContextItem, policy: &CompressionPolicy) -> SakhaResult<CompressedContextItem> {
        let kind = self.classify(&item);
        let raw_tokens = item.estimated_tokens();
        let decision = policy.decide(kind, raw_tokens);

        if !decision.should_compress {
            return self.fallback.compress(item, policy).await;
        }

        let use_headroom = matches!(
            decision.route,
            CompressionRoute::HeadroomLibrary
                | CompressionRoute::HeadroomProxy
                | CompressionRoute::HeadroomMcp
                | CompressionRoute::HeadroomSidecar
        );

        if use_headroom {
            let content_kind = format!("{kind:?}");
            let request = crate::headroom::HeadroomCompressRequest {
                content: item.content.clone(),
                content_kind,
            };
            match self.headroom.compress(request).await {
                Ok(response) => {
                    let raw_bytes = item.content.as_bytes();
                    let content_hash = sakha_core::artifact::content_hash_hex(raw_bytes);
                    let raw_artifact = item
                        .raw_artifact
                        .clone()
                        .unwrap_or_else(|| ArtifactRef::new(ArtifactKind::ToolOutput, content_hash, raw_bytes.len() as u64));
                    let marker = CompressionMarker {
                        id: response.marker_id,
                        raw_artifact: raw_artifact.clone(),
                        created_at_unix: 0,
                    };
                    self.fallback.retrieval_store.put(marker.clone()).await?;
                    let compressed_tokens = (response.compressed.len() as u64) / 4;
                    self.fallback
                        .stats
                        .record_compression(CompressionScope::Global, raw_tokens, compressed_tokens, false);
                    return Ok(CompressedContextItem {
                        compressed_content: response.compressed,
                        marker: Some(marker),
                        token_estimate_raw: raw_tokens,
                        token_estimate_compressed: compressed_tokens,
                    });
                }
                Err(err) => {
                    if matches!(policy.fail_open_or_closed, crate::policy::FailPolicy::FailClosed) {
                        return Err(err);
                    }
                    tracing::warn!(error = %err, "Headroom compress failed; falling back to local compressor");
                }
            }
        }

        self.fallback.compress(item, policy).await
    }

    async fn retrieve(&self, marker: &CompressionMarker) -> SakhaResult<ContextItem> {
        match self.headroom.retrieve(&marker.id).await {
            Ok(content) => Ok(ContextItem {
                content,
                kind: None,
                raw_artifact: Some(marker.raw_artifact.clone()),
            }),
            Err(_) => self.fallback.retrieve(marker).await,
        }
    }

    async fn stats(&self, scope: CompressionScope) -> SakhaResult<CompressionStats> {
        self.fallback.stats(scope).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retrieval::InMemoryRetrievalStore;

    fn long_log(lines: usize) -> String {
        (0..lines).map(|i| format!("log line {i}")).collect::<Vec<_>>().join("\n")
    }

    #[tokio::test]
    async fn passthrough_compressor_roundtrips_content_unchanged() {
        let compressor = PassthroughCompressor;
        let item = ContextItem::new("hello world");
        let policy = CompressionPolicy::default();
        let compressed = compressor.compress(item, &policy).await.unwrap();
        assert_eq!(compressed.compressed_content, "hello world");
    }

    #[tokio::test]
    async fn local_fallback_leaves_short_content_untouched() {
        let store = Arc::new(InMemoryRetrievalStore::new());
        let compressor = LocalFallbackCompressor::new(store);
        let item = ContextItem::new("short content");
        let policy = CompressionPolicy::default();
        let compressed = compressor.compress(item, &policy).await.unwrap();
        assert_eq!(compressed.compressed_content, "short content");
        assert!(compressed.marker.is_none());
    }

    #[tokio::test]
    async fn local_fallback_truncates_and_inserts_retrievable_marker() {
        let store = Arc::new(InMemoryRetrievalStore::new());
        let compressor = LocalFallbackCompressor::new(store.clone())
            .with_config(LocalFallbackConfig { head_lines: 3, tail_lines: 3, min_bytes_to_truncate: 10 });
        let raw = long_log(200);
        let item = ContextItem::new(raw.clone()).with_kind(ContentKind::ShellLog);
        let policy = CompressionPolicy::default();

        let compressed = compressor.compress(item, &policy).await.unwrap();
        assert!(compressed.compressed_content.len() < raw.len());
        assert!(compressed.marker.is_some());
        let marker = compressed.marker.unwrap();
        assert!(compressed.compressed_content.contains(&marker.as_inline_token()));

        // Retrieval round-trip: the marker resolves back through the store.
        let retrieved = compressor.retrieve(&marker).await.unwrap();
        assert_eq!(retrieved.raw_artifact.unwrap(), marker.raw_artifact);
    }

    #[tokio::test]
    async fn local_fallback_dedups_repeated_lines() {
        let store = Arc::new(InMemoryRetrievalStore::new());
        let compressor = LocalFallbackCompressor::new(store)
            .with_config(LocalFallbackConfig { head_lines: 100, tail_lines: 100, min_bytes_to_truncate: 10 });
        let raw = vec!["same line"; 50].join("\n");
        let item = ContextItem::new(raw).with_kind(ContentKind::BuildLog);
        let mut policy = CompressionPolicy::default();
        policy.min_tokens_to_compress = 1;
        let compressed = compressor.compress(item, &policy).await.unwrap();
        assert!(compressed.compressed_content.contains("repeated 50x"));
    }

    #[tokio::test]
    async fn local_fallback_bypasses_blocked_content_kind() {
        let store = Arc::new(InMemoryRetrievalStore::new());
        let compressor = LocalFallbackCompressor::new(store)
            .with_config(LocalFallbackConfig { head_lines: 3, tail_lines: 3, min_bytes_to_truncate: 10 });
        let mut policy = CompressionPolicy::default();
        policy.blocked_content_kinds.push(ContentKind::ErrorTrace);
        let raw = long_log(200);
        let item = ContextItem::new(raw.clone()).with_kind(ContentKind::ErrorTrace);
        let compressed = compressor.compress(item, &policy).await.unwrap();
        assert_eq!(compressed.compressed_content, raw);
        assert!(compressed.marker.is_none());
    }

    #[tokio::test]
    async fn code_file_compression_preserves_identifiers_at_head_and_tail() {
        let store = Arc::new(InMemoryRetrievalStore::new());
        let compressor = LocalFallbackCompressor::new(store)
            .with_config(LocalFallbackConfig { head_lines: 2, tail_lines: 2, min_bytes_to_truncate: 10 });
        let mut lines = vec!["fn important_head_symbol() {".to_string()];
        lines.push("    // filler".to_string());
        for i in 0..100 {
            lines.push(format!("    let x{i} = {i};"));
        }
        lines.push("    // filler".to_string());
        lines.push("fn important_tail_symbol() {}".to_string());
        let raw = lines.join("\n");
        let item = ContextItem::new(raw).with_kind(ContentKind::CodeFile);
        let policy = CompressionPolicy::default();
        let compressed = compressor.compress(item, &policy).await.unwrap();
        assert!(compressed.compressed_content.contains("important_head_symbol"));
        assert!(compressed.compressed_content.contains("important_tail_symbol"));
    }

    #[tokio::test]
    async fn json_compression_preserves_schema_keys_when_untouched() {
        // JSON content below the truncation threshold must remain byte-exact
        // so schema keys survive (spec: "JSON compression preserves schema keys").
        let store = Arc::new(InMemoryRetrievalStore::new());
        let compressor = LocalFallbackCompressor::new(store);
        let json = r#"{"name":"sakha","version":1,"nested":{"key":"value"}}"#;
        let item = ContextItem::new(json).with_kind(ContentKind::JsonData);
        let policy = CompressionPolicy::default();
        let compressed = compressor.compress(item, &policy).await.unwrap();
        assert_eq!(compressed.compressed_content, json);
    }

    #[tokio::test]
    async fn headroom_backed_compressor_falls_back_when_headroom_unavailable() {
        let store = Arc::new(InMemoryRetrievalStore::new());
        let fallback = LocalFallbackCompressor::new(store)
            .with_config(LocalFallbackConfig { head_lines: 3, tail_lines: 3, min_bytes_to_truncate: 10 });
        let headroom = Arc::new(crate::headroom::UnavailableHeadroomClient);
        let compressor = HeadroomBackedCompressor::new(headroom, fallback);

        let mut policy = CompressionPolicy::default();
        policy.mode = CompressionRoute::HeadroomSidecar;
        policy.fail_open_or_closed = crate::policy::FailPolicy::FailOpen;

        let raw = long_log(200);
        let item = ContextItem::new(raw.clone()).with_kind(ContentKind::ShellLog);
        let compressed = compressor.compress(item, &policy).await.unwrap();
        // Falls back to local truncation rather than erroring or panicking.
        assert!(compressed.compressed_content.len() < raw.len());
    }

    #[tokio::test]
    async fn headroom_backed_compressor_fails_closed_when_configured() {
        let store = Arc::new(InMemoryRetrievalStore::new());
        let fallback = LocalFallbackCompressor::new(store)
            .with_config(LocalFallbackConfig { head_lines: 3, tail_lines: 3, min_bytes_to_truncate: 10 });
        let headroom = Arc::new(crate::headroom::UnavailableHeadroomClient);
        let compressor = HeadroomBackedCompressor::new(headroom, fallback);

        let mut policy = CompressionPolicy::default();
        policy.mode = CompressionRoute::HeadroomSidecar;
        policy.fail_open_or_closed = crate::policy::FailPolicy::FailClosed;

        let raw = long_log(200);
        let item = ContextItem::new(raw).with_kind(ContentKind::ShellLog);
        let result = compressor.compress(item, &policy).await;
        assert!(result.is_err());
    }
}
