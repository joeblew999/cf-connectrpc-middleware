//! Per-RPC metrics to Cloudflare Analytics Engine (or any sink).
//!
//! ## What this crate does
//!
//! Provides [`MetricsLayer`] — a transparent `tower::Layer` that times
//! every Connect-RPC call and emits two metrics per request via a
//! consumer-implemented [`MetricSink`]:
//!
//! - **Counter** `rpc_requests_total` — labels `procedure`, `status_class`
//!   (`2xx` / `4xx` / `5xx`).
//! - **Histogram** `rpc_latency_ms` — same labels, value in milliseconds.
//!
//! Pairs with `connectrpc-cf-tracing` — the tracing layer gives you the
//! qualitative log (per-request spans), metrics give you the quantitative
//! dashboard (counters + histograms over time). Together they close the
//! CF Workers observability story.
//!
//! ## CF Workers compatibility
//!
//! - **Builds on `wasm32-unknown-unknown`**: yes.
//! - **CF binding required**: Analytics Engine. Provision in
//!   `wrangler.toml`:
//!   ```toml
//!   [[analytics_engine_datasets]]
//!   binding = "AE"
//!   dataset = "your_dataset"
//!   ```
//! - **Crate-level `worker` dep**: none. Consumer implements
//!   [`MetricSink`] wrapping `env.AE.write_data_point(...)`.
//!
//! ## Why transparent Layer (not Interceptor)
//!
//! The `connectrpc::Interceptor` trait is the natural fit for per-RPC
//! metrics (sees `Spec::procedure`, `Spec::stream_type`, …) but it
//! **does not exist in published `connectrpc 0.4.2`** — only on the
//! upstream `main` branch (see MIDDLEWARES.md §3). Until Interceptor
//! ships in a release, this crate stays on the stable `tower::Layer`
//! surface and reads procedure from `req.uri().path()` — same approach
//! as `connectrpc-cf-tracing`.
//!
//! When Interceptor lands in a release, we'll publish a sibling crate
//! `connectrpc-cf-metrics-interceptor` with `Spec`-aware labels.
//! The two will share the [`MetricSink`] trait so consumers don't
//! rewrite their sink impl.
//!
//! ## Cross-platform timing
//!
//! Uses `web_time::Instant` instead of `std::time::Instant` because the
//! latter doesn't compile on `wasm32-unknown-unknown` (JS has no
//! monotonic clock; `web_time` aliases to `performance.now()` on web,
//! falls through to `std::time::Instant` on native).

pub mod layer;
pub mod sink;

pub use layer::{MetricsFuture, MetricsLayer, MetricsService};
pub use sink::{MetricSink, NoopSink};
