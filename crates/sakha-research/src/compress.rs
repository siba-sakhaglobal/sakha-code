//! Compresses fetched/extracted documents before they enter an
//! `EvidencePack`, via `sakha-compression`'s `ContextCompressor`. See spec
//! `modules/11-web-search-research.md` Research Loop step 8 "Compress
//! documents" and Implementation Tasks item 6 "Integrate Headroom
//! compression for fetched docs".

use std::sync::Arc;

use sakha_compression::{CompressedContextItem, CompressionPolicy, ContentKind, ContextCompressor, ContextItem};
use sakha_core::SakhaResult;

use crate::extract::ExtractedDocument;

/// A document whose `text` has been routed through the configured
/// `ContextCompressor` (Headroom or the local fallback), alongside the
/// compression outcome (token counts, retrieval marker if truncated).
#[derive(Debug, Clone)]
pub struct CompressedDocument {
    pub url: String,
    pub title: Option<String>,
    /// Compressed (possibly unchanged, if under the policy's
    /// `min_tokens_to_compress` threshold) text, safe to fold into an
    /// evidence pack / prompt context.
    pub text: String,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub compression: CompressedContextItem,
}

/// Runs `doc.text` through `compressor` under `policy`, classified as
/// `ContentKind::WebPage`. Never fails the research loop over a compression
/// error: any error from the compressor is treated the same as "bypass" (the
/// document text passes through uncompressed), matching the "Headroom
/// unavailable" fail-open failure mode from module 05 — a compression outage
/// must not block research from making progress.
pub async fn compress_document(
    doc: ExtractedDocument,
    compressor: &dyn ContextCompressor,
    policy: &CompressionPolicy,
) -> SakhaResult<CompressedDocument> {
    let item = ContextItem::new(doc.text.clone()).with_kind(ContentKind::WebPage);
    let raw_tokens = item.estimated_tokens();

    let compression = match compressor.compress(item, policy).await {
        Ok(compressed) => compressed,
        Err(_) => CompressedContextItem {
            compressed_content: doc.text.clone(),
            marker: None,
            token_estimate_raw: raw_tokens,
            token_estimate_compressed: raw_tokens,
        },
    };

    Ok(CompressedDocument {
        url: doc.url,
        title: doc.title,
        text: compression.compressed_content.clone(),
        published_at: doc.published_at,
        compression,
    })
}

/// Compresses a batch of extracted documents, per Research Loop step 8
/// (applied to every fetched/extracted doc before evidence pack assembly).
pub async fn compress_documents(
    docs: Vec<ExtractedDocument>,
    compressor: &dyn ContextCompressor,
    policy: &CompressionPolicy,
) -> SakhaResult<Vec<CompressedDocument>> {
    let mut out = Vec::with_capacity(docs.len());
    for doc in docs {
        out.push(compress_document(doc, compressor, policy).await?);
    }
    Ok(out)
}

/// Convenience: builds a `LocalFallbackCompressor` backed by an in-memory
/// retrieval store, for callers (research loop, tests) that don't need a
/// live Headroom backend. Always available offline, per module 05's mandate.
pub fn default_compressor() -> Arc<dyn ContextCompressor> {
    Arc::new(sakha_compression::LocalFallbackCompressor::new(Arc::new(
        sakha_compression::InMemoryRetrievalStore::new(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::FetchedPage;
    use crate::extract::{Extractor, PlainTextExtractor};

    fn long_page() -> FetchedPage {
        let body: String = (0..500).map(|i| format!("line {i} of fetched documentation content\n")).collect();
        FetchedPage {
            url: "https://example.com/docs".into(),
            status: 200,
            content_type: Some("text/plain".into()),
            body,
            fetched_at: sakha_core::time::now_utc(),
            truncated: false,
        }
    }

    #[tokio::test]
    async fn large_document_is_compressed_and_smaller() {
        let doc = PlainTextExtractor.extract(&long_page()).unwrap();
        let raw_len = doc.text.len();
        let compressor = default_compressor();
        let mut policy = CompressionPolicy::default();
        policy.min_tokens_to_compress = 10;

        let compressed = compress_document(doc, compressor.as_ref(), &policy).await.unwrap();
        assert!(compressed.text.len() < raw_len);
        assert!(compressed.compression.marker.is_some());
        assert_eq!(compressed.url, "https://example.com/docs");
    }

    #[tokio::test]
    async fn small_document_passes_through_unchanged() {
        let page = FetchedPage {
            url: "https://example.com/short".into(),
            status: 200,
            content_type: Some("text/plain".into()),
            body: "short doc".into(),
            fetched_at: sakha_core::time::now_utc(),
            truncated: false,
        };
        let doc = PlainTextExtractor.extract(&page).unwrap();
        let compressor = default_compressor();
        let policy = CompressionPolicy::default();

        let compressed = compress_document(doc, compressor.as_ref(), &policy).await.unwrap();
        assert_eq!(compressed.text, "short doc");
        assert!(compressed.compression.marker.is_none());
    }

    #[tokio::test]
    async fn compress_documents_preserves_batch_order() {
        let docs = vec![
            crate::extract::ExtractedDocument {
                url: "https://a.example".into(),
                title: None,
                text: "doc a".into(),
                published_at: None,
            },
            crate::extract::ExtractedDocument {
                url: "https://b.example".into(),
                title: None,
                text: "doc b".into(),
                published_at: None,
            },
        ];
        let compressor = default_compressor();
        let policy = CompressionPolicy::default();
        let compressed = compress_documents(docs, compressor.as_ref(), &policy).await.unwrap();
        assert_eq!(compressed.len(), 2);
        assert_eq!(compressed[0].url, "https://a.example");
        assert_eq!(compressed[1].url, "https://b.example");
    }
}
