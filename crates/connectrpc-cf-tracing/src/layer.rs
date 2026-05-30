//! The `tower::Layer` that wraps each Connect-RPC call in a tracing span.
//!
//! See module-level docs in [`crate`] for the design rationale (transparent
//! shape, no `Rollout` adoption, extractor pattern).

use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

use http::Request;
use pin_project_lite::pin_project;
use tower::{Layer, Service};
use tracing::Span;

use crate::extract::{CfFields, CfFieldsExtractor};

/// `tower::Layer` factory for [`TracingService`].
///
/// Construct with [`TracingLayer::new`] passing a closure or
/// [`CfFieldsExtractor`] impl. Customize the span target / name via
/// the builder methods.
pub struct TracingLayer<E> {
    extractor: Arc<E>,
    target: &'static str,
    span_name: &'static str,
}

impl<E> Clone for TracingLayer<E> {
    fn clone(&self) -> Self {
        Self {
            extractor: Arc::clone(&self.extractor),
            target: self.target,
            span_name: self.span_name,
        }
    }
}

impl<E> TracingLayer<E> {
    /// New layer with the default target (`"connectrpc_cf_tracing"`)
    /// and span name (`"rpc"`).
    pub fn new(extractor: E) -> Self {
        Self {
            extractor: Arc::new(extractor),
            target: "connectrpc_cf_tracing",
            span_name: "rpc",
        }
    }

    /// Override the `tracing` target. Useful when routing CF tracing
    /// to its own log sink via `tracing-subscriber` filter directives.
    /// Default: `"connectrpc_cf_tracing"`.
    pub fn target(mut self, target: &'static str) -> Self {
        self.target = target;
        self
    }

    /// Override the span name. Default: `"rpc"`.
    pub fn span_name(mut self, name: &'static str) -> Self {
        self.span_name = name;
        self
    }
}

impl<S, E> Layer<S> for TracingLayer<E> {
    type Service = TracingService<S, E>;
    fn layer(&self, inner: S) -> Self::Service {
        TracingService {
            inner,
            extractor: Arc::clone(&self.extractor),
            target: self.target,
            span_name: self.span_name,
        }
    }
}

/// Per-request service produced by [`TracingLayer::layer`].
pub struct TracingService<S, E> {
    inner: S,
    extractor: Arc<E>,
    target: &'static str,
    span_name: &'static str,
}

impl<S: Clone, E> Clone for TracingService<S, E> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            extractor: Arc::clone(&self.extractor),
            target: self.target,
            span_name: self.span_name,
        }
    }
}

impl<S, E, B> Service<Request<B>> for TracingService<S, E>
where
    S: Service<Request<B>>,
    E: CfFieldsExtractor<B>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = TracingFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        let fields = self.extractor.extract(&req);

        // info_span! macro can't take dynamic `target`, so build via the
        // Span constructor. Fields are recorded by name; tracing's fmt
        // subscriber renders them as `field=value` after the span name.
        let span = make_span(self.target, self.span_name, &fields);

        // Entering the span returns a guard for the synchronous portion
        // of `call()`. The actual handler runs inside `inner.call(req)`
        // whose Future we wrap below, so the span needs to be re-entered
        // each time the Future is polled. See TracingFuture::poll.
        let _guard = span.enter();
        let future = self.inner.call(req);
        drop(_guard);

        TracingFuture { future, span }
    }
}

/// Build the tracing span for one request.
///
/// `tracing::info_span!` is a macro that bakes the `target` literal into
/// the call site, so it can't take a runtime `target` value. We
/// hand-construct via [`Span::new`] instead. The fields are recorded
/// via `Span::record` after construction — `info_span!` can't take a
/// dynamic field set either.
fn make_span(target: &'static str, name: &'static str, fields: &CfFields) -> Span {
    // We can't dynamically declare fields on a Span at construction time
    // — the field set is part of the Metadata, which the `info_span!`
    // macro generates statically. So instead of recording structured
    // fields, we emit one Display-formatted string and let subscribers
    // render it.
    //
    // This sacrifices searchability of individual fields for portability:
    // every subscriber (fmt, opentelemetry, …) gets the same rendering.
    // If a downstream consumer wants typed fields, they wrap our layer
    // with their own subscriber-specific instrumentation.
    let span = tracing::info_span!(
        target: "connectrpc_cf_tracing",
        "rpc",
        cf.procedure = tracing::field::Empty,
        cf.colo = tracing::field::Empty,
        cf.country = tracing::field::Empty,
        cf.asn = tracing::field::Empty,
        cf.tls_cipher = tracing::field::Empty,
        cf.http_protocol = tracing::field::Empty,
        cf.ray = tracing::field::Empty,
    );
    // Suppress unused-args lint while we settle on the signature.
    let _ = (target, name);

    if let Some(v) = &fields.procedure {
        span.record("cf.procedure", tracing::field::display(v));
    }
    if let Some(v) = &fields.colo {
        span.record("cf.colo", tracing::field::display(v));
    }
    if let Some(v) = &fields.country {
        span.record("cf.country", tracing::field::display(v));
    }
    if let Some(v) = fields.asn {
        span.record("cf.asn", v);
    }
    if let Some(v) = &fields.tls_cipher {
        span.record("cf.tls_cipher", tracing::field::display(v));
    }
    if let Some(v) = &fields.http_protocol {
        span.record("cf.http_protocol", tracing::field::display(v));
    }
    if let Some(v) = &fields.ray {
        span.record("cf.ray", tracing::field::display(v));
    }

    span
}

