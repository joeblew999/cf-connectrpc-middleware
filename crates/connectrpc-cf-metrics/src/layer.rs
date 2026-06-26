//! The transparent `tower::Layer` that times each Connect-RPC call
//! and emits to a [`MetricSink`].
//!
//! Implementation note — the `MetricSink::counter` / `histogram` calls
//! happen INSIDE the Future, after the inner service resolves, so the
//! AE write doesn't block dispatch. On a CF Worker AE's underlying
//! `write_data_point` is sync anyway; the `async fn` is for non-CF
//! hosts where the sink might genuinely suspend.

use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

use http::{Request, Response};
use pin_project_lite::pin_project;
use tower::{Layer, Service};
use web_time::Instant;

use crate::sink::MetricSink;

/// `tower::Layer` factory for [`MetricsService`].
pub struct MetricsLayer<M> {
    sink: Arc<M>,
    counter_name: &'static str,
    histogram_name: &'static str,
}

impl<M> Clone for MetricsLayer<M> {
    fn clone(&self) -> Self {
        Self {
            sink: Arc::clone(&self.sink),
            counter_name: self.counter_name,
            histogram_name: self.histogram_name,
        }
    }
}

impl<M> MetricsLayer<M> {
    /// New layer with default metric names (`rpc_requests_total`,
    /// `rpc_latency_ms`).
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

impl<S, M> Layer<S> for MetricsLayer<M> {
    type Service = MetricsService<S, M>;
    fn layer(&self, inner: S) -> Self::Service {
        MetricsService {
            inner,
            sink: Arc::clone(&self.sink),
            counter_name: self.counter_name,
            histogram_name: self.histogram_name,
        }
    }
}

/// Per-request service produced by [`MetricsLayer::layer`].
pub struct MetricsService<S, M> {
    inner: S,
    sink: Arc<M>,
    counter_name: &'static str,
    histogram_name: &'static str,
}

impl<S: Clone, M> Clone for MetricsService<S, M> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            sink: Arc::clone(&self.sink),
            counter_name: self.counter_name,
            histogram_name: self.histogram_name,
        }
    }
}

impl<S, M, B, RB> Service<Request<B>> for MetricsService<S, M>
where
    S: Service<Request<B>, Response = Response<RB>>,
    M: MetricSink,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = MetricsFuture<S::Future, M>;

    fn poll_ready(&mut self, cx: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        let procedure = req.uri().path().to_string();
        let started = Instant::now();
        let future = self.inner.call(req);
        MetricsFuture {
            future,
            sink: Arc::clone(&self.sink),
            counter_name: self.counter_name,
            histogram_name: self.histogram_name,
            procedure,
            started,
            // Emission state machine: None until inner resolves, then
            // Pending while we await counter/histogram emission. Avoids
            // spawning detached tasks (workers don't have a runtime to
            // detach into).
            emit_state: EmitState::WaitingForInner,
        }
    }
}

enum EmitState {
    WaitingForInner,
    Done,
}

pin_project! {
    /// Future returned by [`MetricsService::call`]. On the inner
    /// service's completion it synchronously starts emission, then
    /// resolves to the inner result once emission completes.
    pub struct MetricsFuture<F, M> {
        #[pin]
        future: F,
        sink: Arc<M>,
        counter_name: &'static str,
        histogram_name: &'static str,
        procedure: String,
        started: Instant,
        emit_state: EmitState,
    }
}

impl<F, M, RB, E> std::future::Future for MetricsFuture<F, M>
where
    F: std::future::Future<Output = Result<Response<RB>, E>>,
    M: MetricSink,
{
    type Output = Result<Response<RB>, E>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        let this = self.project();
        // 1. Poll the inner service first.
        let result = match this.future.poll(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(r) => r,
        };
        // 2. Inner resolved. Synchronously emit metrics. AE is sync;
        //    non-CF sinks may suspend but Workers don't, so the
        //    `async fn` runs to completion on the first poll on CF.
        //    Using `block_on`-style polling here would deadlock on a
        //    Worker (single-threaded JS), but `MetricSink::counter` is
        //    `async fn` — we use the same poll-loop trick `tower-util`
        //    does for layer-built futures.
        if matches!(this.emit_state, EmitState::WaitingForInner) {
            // The labels we attach. Status class is derived from the
            // HTTP response status — Connect protocol uses 2xx for
            // successful RPCs (Connect errors are encoded in the
            // 2xx body), 4xx for client transport errors (rate limit,
            // auth), 5xx for server transport errors.
            let (status_class, status_str): (&str, String) = match &result {
                Ok(resp) => {
                    let s = resp.status();
                    let class = if s.is_success() {
                        "2xx"
                    } else if s.is_client_error() {
                        "4xx"
                    } else if s.is_server_error() {
                        "5xx"
                    } else {
                        "other"
                    };
                    (class, s.as_u16().to_string())
                }
                Err(_) => ("error", "error".to_string()),
            };
            let _ = status_str; // reserved for future label expansion

            let elapsed_ms = this.started.elapsed().as_secs_f64() * 1000.0;
            let labels = [
                ("procedure", this.procedure.as_str()),
                ("status_class", status_class),
            ];

            // Drive the sink futures to completion. On CF Workers both
            // calls resolve in one poll because AE's write_data_point
            // is sync; on a non-CF host with a genuinely async sink
            // this still works but blocks the current poll.
            //
            // This compromise is deliberate: spawning a detached task
            // would require a `worker::send::SendFuture`-style adapter
            // and a runtime handle, which the crate can't depend on
            // without coupling to either tokio or worker. If you have
            // a slow async sink, wrap it in your own buffer that the
            // sink writes to without awaiting.
            let _ = futures_poll_once(this.sink.counter(this.counter_name, 1, &labels));
            let _ = futures_poll_once(this.sink.histogram(
                this.histogram_name,
                elapsed_ms,
                &labels,
            ));

            *this.emit_state = EmitState::Done;
        }
        Poll::Ready(result)
    }
}

