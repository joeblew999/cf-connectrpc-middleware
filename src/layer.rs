//! `tower::Layer` wrapping a `CedarAuthorizer`.
//!
//! ## Modes
//!
//! - [`Mode::Shadow`] — Cedar evaluates every request and logs the
//!   decision via `tracing`, but ALWAYS passes through to the inner
//!   service. Use during rollout when a hand-rolled authz layer is
//!   still in place; operators compare Cedar's logs against actual
//!   responses to catch mismatches before flipping to enforce.
//! - [`Mode::Enforce`] — Cedar evaluates and rejects on `Decision::Deny`
//!   by returning a Connect-protocol `permission_denied` HTTP response
//!   (not by raising `Error`; ConnectRpcService has `Error = Infallible`
//!   and Connect encodes failures into the body, so we follow suit).
//!
//! ## Skip paths
//!
//! Public endpoints (health checks, OAuth callbacks, etc.) don't have
//! a session and shouldn't be Cedar-authorized. Pass them to
//! [`CedarLayer::skip_paths`] and the layer falls through to the inner
//! service without evaluating. Pattern lifted from
//! `cedar-policy/authorization-for-expressjs`'s `skippedEndpoints`.

use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

use bytes::Bytes;
use cedar_policy::Decision;
use connectrpc::{ConnectError, ConnectRpcBody};
use http::{HeaderName, Response, StatusCode};
use http_body_util::Full;
use pin_project_lite::pin_project;
use tower::{Layer, Service};
use tracing::{info, warn};

use crate::authorizer::CedarAuthorizer;
use crate::extract::CedarRequestExtractor;

/// Run mode for the layer. See module-level docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Evaluate + log, never reject. Use during shadow rollout.
    Shadow,
    /// Evaluate + reject on Deny.
    Enforce,
}

/// The Cedar middleware layer. Construct via [`CedarLayer::shadow`] or
/// [`CedarLayer::enforce`]; configure skip-paths via the builder
/// methods.
pub struct CedarLayer<E> {
    authorizer: Arc<CedarAuthorizer>,
    extractor: Arc<E>,
    mode: Mode,
    skip_paths: Arc<Vec<String>>,
}

impl<E> Clone for CedarLayer<E> {
    fn clone(&self) -> Self {
        Self {
            authorizer: Arc::clone(&self.authorizer),
            extractor: Arc::clone(&self.extractor),
            mode: self.mode,
            skip_paths: Arc::clone(&self.skip_paths),
        }
    }
}

impl<E> CedarLayer<E> {
    /// Shadow-mode layer: Cedar evaluates and logs but never rejects.
    pub fn shadow(authorizer: Arc<CedarAuthorizer>, extractor: E) -> Self {
        Self {
            authorizer,
            extractor: Arc::new(extractor),
            mode: Mode::Shadow,
            skip_paths: Arc::new(Vec::new()),
        }
    }

    /// Enforce-mode layer: Cedar rejects on Deny.
    pub fn enforce(authorizer: Arc<CedarAuthorizer>, extractor: E) -> Self {
        Self {
            authorizer,
            extractor: Arc::new(extractor),
            mode: Mode::Enforce,
            skip_paths: Arc::new(Vec::new()),
        }
    }

    /// Paths the layer skips entirely (no Cedar evaluation, pass
    /// straight through to the inner service). For health checks,
    /// OAuth callbacks, anything that has no session.
    pub fn skip_paths<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let owned: Vec<String> = paths.into_iter().map(Into::into).collect();
        self.skip_paths = Arc::new(owned);
        self
    }
}

impl<S, E> Layer<S> for CedarLayer<E> {
    type Service = CedarService<S, E>;
    fn layer(&self, inner: S) -> Self::Service {
        CedarService {
            inner,
            authorizer: Arc::clone(&self.authorizer),
            extractor: Arc::clone(&self.extractor),
            mode: self.mode,
            skip_paths: Arc::clone(&self.skip_paths),
        }
    }
}

/// Per-request service produced by [`CedarLayer::layer`].
pub struct CedarService<S, E> {
    inner: S,
    authorizer: Arc<CedarAuthorizer>,
    extractor: Arc<E>,
    mode: Mode,
    skip_paths: Arc<Vec<String>>,
}

impl<S: Clone, E> Clone for CedarService<S, E> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            authorizer: Arc::clone(&self.authorizer),
            extractor: Arc::clone(&self.extractor),
            mode: self.mode,
            skip_paths: Arc::clone(&self.skip_paths),
        }
    }
}

