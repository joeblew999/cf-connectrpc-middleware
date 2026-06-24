//! Non-OIDC AuthN for ConnectRPC — the sibling of `connectrpc-oidc` for
//! projects that DON'T use Rauthy.
//!
//! A `tower::Layer` that pulls the bearer token off the request and verifies it
//! with a caller-supplied closure (`Fn(&str) -> Option<Session>`): a DB session
//! lookup, an API-key check, a macaroon verification — whatever the project
//! uses. On success it inserts the shared [`Session`] into `req.extensions()`
//! exactly like `OidcLayer` does, so the SAME `connectrpc-cedar` /
//! `connectrpc-guard::cedar_enforce` authorizes the request. Both AuthN crates
//! feed one Session contract; the authz layer never knows which authenticated
//! the caller.
//!
//! ```ignore
//! // Non-Rauthy: your own token verification + the shared Cedar enforcement.
//! SessionLayer::new(|tok| my_store.lookup(tok))   // → Session
//!     .skip_paths(["/health"])
//!     .layer(connectrpc_guard::cedar_enforce(authorizer, "myapp", my_service));
//! ```
//!
//! Shape mirrors `connectrpc-oidc`'s `OidcLayer` (same `ShortCircuitFuture`,
//! `deny_response`, `Error = Infallible`), so the two are drop-in alternatives.

use std::convert::Infallible;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

use connectrpc::{ConnectError, ConnectRpcBody};
use connectrpc_tower_kit::{deny_response, Session, ShortCircuitFuture};
use http::Response;
use tower::{Layer, Service};
use tracing::warn;

pub use connectrpc_tower_kit::Session as SessionContract;

/// Tower layer wrapping a service with opaque-token verification.
pub struct SessionLayer<F> {
    verify: Arc<F>,
    skip_paths: Arc<Vec<String>>,
}

impl<F> Clone for SessionLayer<F> {
    fn clone(&self) -> Self {
        Self {
            verify: Arc::clone(&self.verify),
            skip_paths: Arc::clone(&self.skip_paths),
        }
    }
}

impl<F> SessionLayer<F>
where
    F: Fn(&str) -> Option<Session>,
{
    /// `verify` maps a bearer token to a [`Session`] (`Some` = authenticated,
    /// `None` = reject). It owns whatever the project's auth means — a session
    /// store lookup, signature check, API-key match, macaroon verification.
    pub fn new(verify: F) -> Self {
        Self {
            verify: Arc::new(verify),
            skip_paths: Arc::new(Vec::new()),
        }
    }

    /// Paths that bypass verification (e.g. `/health`).
    pub fn skip_paths<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.skip_paths = Arc::new(paths.into_iter().map(Into::into).collect());
        self
    }
}

impl<S, F> Layer<S> for SessionLayer<F> {
    type Service = SessionService<S, F>;

    fn layer(&self, inner: S) -> Self::Service {
        SessionService {
            inner,
            verify: Arc::clone(&self.verify),
            skip_paths: Arc::clone(&self.skip_paths),
        }
    }
}

/// The wrapped service produced by [`SessionLayer`].
pub struct SessionService<S, F> {
    inner: S,
    verify: Arc<F>,
    skip_paths: Arc<Vec<String>>,
}

impl<S: Clone, F> Clone for SessionService<S, F> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            verify: Arc::clone(&self.verify),
            skip_paths: Arc::clone(&self.skip_paths),
        }
    }
}

impl<S, F, B> Service<http::Request<B>> for SessionService<S, F>
where
    S: Service<http::Request<B>, Response = Response<ConnectRpcBody>, Error = Infallible>,
    F: Fn(&str) -> Option<Session>,
{
    type Response = Response<ConnectRpcBody>;
    type Error = Infallible;
    type Future = ShortCircuitFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: http::Request<B>) -> Self::Future {
        if self.skip_paths.iter().any(|p| p == req.uri().path()) {
            return ShortCircuitFuture::pass(self.inner.call(req));
        }

        let token = req
            .headers()
            .get(http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .map(str::to_owned);

        let Some(token) = token else {
            warn!(target: "connectrpc_session", reason = "missing_bearer", "rejecting request");
            return ShortCircuitFuture::denied(deny_response(ConnectError::unauthenticated(
                "missing bearer token",
            )));
        };

        match (self.verify)(&token) {
            Some(session) => {
                // Same AuthN→AuthZ handoff as OidcLayer: downstream Cedar reads
                // this Session out of extensions.
                req.extensions_mut().insert(session);
                ShortCircuitFuture::pass(self.inner.call(req))
            }
            None => {
                warn!(target: "connectrpc_session", reason = "invalid_token", "rejecting token");
                ShortCircuitFuture::denied(deny_response(ConnectError::unauthenticated(
                    "invalid session token",
                )))
            }
        }
    }
}
