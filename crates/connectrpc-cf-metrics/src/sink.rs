//! The `MetricSink` trait + a no-op implementation for tests.
//!
//! Consumer wires the Cloudflare Analytics Engine binding in ~10 LOC:
//!
//! ```rust,ignore
//! use async_trait::async_trait;
//! use connectrpc_cf_metrics::MetricSink;
//! use worker::analytics_engine::{AnalyticsEngineDataPoint, BlobType};
//! use std::sync::Arc;
//!
//! pub struct AeMetricSink(pub Arc<worker::AnalyticsEngineDataset>);
//!
//! #[async_trait]
//! impl MetricSink for AeMetricSink {
//!     async fn counter(&self, name: &str, value: u64, labels: &[(&str, &str)]) {
//!         // AE schema: blobs[0]=metric_name, blobs[1..]=label values,
//!         //            doubles[0]=value, indexes[0]=cardinality key.
//!         let mut point = AnalyticsEngineDataPoint::new();
//!         point.add_blob(name);
//!         for (_, v) in labels { point.add_blob(*v); }
//!         point.add_double(value as f64);
//!         let _ = self.0.write_data_point(&point);
//!     }
//!
//!     async fn histogram(&self, name: &str, value: f64, labels: &[(&str, &str)]) {
//!         let mut point = AnalyticsEngineDataPoint::new();
//!         point.add_blob(name);
//!         for (_, v) in labels { point.add_blob(*v); }
//!         point.add_double(value);
//!         let _ = self.0.write_data_point(&point);
//!     }
//! }
//! ```
//!
//! AE's `write_data_point` is itself synchronous — the `async fn` here
//! exists so non-CF hosts can wire async sinks (Prometheus pushgateway,
//! OTLP collector). On a CF Worker the async block resolves
//! immediately with no .await suspend point.

use async_trait::async_trait;
use std::fmt;

/// Sink for per-RPC metrics. Consumer implements this wrapping their
/// Analytics Engine binding (or any other sink — Prometheus,
/// OpenTelemetry, statsd, log line, …).
#[async_trait]
pub trait MetricSink: Send + Sync + 'static {
    /// Increment a counter by `value`. Typical use: count requests,
    /// errors, retries.
    async fn counter(&self, name: &str, value: u64, labels: &[(&str, &str)]);

    /// Record a value into a histogram. Typical use: latencies in ms,
    /// payload sizes in bytes.
    async fn histogram(&self, name: &str, value: f64, labels: &[(&str, &str)]);
}

/// Sink that drops every metric. Useful for tests and dev where AE
/// isn't provisioned.
#[derive(Clone, Debug, Default)]
pub struct NoopSink;

impl NoopSink {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl MetricSink for NoopSink {
    async fn counter(&self, _name: &str, _value: u64, _labels: &[(&str, &str)]) {}
    async fn histogram(&self, _name: &str, _value: f64, _labels: &[(&str, &str)]) {}
}

impl fmt::Display for NoopSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NoopSink")
    }
}
