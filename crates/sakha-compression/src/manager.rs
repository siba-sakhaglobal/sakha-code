//! `CompressionManager`, `ContextClassifier`, `CompressionAudit`: the
//! top-level facade tying classification, policy routing, a
//! `ContextCompressor`, and audit logging together into the "Compression
//! Pipeline" described in spec `modules/05-context-compression-headroom.md`:
//!
//! ```text
//! ContextItem
//!   -> classify content kind
//!   -> determine compression policy
//!   -> Headroom route or local fallback
//!   -> write raw artifact
//!   -> write compressed artifact
//!   -> insert retrieval marker
//!   -> send compressed bundle to model
//! ```

use std::sync::Arc;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use sakha_core::SakhaError;
use sakha_core::SakhaResult;

use crate::compressor::{classify_item, CompressedContextItem, ContextCompressor, ContextItem};
use crate::policy::{CompressionPolicy, ContentKind};
use crate::retrieval::CompressionMarker;
use crate::stats::CompressionScope;

/// Classifies `ContextItem`s into a `ContentKind`, driving compression
/// routing decisions. Thin, stateless wrapper around
/// `compressor::classify_item` so callers have a named type to depend on
/// (spec "Main Structs": `ContextClassifier`) independent of any one
/// `ContextCompressor` implementation's internal classification logic.
#[derive(Debug, Default, Clone, Copy)]
pub struct ContextClassifier;

impl ContextClassifier {
    pub fn new() -> Self {
        Self
    }

    /// Classifies `item`, preferring its declared `kind` and falling back to
    /// cheap content sniffing (JSON/error-trace/generic tool output).
    pub fn classify(&self, item: &ContextItem) -> ContentKind {
        classify_item(item)
    }

    /// Whether `policy` permits compressing content of `kind`.
    pub fn allowed_under(&self, kind: ContentKind, policy: &CompressionPolicy) -> bool {
        policy.allows(kind)
    }
}

/// One audited compression or retrieval event. Reversible CCR requires that
/// "Retrieval is permissioned and audited" (spec "Reversible CCR
/// Requirements"); this is the record of that audit trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionAudit {
    pub kind: AuditEventKind,
    pub content_kind: ContentKind,
    pub marker_id: Option<String>,
    pub timestamp_unix: u64,
}

/// What kind of event a `CompressionAudit` entry records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventKind {
    Compress,
    Retrieve,
    RetrievalDenied,
}

/// Thread-safe, append-only in-memory audit log of `CompressionAudit`
/// entries. A durable (SQLite-backed) log can be layered on later without
/// changing `CompressionManager`'s call sites, since this is consulted only
/// through `record`/`entries`.
#[derive(Debug, Default)]
pub struct CompressionAuditLog {
    entries: Mutex<Vec<CompressionAudit>>,
}

impl CompressionAuditLog {
    pub fn new() -> Self {
        Self::default()
    }

    fn record(&self, entry: CompressionAudit) {
        match self.entries.lock() {
            Ok(mut guard) => guard.push(entry),
            Err(poisoned) => {
                // Auditing must never panic compression's hot path; degrade
                // to a best-effort log rather than propagating a prior panic.
                tracing::error!("CompressionAuditLog mutex poisoned by a prior panic; continuing with degraded audit trail");
                poisoned.into_inner().push(entry);
            }
        }
    }