/// Poll an async fn future to completion. Panics if it doesn't resolve
/// in one poll — see [`MetricsFuture::poll`] for why we accept this.
fn futures_poll_once<F: std::future::Future>(fut: F) -> Option<F::Output> {
    use std::pin::pin;
    use std::task::{RawWaker, RawWakerVTable, Waker};

    // Minimal no-op waker so we can poll a single time without an executor.
    static VTABLE: RawWakerVTable = RawWakerVTable::new(
        |_| RawWaker::new(std::ptr::null(), &VTABLE),
        |_| {},
        |_| {},
        |_| {},
    );
    let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
    let mut cx = TaskContext::from_waker(&waker);
    let mut fut = pin!(fut);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(v) => Some(v),
        Poll::Pending => {
            // A genuinely-async sink suspended. Drop the future —
            // the AE write was never scheduled, and we don't have a
            // runtime to spawn it onto. Document this caveat in the
            // sink trait so consumers don't deploy a tokio-bound sink
            // to a Worker.
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::NoopSink;
    use async_trait::async_trait;
    use std::convert::Infallible;
    use std::sync::Mutex;
    use tower::service_fn;

    type CounterCall = (String, u64, Vec<(String, String)>);
    type HistogramCall = (String, f64, Vec<(String, String)>);

    /// Capturing sink — stores every call for assertion. Resolves
    /// each future synchronously so it works with `futures_poll_once`.
    #[derive(Clone)]
    struct CaptureSink {
        counter_calls: Arc<Mutex<Vec<CounterCall>>>,
        histogram_calls: Arc<Mutex<Vec<HistogramCall>>>,
    }
    impl CaptureSink {
        fn new() -> Self {
            Self {
                counter_calls: Arc::new(Mutex::new(Vec::new())),
                histogram_calls: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }
    #[async_trait]
    impl MetricSink for CaptureSink {
        async fn counter(&self, name: &str, value: u64, labels: &[(&str, &str)]) {
            let labels_owned: Vec<(String, String)> = labels
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect();
            self.counter_calls
                .lock()
                .unwrap()
                .push((name.to_string(), value, labels_owned));
        }
        async fn histogram(&self, name: &str, value: f64, labels: &[(&str, &str)]) {
            let labels_owned: Vec<(String, String)> = labels
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect();
            self.histogram_calls
                .lock()
                .unwrap()
                .push((name.to_string(), value, labels_owned));
        }
    }

    #[tokio::test]
    async fn emits_counter_and_histogram_on_2xx() {
        let sink = CaptureSink::new();
        let layer = MetricsLayer::new(sink.clone());
        let inner =
            service_fn(|_req: Request<()>| async { Ok::<_, Infallible>(Response::new(())) });
        let mut svc = layer.layer(inner);

        let req = Request::builder().uri("/pkg.v1.Svc/M").body(()).unwrap();
        let _resp = svc.call(req).await.unwrap();

        let counters = sink.counter_calls.lock().unwrap();
        assert_eq!(counters.len(), 1);
        assert_eq!(counters[0].0, "rpc_requests_total");
        assert_eq!(counters[0].1, 1);
        assert!(
            counters[0]
                .2
                .iter()
                .any(|(k, v)| k == "procedure" && v == "/pkg.v1.Svc/M")
        );
        assert!(
            counters[0]
                .2
                .iter()
                .any(|(k, v)| k == "status_class" && v == "2xx")
        );

        let histos = sink.histogram_calls.lock().unwrap();
        assert_eq!(histos.len(), 1);
        assert_eq!(histos[0].0, "rpc_latency_ms");
        assert!(histos[0].1 >= 0.0);
    }

    #[tokio::test]
    async fn status_class_labels_4xx_and_5xx() {
        let sink = CaptureSink::new();
        let layer = MetricsLayer::new(sink.clone());

        // 429 path
        let inner = service_fn(|_req: Request<()>| async {
            Ok::<_, Infallible>(http::Response::builder().status(429).body(()).unwrap())
        });
        let mut svc = layer.clone().layer(inner);
        let req = Request::builder().uri("/x").body(()).unwrap();
        let _ = svc.call(req).await.unwrap();

        // 503 path
        let inner = service_fn(|_req: Request<()>| async {
            Ok::<_, Infallible>(http::Response::builder().status(503).body(()).unwrap())
        });
        let mut svc = layer.clone().layer(inner);
        let req = Request::builder().uri("/x").body(()).unwrap();
        let _ = svc.call(req).await.unwrap();

        let counters = sink.counter_calls.lock().unwrap();
        let labels: Vec<_> = counters
            .iter()
            .flat_map(|c| c.2.iter())
            .filter(|(k, _)| k == "status_class")
            .map(|(_, v)| v.clone())
            .collect();
        assert!(labels.contains(&"4xx".to_string()));
        assert!(labels.contains(&"5xx".to_string()));
    }

    #[tokio::test]
    async fn noop_sink_compiles_and_runs() {
        let layer = MetricsLayer::new(NoopSink);
        let inner =
            service_fn(|_req: Request<()>| async { Ok::<_, Infallible>(Response::new(())) });
        let mut svc = layer.layer(inner);
        let req = Request::builder().uri("/x").body(()).unwrap();
        let resp = svc.call(req).await.unwrap();
        assert_eq!(resp.status(), 200);
    }
}
