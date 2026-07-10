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

impl MetricRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, name: impl Into<String>, value: f64, labels: Vec<(String, String)>) {
        self.samples.lock().unwrap().push(MetricSample { name: name.into(), value, labels });
    }

    pub fn snapshot(&self) -> Vec<MetricSample> {
        self.samples.lock().unwrap().clone()
    }

    /// Sums all recorded values for a given metric name.
    pub fn total(&self, name: &str) -> f64 {
        self.samples.lock().unwrap().iter().filter(|s| s.name == name).map(|s| s.value).sum()
    }

    pub fn grouped_by_label(&self, name: &str, label_key: &str) -> HashMap<String, f64> {
        let mut out = HashMap::new();
        for sample in self.samples.lock().unwrap().iter().filter(|s| s.name == name) {
            if let Some((_, value)) = sample.labels.iter().find(|(k, _)| k == label_key) {
                *out.entry(value.clone()).or_insert(0.0) += sample.value;
            }
        }
        out
    }
}
