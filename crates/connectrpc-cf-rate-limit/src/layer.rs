//! The short-circuit `tower::Layer` that wraps each request in a
//! rate-limit check.
//!
//! The rate-limit check is async (the CF binding round-trips to the
//! runtime), so unlike `connectrpc-cedar`'s sync `is_authorized`,
//! this layer's `Future` is `BoxFuture` — one `Box::pin` per request.
//! That's a small alloc, but rate limiting is already async-bound and
//! the inner Service is also async, so there's no avoiding it.

use std::convert::Infallible;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

use connectrpc::ConnectError;
use connectrpc_tower_kit::{Rollout, deny_response, log_shadow};
use futures::future::BoxFuture;
use http::Response;
use tower::{Layer, Service};
use tracing::warn;

use crate::key::RateLimitKeyExtractor;
use crate::limiter::{RateLimitOutcome, RateLimiter};
use crate::mode::Mode;

use connectrpc::ConnectRpcBody;

/// `tower::Layer` factory for [`RateLimitService`].
pub struct RateLimitLayer<L, E> {
    limiter: Arc<L>,
    extractor: Arc<E>,
    mode: Mode,
    skip_paths: Arc<Vec<String>>,
}

impl<L, E> Clone for RateLimitLayer<L, E> {
    fn clone(&self) -> Self {
        Self {
            limiter: Arc::clone(&self.limiter),
            extractor: Arc::clone(&self.extractor),
            mode: self.mode,
            skip_paths: Arc::clone(&self.skip_paths),
        }
    }
}

impl<L, E> RateLimitLayer<L, E> {
    /// Observe mode — call the binding, log "would have blocked", but
    /// always pass through. Use this until logs are clean.
    pub fn observe(limiter: L, extractor: E) -> Self {
        Self {
            limiter: Arc::new(limiter),
            extractor: Arc::new(extractor),
            mode: Mode::Observe,
            skip_paths: Arc::new(Vec::new()),
        }
    }

    /// Enforce mode — block on `Exceeded`. Production mode.
    pub fn enforce(limiter: L, extractor: E) -> Self {
        Self {
            limiter: Arc::new(limiter),
            extractor: Arc::new(extractor),
            mode: Mode::Enforce,
            skip_paths: Arc::new(Vec::new()),
        }
    }

    /// Paths the layer skips entirely (no binding call, no log).
    /// Use for health checks and any endpoint that shouldn't be
    /// counted against the limit.
    pub fn skip_paths<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.skip_paths = Arc::new(paths.into_iter().map(Into::into).collect());
        self
    }
}

impl<S, L, E> Layer<S> for RateLimitLayer<L, E> {
    type Service = RateLimitService<S, L, E>;
    fn layer(&self, inner: S) -> Self::Service {
        RateLimitService {
            inner,
            limiter: Arc::clone(&self.limiter),
            extractor: Arc::clone(&self.extractor),
            mode: self.mode,
            skip_paths: Arc::clone(&self.skip_paths),
        }
    }
}

/// Per-request service produced by [`RateLimitLayer::layer`].
pub struct RateLimitService<S, L, E> {
    inner: S,
    limiter: Arc<L>,
    extractor: Arc<E>,
    mode: Mode,
    skip_paths: Arc<Vec<String>>,
}

impl<S: Clone, L, E> Clone for RateLimitService<S, L, E> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            limiter: Arc::clone(&self.limiter),
            extractor: Arc::clone(&self.extractor),
            mode: self.mode,
            skip_paths: Arc::clone(&self.skip_paths),
        }
    }
}