    /// A snapshot of every audit entry recorded so far, oldest first.
    pub fn entries(&self) -> Vec<CompressionAudit> {
        match self.entries.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

/// Top-level facade for the compression pipeline: classifies content,
/// applies policy, delegates to a `ContextCompressor` (Headroom-backed or
/// local fallback), and records an audit trail for every compress/retrieve
/// call (spec "Main Structs": `CompressionManager`; "Compression Pipeline").
pub struct CompressionManager {
    compressor: Arc<dyn ContextCompressor>,
    classifier: ContextClassifier,
    policy: CompressionPolicy,
    pub audit: Arc<CompressionAuditLog>,
}

impl CompressionManager {
    pub fn new(compressor: Arc<dyn ContextCompressor>, policy: CompressionPolicy) -> Self {
        Self { compressor, classifier: ContextClassifier::new(), policy, audit: Arc::new(CompressionAuditLog::new()) }
    }

    pub fn with_audit_log(mut self, audit: Arc<CompressionAuditLog>) -> Self {
        self.audit = audit;
        self
    }

    pub fn policy(&self) -> &CompressionPolicy {
        &self.policy
    }

    /// Runs `item` through the full compression pipeline: classify, compress
    /// (Headroom route or local fallback per the wrapped `ContextCompressor`),
    /// and record an audit entry.
    pub async fn compress(&self, item: ContextItem) -> SakhaResult<CompressedContextItem> {
        let content_kind = self.classifier.classify(&item);
        let result = self.compressor.compress(item, &self.policy).await?;
        self.audit.record(CompressionAudit {
            kind: AuditEventKind::Compress,
            content_kind,
            marker_id: result.marker.as_ref().map(|m| m.id.clone()),
            timestamp_unix: crate::retrieval::now_unix(),
        });
        Ok(result)
    }

    /// Retrieves the original content behind `marker`, per
    /// `retrieve_on_model_request`. Reversible CCR requires retrieval to be
    /// "permissioned and audited" — when the policy has retrieval disabled,
    /// this records a `RetrievalDenied` audit entry and returns a permission
    /// error rather than silently falling through to the compressor.
    pub async fn retrieve(&self, marker: &CompressionMarker) -> SakhaResult<ContextItem> {
        if !self.policy.retrieve_on_model_request {
            self.audit.record(CompressionAudit {
                kind: AuditEventKind::RetrievalDenied,
                content_kind: ContentKind::ToolOutput,
                marker_id: Some(marker.id.clone()),
                timestamp_unix: crate::retrieval::now_unix(),
            });
            return Err(SakhaError::permission(
                "sakha-compression",
                "retrieval is disabled by policy (retrieve_on_model_request = false)",
            ));
        }
        let result = self.compressor.retrieve(marker).await?;
        self.audit.record(CompressionAudit {
            kind: AuditEventKind::Retrieve,
            content_kind: result.kind.unwrap_or(ContentKind::ToolOutput),
            marker_id: Some(marker.id.clone()),
            timestamp_unix: crate::retrieval::now_unix(),
        });
        Ok(result)
    }

    pub async fn stats(&self, scope: CompressionScope) -> SakhaResult<crate::stats::CompressionStats> {
        self.compressor.stats(scope).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compressor::LocalFallbackCompressor;
    use crate::retrieval::InMemoryRetrievalStore;

    #[test]
    fn context_classifier_respects_declared_kind() {
        let classifier = ContextClassifier::new();
        let item = ContextItem::new("{}").with_kind(ContentKind::Handoff);
        assert_eq!(classifier.classify(&item), ContentKind::Handoff);
    }

    #[test]
    fn context_classifier_allowed_under_respects_blocked_kinds() {
        let classifier = ContextClassifier::new();
        let mut policy = CompressionPolicy::default();
        policy.blocked_content_kinds.push(ContentKind::ErrorTrace);
        assert!(!classifier.allowed_under(ContentKind::ErrorTrace, &policy));
        assert!(classifier.allowed_under(ContentKind::CodeFile, &policy));
    }

    #[tokio::test]
    async fn compression_manager_compress_records_audit_entry() {
        let store = Arc::new(InMemoryRetrievalStore::new());
        let compressor = Arc::new(
            LocalFallbackCompressor::new(store)
                .with_config(crate::compressor::LocalFallbackConfig { head_lines: 3, tail_lines: 3, min_bytes_to_truncate: 10 }),
        );
        let manager = CompressionManager::new(compressor, CompressionPolicy::default());

        let raw: String = (0..500).map(|i| format!("line {i}\n")).collect();
        let item = ContextItem::new(raw).with_kind(ContentKind::ShellLog);
        let compressed = manager.compress(item).await.unwrap();
        assert!(compressed.marker.is_some());

        let entries = manager.audit.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, AuditEventKind::Compress);
        assert_eq!(entries[0].content_kind, ContentKind::ShellLog);
        assert!(entries[0].marker_id.is_some());
    }

    #[tokio::test]
    async fn compression_manager_retrieve_round_trips_and_audits() {
        let store = Arc::new(InMemoryRetrievalStore::new());
        let compressor = Arc::new(
            LocalFallbackCompressor::new(store)
                .with_config(crate::compressor::LocalFallbackConfig { head_lines: 3, tail_lines: 3, min_bytes_to_truncate: 10 }),
        );
        let manager = CompressionManager::new(compressor, CompressionPolicy::default());

        let raw: String = (0..500).map(|i| format!("line {i}\n")).collect();
        let item = ContextItem::new(raw.clone()).with_kind(ContentKind::ShellLog);
        let compressed = manager.compress(item).await.unwrap();
        let marker = compressed.marker.unwrap();

        let retrieved = manager.retrieve(&marker).await.unwrap();
        assert_eq!(retrieved.raw_artifact.unwrap().byte_len, raw.len() as u64);

        let entries = manager.audit.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].kind, AuditEventKind::Retrieve);
    }

    #[tokio::test]
    async fn compression_manager_denies_retrieval_when_policy_disables_it() {
        let store = Arc::new(InMemoryRetrievalStore::new());
        let compressor = Arc::new(
            LocalFallbackCompressor::new(store)
                .with_config(crate::compressor::LocalFallbackConfig { head_lines: 3, tail_lines: 3, min_bytes_to_truncate: 10 }),
        );
        let mut policy = CompressionPolicy::default();
        policy.retrieve_on_model_request = false;
        let manager = CompressionManager::new(compressor, policy);

        let raw: String = (0..500).map(|i| format!("line {i}\n")).collect();
        let item = ContextItem::new(raw).with_kind(ContentKind::ShellLog);
        let compressed = manager.compress(item).await.unwrap();
        let marker = compressed.marker.unwrap();

        let result = manager.retrieve(&marker).await;
        assert!(result.is_err());

        let entries = manager.audit.entries();
        assert_eq!(entries.last().unwrap().kind, AuditEventKind::RetrievalDenied);
    }
}