impl<S, E, B> Service<http::Request<B>> for CedarService<S, E>
where
    S: Service<http::Request<B>, Response = Response<ConnectRpcBody>, Error = Infallible>,
    E: CedarRequestExtractor<B>,
{
    type Response = Response<ConnectRpcBody>;
    type Error = Infallible;
    type Future = CedarFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<B>) -> Self::Future {
        let path = req.uri().path();

        if self.skip_paths.iter().any(|p| p == path) {
            return CedarFuture::pass(self.inner.call(req));
        }

        let Some(cedar_req) = self.extractor.extract(&req) else {
            return CedarFuture::pass(self.inner.call(req));
        };

        let (decision, reasons) = self.authorizer.is_authorized(
            &cedar_req.principal,
            &cedar_req.action,
            &cedar_req.resource,
            cedar_req.context,
        );

        // Log every decision (both modes), so operators can diff
        // Cedar's would-have-done log against the actual response
        // stream during shadow rollout.
        match (self.mode, decision) {
            (Mode::Shadow, Decision::Allow) => info!(
                target: "connectrpc_cedar",
                mode = "shadow",
                decision = "ALLOW",
                principal = %cedar_req.principal,
                action = %cedar_req.action,
                resource = %cedar_req.resource,
                reasons = ?reasons,
            ),
            (Mode::Shadow, Decision::Deny) => warn!(
                target: "connectrpc_cedar",
                mode = "shadow",
                decision = "DENY",
                note = "would-have-rejected in enforce mode",
                principal = %cedar_req.principal,
                action = %cedar_req.action,
                resource = %cedar_req.resource,
                reasons = ?reasons,
            ),
            (Mode::Enforce, Decision::Allow) => info!(
                target: "connectrpc_cedar",
                mode = "enforce",
                decision = "ALLOW",
                principal = %cedar_req.principal,
                action = %cedar_req.action,
                resource = %cedar_req.resource,
                reasons = ?reasons,
            ),
            (Mode::Enforce, Decision::Deny) => warn!(
                target: "connectrpc_cedar",
                mode = "enforce",
                decision = "DENY",
                principal = %cedar_req.principal,
                action = %cedar_req.action,
                resource = %cedar_req.resource,
                reasons = ?reasons,
            ),
        }

        match (self.mode, decision) {
            (Mode::Shadow, _) | (Mode::Enforce, Decision::Allow) => {
                CedarFuture::pass(self.inner.call(req))
            }
            (Mode::Enforce, Decision::Deny) => {
                let msg = if reasons.is_empty() {
                    "cedar denied".to_string()
                } else {
                    format!("cedar denied: [{}]", reasons.join(", "))
                };
                CedarFuture::denied(deny_response(msg))
            }
        }
    }
}

/// Build a Connect-protocol error response body for a Cedar denial.
/// The Connect spec encodes errors as a 2xx-like HTTP response with a
/// JSON body `{"code": "permission_denied", "message": "..."}`.
/// ConnectError handles the encoding via its public helpers.
fn deny_response(msg: String) -> Response<ConnectRpcBody> {
    let err = ConnectError::permission_denied(msg);
    let body_bytes = err.to_json();
    let status = err.http_status();

    let mut builder = Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, "application/json");

    // Surface ConnectError-attached response headers (rare but allowed).
    for (k, v) in err.response_headers() {
        builder = builder.header(k, v);
    }

    // Connect protocol trailers travel as `trailer-<name>` headers.
    for (k, v) in err.trailers() {
        let prefixed = format!("trailer-{}", k.as_str());
        if let Ok(name) = HeaderName::try_from(prefixed) {
            builder = builder.header(name, v);
        }
    }

    builder
        .body(ConnectRpcBody::Full(Full::new(body_bytes)))
        .unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(ConnectRpcBody::Full(Full::new(Bytes::new())))
                .expect("static infallible builder")
        })
}

pin_project! {
    /// Future returned by [`CedarService::call`]. Either passes through
    /// to the inner service or short-circuits with a denial response.
    #[project = CedarFutureProj]
    pub enum CedarFuture<F> {
        Pass { #[pin] inner: F },
        Denied { response: Option<Response<ConnectRpcBody>> },
    }
}

impl<F> CedarFuture<F> {
    fn pass(inner: F) -> Self {
        Self::Pass { inner }
    }
    fn denied(response: Response<ConnectRpcBody>) -> Self {
        Self::Denied {
            response: Some(response),
        }
    }
}

impl<F> std::future::Future for CedarFuture<F>
where
    F: std::future::Future<Output = Result<Response<ConnectRpcBody>, Infallible>>,
{
    type Output = Result<Response<ConnectRpcBody>, Infallible>;
    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        match self.project() {
            CedarFutureProj::Pass { inner } => inner.poll(cx),
            CedarFutureProj::Denied { response } => Poll::Ready(Ok(response
                .take()
                .expect("CedarFuture::Denied polled after completion"))),
        }
    }
}
