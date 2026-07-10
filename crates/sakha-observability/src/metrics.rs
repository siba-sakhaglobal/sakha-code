//! Metric recording: counters/gauges for tokens, latency, failure rates. See
//! spec `modules/15-observability-audit.md` "Metrics".

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// A single named metric sample.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSample {
    pub name: String,
    pub value: f64,
    pub labels: Vec<(String, String)>,
}

/// Collects metric samples in-memory. A real exporter (OpenTelemetry, etc.)
/// can wrap or replace this later.
#[derive(Default)]
pub struct MetricRecorder {
    samples: Mutex<Vec<MetricSample>>,
}

/// Recovers from a poisoned mutex by taking the inner data anyway.
///
/// Metrics recording must never bring down the caller: even if some other
/// thread panicked while holding the lock, the (possibly partially updated,
/// but still structurally valid `Vec`) data is still usable, so we recover
/// it via `into_inner` rather than propagating the panic to every future
/// metrics call.
fn recover<'a, T>(result: Result<std::sync::MutexGuard<'a, T>, std::sync::PoisonError<std::sync::MutexGuard<'a, T>>>) -> std::sync::MutexGuard<'a, T> {
    match result {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl MetricRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, name: impl Into<String>, value: f64, labels: Vec<(String, String)>) {
        recover(self.samples.lock()).push(MetricSample { name: name.into(), value, labels });
    }

    pub fn snapshot(&self) -> Vec<MetricSample> {
        recover(self.samples.lock()).clone()
    }

    /// Sums all recorded values for a given metric name.
    pub fn total(&self, name: &str) -> f64 {
        recover(self.samples.lock()).iter().filter(|s| s.name == name).map(|s| s.value).sum()
    }

    pub fn grouped_by_label(&self, name: &str, label_key: &str) -> HashMap<String, f64> {
        let mut out = HashMap::new();
        for sample in recover(self.samples.lock()).iter().filter(|s| s.name == name) {
            if let Some((_, value)) = sample.labels.iter().find(|(k, _)| k == label_key) {
                *out.entry(value.clone()).or_insert(0.0) += sample.value;
            }
        }
        out
    }
}
