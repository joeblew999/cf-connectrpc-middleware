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
//!   by returning a Connect-protocol `permission_denied` response.
//!
//! ## Skip paths
//!
//! Public endpoints (health checks, OAuth callbacks, etc.) don't have
//! a session and shouldn't be Cedar-authorized. Pass them to
//! [`CedarLayer::skip_paths`] and the layer falls through to the inner
//! service without evaluating. Pattern lifted from
//! `cedar-policy/authorization-for-expressjs`'s `skippedEndpoints`.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

use bytes::Bytes;
use cedar_policy::Decision;
use connectrpc::{ConnectError, ConnectRpcBody};
use http::Response;
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
    ///
    /// Matches by exact path equality. For prefix or pattern matching,
    /// use [`Self::skip_with`] instead.
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
    S: Service<http::Request<B>, Response = Response<ConnectRpcBody>, Error = ConnectError>,
    E: CedarRequestExtractor<B>,
{
    type Response = Response<ConnectRpcBody>;
    type Error = ConnectError;
    type Future = CedarFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<B>) -> Self::Future {
        let path = req.uri().path();

        // 1. Skip paths — health checks, OAuth callbacks, etc.
        if self.skip_paths.iter().any(|p| p == path) {
            return CedarFuture::pass(self.inner.call(req));
        }

        // 2. Extract Cedar request. None means "not a Cedar-relevant
        //    shape" (e.g. anonymous endpoint, no session). Pass through.
        let Some(cedar_req) = self.extractor.extract(&req) else {
            return CedarFuture::pass(self.inner.call(req));
        };

        // 3. Evaluate.
        let (decision, reasons) = self.authorizer.is_authorized(
            &cedar_req.principal,
            &cedar_req.action,
            &cedar_req.resource,
            cedar_req.context,
        );

        // 4. Log. Always — both modes log so operators can diff.
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

        // 5. Decide what to do with the request.
        match (self.mode, decision) {
            // Shadow always passes through.
            (Mode::Shadow, _) | (Mode::Enforce, Decision::Allow) => {
                CedarFuture::pass(self.inner.call(req))
            }
            // Enforce + Deny → short-circuit with permission_denied.
            (Mode::Enforce, Decision::Deny) => {
                let msg = if reasons.is_empty() {
                    "cedar denied".to_string()
                } else {
                    format!("cedar denied: [{}]", reasons.join(", "))
                };
                CedarFuture::denied(ConnectError::permission_denied(msg))
            }
        }
    }
}

pin_project! {
    /// Future returned by [`CedarService::call`]. Either passes through
    /// to the inner service or short-circuits with a denial.
    #[project = CedarFutureProj]
    pub enum CedarFuture<F> {
        Pass { #[pin] inner: F },
        Denied { err: Option<ConnectError> },
    }
}

impl<F> CedarFuture<F> {
    fn pass(inner: F) -> Self {
        Self::Pass { inner }
    }
    fn denied(err: ConnectError) -> Self {
        Self::Denied { err: Some(err) }
    }
}

impl<F> std::future::Future for CedarFuture<F>
where
    F: std::future::Future<Output = Result<Response<ConnectRpcBody>, ConnectError>>,
{
    type Output = Result<Response<ConnectRpcBody>, ConnectError>;
    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        match self.project() {
            CedarFutureProj::Pass { inner } => inner.poll(cx),
            CedarFutureProj::Denied { err } => {
                let err = err.take().expect("CedarFuture::Denied polled after completion");
                Poll::Ready(Err(err))
            }
        }
    }
}

// Suppress unused-warning for Bytes — the Bytes type appears in the
// public Service bound via ConnectRpcBody but rustc 1.88's unused-imports
// lint doesn't see through it.
const _: fn() = || {
    let _: Bytes = Bytes::new();
};
