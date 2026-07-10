//! Compression statistics for a scope (session, turn, or loop).

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Maps a poisoned-mutex error to a non-panicking fallback: rather than
/// propagating a prior panic to every future caller of `record`/`get`, log
/// and fall back to an empty snapshot / no-op record. Compression stats are
/// diagnostic, not correctness-critical, so fail-open here rather than
/// panicking during compression's hot path.
fn on_poisoned(what: &str) {
    tracing::error!(what, "StatsRecorder mutex poisoned by a prior panic; continuing with degraded stats");
}

/// What scope `CompressionStats` are being reported for.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum CompressionScope {
    Session(sakha_core::SessionId),
    Turn(sakha_core::TurnId),
    Loop(sakha_core::LoopId),
    /// Aggregate across every scope tracked so far. Convenient for global
    /// dashboards / the compression diff viewer without needing every id.
    Global,
}

/// Aggregate compression metrics. See `04-core-domain-model.md`
/// `Turn.compression_stats` and spec metrics.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct CompressionStats {
    pub items_compressed: u64,
    pub items_bypassed: u64,
    pub raw_tokens: u64,
    pub compressed_tokens: u64,
    pub retrieval_count: u64,
    /// Number of times an over-compression detector flagged a result and
    /// triggered a retry with lower compression (spec "Tests": over-compression
    /// triggers retry with lower compression).
    pub over_compression_retries: u64,
}

impl CompressionStats {
    pub fn tokens_saved(&self) -> u64 {
        self.raw_tokens.saturating_sub(self.compressed_tokens)
    }

    pub fn compression_ratio(&self) -> f64 {
        if self.raw_tokens == 0 {
            return 0.0;
        }
        self.compressed_tokens as f64 / self.raw_tokens as f64
    }

    /// Retrieval rate: retrievals per compressed item. Used by the
    /// "retrieval-rate metric" sakha-specific addition — a high rate hints
    /// compression is too aggressive (over-compression).
    pub fn retrieval_rate(&self) -> f64 {
        if self.items_compressed == 0 {
            return 0.0;
        }
        self.retrieval_count as f64 / self.items_compressed as f64
    }

    pub fn merge(mut self, other: CompressionStats) -> Self {
        self.items_compressed += other.items_compressed;
        self.items_bypassed += other.items_bypassed;
        self.raw_tokens += other.raw_tokens;
        self.compressed_tokens += other.compressed_tokens;
        self.retrieval_count += other.retrieval_count;
        self.over_compression_retries += other.over_compression_retries;
        self
    }
}

/// Thread-safe accumulator of `CompressionStats` keyed by scope, plus a
/// running `Global` total. `ContextCompressor` impls record into this as
/// they compress/retrieve so `stats()` has something real to return.
#[derive(Debug, Default)]
pub struct StatsRecorder {
    by_scope: Mutex<HashMap<CompressionScope, CompressionStats>>,
}

impl StatsRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    fn record(&self, scope: CompressionScope, delta: CompressionStats) {
        let mut map = match self.by_scope.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                on_poisoned("record");
                poisoned.into_inner()
            }
        };
        let entry = map.entry(scope.clone()).or_default();
        *entry = entry.merge(delta);
        // Avoid double-counting when the caller already recorded directly
        // against the `Global` scope.
        if scope != CompressionScope::Global {
            let global = map.entry(CompressionScope::Global).or_default();
            *global = global.merge(delta);
        }
    }

    pub fn record_compression(&self, scope: CompressionScope, raw_tokens: u64, compressed_tokens: u64, bypassed: bool) {
        let delta = if bypassed {
            CompressionStats {
                items_bypassed: 1,
                raw_tokens,
                compressed_tokens: raw_tokens,
                ..Default::default()
            }
        } else {
            CompressionStats {
                items_compressed: 1,
                raw_tokens,
                compressed_tokens,
                ..Default::default()
            }
        };
        self.record(scope, delta);
    }

    pub fn record_retrieval(&self, scope: CompressionScope) {
        self.record(
            scope,
            CompressionStats {
                retrieval_count: 1,
                ..Default::default()
            },
        );
    }

    pub fn record_over_compression_retry(&self, scope: CompressionScope) {
        self.record(
            scope,
            CompressionStats {
                over_compression_retries: 1,
                ..Default::default()
            },
        );
    }

    pub fn get(&self, scope: &CompressionScope) -> CompressionStats {
        let map = match self.by_scope.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                on_poisoned("get");
                poisoned.into_inner()
            }
        };
        map.get(scope).copied().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorder_tracks_per_scope_and_global_totals() {
        let recorder = StatsRecorder::new();
        let session = CompressionScope::Session(sakha_core::SessionId::new());
        recorder.record_compression(session.clone(), 1000, 200, false);
        recorder.record_compression(session.clone(), 500, 500, true);
        recorder.record_retrieval(session.clone());

        let stats = recorder.get(&session);
        assert_eq!(stats.items_compressed, 1);
        assert_eq!(stats.items_bypassed, 1);
        assert_eq!(stats.raw_tokens, 1500);
        assert_eq!(stats.compressed_tokens, 700);
        assert_eq!(stats.retrieval_count, 1);

        let global = recorder.get(&CompressionScope::Global);
        assert_eq!(global.items_compressed, 1);
        assert_eq!(global.raw_tokens, 1500);
    }

    #[test]
    fn compression_ratio_and_tokens_saved_are_correct() {
        let stats = CompressionStats {
            items_compressed: 1,
            raw_tokens: 1000,
            compressed_tokens: 250,
            ..Default::default()
        };
        assert_eq!(stats.tokens_saved(), 750);
        assert!((stats.compression_ratio() - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn retrieval_rate_is_zero_with_no_compressed_items() {
        let stats = CompressionStats::default();
        assert_eq!(stats.retrieval_rate(), 0.0);
    }
}