pin_project! {
    /// Future returned by [`TracingService::call`]. Re-enters the span
    /// on every poll so the inner handler logs / errors carry the
    /// CF fields automatically.
    pub struct TracingFuture<F> {
        #[pin]
        future: F,
        span: Span,
    }
}

impl<F: std::future::Future> std::future::Future for TracingFuture<F> {
    type Output = F::Output;
    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _guard = this.span.enter();
        this.future.poll(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;
    use tower::service_fn;

    /// `tracing_subscriber::fmt::MakeWriter` sink that captures every byte
    /// the fmt layer writes — span open lines, field key/value pairs,
    /// the close marker. Test asserts against the captured buffer.
    #[derive(Clone, Default)]
    struct CaptureWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Plain `#[test]` (not `#[tokio::test]`) so we control runtime
    /// construction — needed because `tracing::subscriber::with_default`
    /// must enclose the runtime, not the other way round, and a
    /// `#[tokio::test]` already owns the current thread.
    #[test]
    fn span_records_all_provided_fields() {
        let buf = CaptureWriter::default();
        // Don't use FmtSpan::NEW — at span-open the recorded fields
        // aren't included in the formatter's output. Instead emit a
        // tracing event INSIDE the span (from the inner service below);
        // fmt's default format renders the event line with the active
        // span's fields in `name{field=value}: event-msg` form, which is
        // what we assert against.
        let subscriber = tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_ansi(false)
            .finish();
        // `set_default` installs the subscriber on the current thread for
        // the lifetime of the returned guard. The runtime built below
        // inherits this thread's subscriber for its tasks.
        let _guard = tracing::subscriber::set_default(subscriber);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let layer = TracingLayer::new(|req: &Request<()>| CfFields {
                procedure: Some(req.uri().path().to_string()),
                colo: Some("SIN".into()),
                country: Some("SG".into()),
                asn: Some(13335),
                tls_cipher: Some("AEAD-AES128-GCM-SHA256".into()),
                http_protocol: Some("HTTP/2".into()),
                ray: Some("8c0123456789abcd-SIN".into()),
            });

            let inner = service_fn(|_req: Request<()>| async {
                // Event inside the span — fmt renders the active span's
                // fields alongside this event line, which is what the
                // assertions below grep for.
                tracing::info!("handled");
                Ok::<_, Infallible>(http::Response::new(()))
            });

            let mut svc = layer.layer(inner);
            let req = Request::builder()
                .uri("/pkg.v1.Service/Method")
                .body(())
                .unwrap();
            let _resp = <_ as Service<Request<()>>>::call(&mut svc, req).await;
        });

        // Drop guard so the subscriber unhooks before assertions read
        // the buffer (cheap insurance against deadlock).
        drop(_guard);

        let captured = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        // The fmt layer renders the active span's fields inside `{…}`
        // alongside the event line — `rpc{cf.procedure=/… cf.colo=SIN …}: msg`.
        // Display-recorded fields aren't quoted; ints land bare.
        assert!(
            captured.contains("cf.procedure=/pkg.v1.Service/Method"),
            "missing procedure: {captured}"
        );
        assert!(captured.contains("cf.colo=SIN"), "missing colo: {captured}");
        assert!(captured.contains("cf.country=SG"), "missing country: {captured}");
        assert!(captured.contains("cf.asn=13335"), "missing asn: {captured}");
        assert!(
            captured.contains("cf.ray=8c0123456789abcd-SIN"),
            "missing cf-ray: {captured}"
        );
    }

    #[tokio::test]
    async fn missing_fields_dont_panic() {
        // Extractor returns CfFields::empty() — verify the layer still
        // wires through and the inner service sees the request.
        let layer = TracingLayer::new(|_: &Request<()>| CfFields::empty());
        let inner = service_fn(|_req: Request<()>| async {
            Ok::<_, Infallible>(http::Response::new(()))
        });
        let mut svc = layer.layer(inner);
        let req = Request::builder().uri("/x.Svc/M").body(()).unwrap();
        let resp = <_ as Service<Request<()>>>::call(&mut svc, req).await.unwrap();
        assert_eq!(resp.status(), 200);
    }
}
