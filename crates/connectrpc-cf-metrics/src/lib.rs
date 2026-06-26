//! Per-RPC metrics to Cloudflare Analytics Engine (or any sink).
//!
//! ## What this crate does
//!
//! Two surfaces, both emitting the same two metrics through the same
//! consumer-implemented [`MetricSink`]:
//!
//! - [`MetricsLayer`] — transparent `tower::Layer`. Reads `procedure`
//!   from `req.uri().path()`; status from the HTTP response status. Use
//!   in a plain `tower` stack, or before envelope decode.
//! - [`MetricsInterceptor`] — `connectrpc::Interceptor` (connectrpc 0.7). Reads
//!   `procedure` from `Spec::procedure` and status from the typed
//!   `Result<_, ConnectError>`. Preferred for connectrpc services — see
//!   the `interceptor` module docs for why it's strictly better there.
//!
//! Metrics emitted (both surfaces):
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
//! ## Layer and Interceptor
//!
//! The `connectrpc::Interceptor` trait (connectrpc 0.7) is the natural
//! fit for per-RPC metrics (sees `Spec::procedure`, derives status from
//! the typed result), and ships here as [`MetricsInterceptor`] (module
//! `interceptor`) — same crate, same [`MetricSink`], so switching
//! surfaces doesn't touch your sink impl. The Layer stays for
//! non-connectrpc `tower` stacks.
//!
//! ## Cross-platform timing
//!
//! Uses `web_time::Instant` instead of `std::time::Instant` because the
//! latter doesn't compile on `wasm32-unknown-unknown` (JS has no
//! monotonic clock; `web_time` aliases to `performance.now()` on web,
//! falls through to `std::time::Instant` on native).

pub mod interceptor;
pub mod layer;
pub mod sink;

pub use interceptor::MetricsInterceptor;

pub use layer::{MetricsFuture, MetricsLayer, MetricsService};
pub use sink::{MetricSink, NoopSink};