impl<S, L, E, B> Service<http::Request<B>> for RateLimitService<S, L, E>
where
    // Same pinning as connectrpc-cedar — short-circuit means we must
    // be able to construct a Response<ConnectRpcBody> ourselves without
    // touching S, so S has to produce that type.
    S: Service<http::Request<B>, Response = Response<ConnectRpcBody>, Error = Infallible>
        + Send
        + Clone
        + 'static,
    S::Future: Send + 'static,
    L: RateLimiter,
    E: RateLimitKeyExtractor<B>,
    B: Send + 'static,
{
    type Response = Response<ConnectRpcBody>;
    type Error = Infallible;
    type Future = BoxFuture<'static, Result<Response<ConnectRpcBody>, Infallible>>;

    fn poll_ready(&mut self, cx: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<B>) -> Self::Future {
        let path = req.uri().path().to_string();

        if self.skip_paths.iter().any(|p| p == &path) {
            // Don't bother with the binding call — fast-path through.
            return Box::pin(self.inner.call(req));
        }

        let Some(key) = self.extractor.extract(&req) else {
            // No key derivable — fail-open. Common for healthchecks
            // and local dev where cf-connecting-ip is absent.
            return Box::pin(self.inner.call(req));
        };

        // Clone the bits we move into the async block. Cloning Arc is
        // cheap; cloning the inner Service is required because Service::call
        // takes `&mut self` and we need to call it from inside an async move.
        let limiter = Arc::clone(&self.limiter);
        let mode = self.mode;
        let mut inner = self.inner.clone();

        Box::pin(async move {
            let outcome = limiter.check(key.clone()).await;
            match outcome {
                RateLimitOutcome::Allowed => inner.call(req).await,
                RateLimitOutcome::Error(err) => {
                    // Fail-open with a warning. Production rate
                    // limiters degrade to "all requests pass" rather
                    // than "nothing works", because the latter is a
                    // worse failure mode.
                    warn!(
                        target: "connectrpc_cf_rate_limit",
                        mode = mode.name(),
                        outcome = "error",
                        key = %key,
                        err = %err,
                        "rate-limit binding errored — failing open",
                    );
                    inner.call(req).await
                }
                RateLimitOutcome::Exceeded => {
                    if mode.is_enforcing() {
                        warn!(
                            target: "connectrpc_cf_rate_limit",
                            mode = mode.name(),
                            outcome = "throttled",
                            key = %key,
                            "rate-limit exceeded — rejecting",
                        );
                        let msg = format!("rate limit exceeded for key {key}");
                        Ok(deny_response(ConnectError::resource_exhausted(msg)))
                    } else {
                        log_shadow("rate-limit", mode.name(), "throttle", &format!("key={key}"));
                        inner.call(req).await
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IpKeyExtractor;
    use async_trait::async_trait;
    use bytes::Bytes;
    use http_body_util::Full;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::service_fn;

    /// Mock limiter: rejects every Nth request, never errors.
    struct EveryN {
        n: usize,
        count: AtomicUsize,
    }

    #[async_trait]
    impl RateLimiter for EveryN {
        async fn check(&self, _key: String) -> RateLimitOutcome {
            let i = self.count.fetch_add(1, Ordering::SeqCst);
            if (i + 1).is_multiple_of(self.n) {
                RateLimitOutcome::Exceeded
            } else {
                RateLimitOutcome::Allowed
            }
        }
    }

    fn ok_response() -> Response<ConnectRpcBody> {
        Response::new(ConnectRpcBody::Full(Full::new(Bytes::from_static(b"{}"))))
    }

    #[tokio::test]
    async fn enforce_blocks_exceeded() {
        let layer = RateLimitLayer::enforce(
            EveryN {
                n: 2,
                count: AtomicUsize::new(0),
            },
            IpKeyExtractor::new(),
        );
        let inner =
            service_fn(|_req: http::Request<()>| async { Ok::<_, Infallible>(ok_response()) });
        let mut svc = layer.layer(inner);

        // First request: allowed (count == 1, not divisible by 2).
        let req1 = http::Request::builder()
            .header("cf-connecting-ip", "1.2.3.4")
            .uri("/svc/M")
            .body(())
            .unwrap();
        let resp1 = svc.call(req1).await.unwrap();
        assert_eq!(resp1.status(), 200);

        // Second request: exceeded (count == 2, divisible by 2).
        let req2 = http::Request::builder()
            .header("cf-connecting-ip", "1.2.3.4")
            .uri("/svc/M")
            .body(())
            .unwrap();
        let resp2 = svc.call(req2).await.unwrap();
        // Connect-protocol error: HTTP 429 for resource_exhausted.
        assert_eq!(resp2.status(), 429);
    }

    #[tokio::test]
    async fn observe_logs_but_passes_through() {
        let layer = RateLimitLayer::observe(
            EveryN {
                n: 1,
                count: AtomicUsize::new(0),
            },
            IpKeyExtractor::new(),
        );
        let inner =
            service_fn(|_req: http::Request<()>| async { Ok::<_, Infallible>(ok_response()) });
        let mut svc = layer.layer(inner);

        // Mock always says Exceeded, but Observe never blocks.
        let req = http::Request::builder()
            .header("cf-connecting-ip", "1.2.3.4")
            .uri("/svc/M")
            .body(())
            .unwrap();
        let resp = svc.call(req).await.unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn skip_paths_dodge_binding_call() {
        let layer = RateLimitLayer::enforce(
            EveryN {
                n: 1,
                count: AtomicUsize::new(0),
            },
            IpKeyExtractor::new(),
        )
        .skip_paths(["/healthz"]);
        let inner =
            service_fn(|_req: http::Request<()>| async { Ok::<_, Infallible>(ok_response()) });
        let mut svc = layer.layer(inner);

        let req = http::Request::builder()
            .header("cf-connecting-ip", "1.2.3.4")
            .uri("/healthz")
            .body(())
            .unwrap();
        let resp = svc.call(req).await.unwrap();
        // Healthz passes despite mock always saying Exceeded.
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn missing_key_fails_open() {
        let layer = RateLimitLayer::enforce(
            EveryN {
                n: 1,
                count: AtomicUsize::new(0),
            },
            IpKeyExtractor::new(),
        );
        let inner =
            service_fn(|_req: http::Request<()>| async { Ok::<_, Infallible>(ok_response()) });
        let mut svc = layer.layer(inner);

        // No cf-connecting-ip header — extractor returns None.
        let req = http::Request::builder().uri("/svc/M").body(()).unwrap();
        let resp = svc.call(req).await.unwrap();
        assert_eq!(resp.status(), 200);
    }
}
