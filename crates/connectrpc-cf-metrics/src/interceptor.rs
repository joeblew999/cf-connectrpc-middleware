//! `connectrpc::Interceptor` that times each unary RPC and emits to a
//! [`MetricSink`] — the Interceptor-surface sibling of [`MetricsLayer`].
//!
//! Why this exists alongside the Layer (MIDDLEWARES.md §3 flagged it):
//!
//! - **Better `procedure` label.** An interceptor runs after the Connect
//!   envelope is decoded, so it reads the static `Spec::procedure`
//!   (proto-qualified, stable) rather than sniffing `req.uri().path()`.
//! - **Honest `status_class`.** The result is a typed
//!   `Result<UnaryResponse, ConnectError>`; on error we bucket via
//!   `ConnectError::http_status()` instead of inspecting an opaque
//!   200-with-error-in-body Connect response.
//! - **No single-poll hack.** The interceptor method is genuinely
//!   `async`, so it just `.await`s the sink — the Layer's
//!   `futures_poll_once` workaround (needed because a `tower` Future
//!   can't suspend mid-poll on a Worker) disappears entirely.
//!
//! Both surfaces share the same [`MetricSink`], so a consumer keeps their
//! AE wiring when switching from the Layer to this.

use std::sync::Arc;

use connectrpc::{ConnectError, Interceptor, Next, UnaryRequest, UnaryResponse, async_trait};
use web_time::Instant;

use crate::sink::MetricSink;

/// Per-RPC metrics on the `connectrpc::Interceptor` surface. Register on a
/// `ConnectRpcService` with `.with_interceptor(MetricsInterceptor::new(sink))`.
pub struct MetricsInterceptor<M> {
    sink: Arc<M>,
    counter_name: &'static str,
    histogram_name: &'static str,
}

impl<M> Clone for MetricsInterceptor<M> {
    fn clone(&self) -> Self {
        Self {
            sink: Arc::clone(&self.sink),
            counter_name: self.counter_name,
            histogram_name: self.histogram_name,
        }
    }
}

impl<M> MetricsInterceptor<M> {
    /// New interceptor with default metric names (`rpc_requests_total`,
    /// `rpc_latency_ms`) — same defaults as [`MetricsLayer`](crate::MetricsLayer).
    pub fn new(sink: M) -> Self {
        Self {
            sink: Arc::new(sink),
            counter_name: "rpc_requests_total",
            histogram_name: "rpc_latency_ms",
        }
    }

    /// Override the counter metric name. Default: `"rpc_requests_total"`.
    pub fn counter_name(mut self, name: &'static str) -> Self {
        self.counter_name = name;
        self
    }

    /// Override the histogram metric name. Default: `"rpc_latency_ms"`.
    pub fn histogram_name(mut self, name: &'static str) -> Self {
        self.histogram_name = name;
        self
    }
}

#[async_trait]
impl<M: MetricSink> Interceptor for MetricsInterceptor<M> {
    async fn intercept_unary(
        &self,
        req: UnaryRequest,
        next: Next<'_>,
    ) -> Result<UnaryResponse, ConnectError> {
        // Prefer the static Spec procedure (e.g. "/pkg.v1.Svc/Method",
        // proto-qualified); fall back to the dispatch path. Build it before
        // moving `req` into `next.run`.
        let procedure = req
            .ctx
            .spec()
            .map(|s| s.procedure.to_string())
            .or_else(|| req.ctx.path().map(str::to_string))
            .unwrap_or_default();

        let started = Instant::now();
        let result = next.run(req).await;
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

        // 2xx for a successful RPC; otherwise bucket the ConnectError by
        // its mapped HTTP status (PermissionDenied/Unauthenticated/... →
        // 4xx, Internal/Unavailable/... → 5xx).
        let status_class = match &result {
            Ok(_) => "2xx",
            Err(e) => {
                let s = e.http_status();
                if s.is_client_error() {
                    "4xx"
                } else if s.is_server_error() {
                    "5xx"
                } else {
                    "other"
                }
            }
        };

        let labels = [
            ("procedure", procedure.as_str()),
            ("status_class", status_class),
        ];
        self.sink.counter(self.counter_name, 1, &labels).await;
        self.sink
            .histogram(self.histogram_name, elapsed_ms, &labels)
            .await;

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::NoopSink;

    // Compile-level guarantee that MetricsInterceptor satisfies the
    // connectrpc::Interceptor bound. Behavioural verification (labels,
    // status buckets) is covered by the Layer's unit tests against the
    // shared MetricSink and by the example worker integration — `Next`
    // is constructed only by the dispatcher, so it can't be unit-driven.
    #[test]
    fn implements_interceptor() {
        fn assert_interceptor<I: Interceptor>(_: &I) {}
        let i = MetricsInterceptor::new(NoopSink)
            .counter_name("rpc_total")
            .histogram_name("rpc_ms");
        assert_interceptor(&i);
    }
}
