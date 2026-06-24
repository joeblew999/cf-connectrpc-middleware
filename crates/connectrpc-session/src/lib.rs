//! Non-OIDC AuthN for ConnectRPC — the sibling of `connectrpc-oidc` for
//! projects that DON'T use Rauthy.
//!
//! A `tower::Layer` that pulls the bearer token off the request and verifies it
//! with a caller-supplied closure (`Fn(&str) -> Option<T>`): a DB session
//! lookup, an API-key check, a **macaroon** verification — whatever the project
//! uses. On success it inserts `T` into `req.extensions()`, exactly like
//! `OidcLayer` inserts a `Session`, so a downstream authz layer
//! (`connectrpc-cedar` / `connectrpc-guard::cedar_enforce`) reads it out.
//!
//! **Generic over the inserted type `T`** — use the shared
//! [`connectrpc_tower_kit::Session`] for the common case, or your own richer
//! session struct (the example-multitenant-worker inserts a typed
//! `SessionContext` carrying billing/org/role — no downgrade to generic strings).
//!
//! **Two modes** (matching the family's soft-middleware pattern, MIDDLEWARES.md
//! §6 pattern 3):
//! - [`enforce`](SessionLayer::new) (default) — reject (`401`) when the token is
//!   missing/invalid. Like `OidcLayer`.
//! - [`decode`](SessionLayer::decode) — soft: insert `T` if a valid token is
//!   present, otherwise **pass through** unauthenticated. For services where
//!   some RPCs are public (signup/login) and handlers enforce per-RPC.
//!
//! ```ignore
//! // macaroon, soft (the example-multitenant-worker pattern):
//! SessionLayer::new(move |tok| verify_macaroon(&keyring, tok).ok())  // -> Option<SessionContext>
//!     .decode()
//!     .layer(service);
//!
//! // generic non-Rauthy, hard:
//! SessionLayer::new(move |tok| store.lookup(tok))  // -> Option<connectrpc_tower_kit::Session>
//!     .skip_paths(["/health"])
//!     .layer(cedar_enforce(authorizer, "myapp", service));
//! ```

use std::convert::Infallible;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

use connectrpc::{ConnectError, ConnectRpcBody};
use connectrpc_tower_kit::{deny_response, ShortCircuitFuture};
use http::Response;
use tower::{Layer, Service};
use tracing::warn;

// The common inserted type; projects may insert their own richer struct instead.
pub use connectrpc_tower_kit::Session;

/// How the layer treats a missing/invalid token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Reject with `401` (the default; like `OidcLayer`).
    Enforce,
    /// Soft: insert the session if a valid token is present, else pass through
    /// unauthenticated and let the handler enforce.
    Decode,
}

/// Tower layer wrapping a service with opaque-token verification. Generic over
/// the inserted session type `T` (whatever `verify` returns).
pub struct SessionLayer<F> {
    verify: Arc<F>,
    skip_paths: Arc<Vec<String>>,
    mode: Mode,
}

impl<F> Clone for SessionLayer<F> {
    fn clone(&self) -> Self {
        Self {
            verify: Arc::clone(&self.verify),
            skip_paths: Arc::clone(&self.skip_paths),
            mode: self.mode,
        }
    }
}

impl<F> SessionLayer<F> {
    /// `verify` maps a bearer token to `Some(session)` (authenticated) or `None`
    /// (reject/skip). Defaults to [`Mode::Enforce`].
    pub fn new(verify: F) -> Self {
        Self {
            verify: Arc::new(verify),
            skip_paths: Arc::new(Vec::new()),
            mode: Mode::Enforce,
        }
    }

    /// Switch to [`Mode::Decode`] (soft — insert if present, never reject).
    #[must_use]
    pub fn decode(mut self) -> Self {
        self.mode = Mode::Decode;
        self
    }

    /// Paths that bypass verification entirely (e.g. `/health`).
    #[must_use]
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
            mode: self.mode,
        }
    }
}

/// The wrapped service produced by [`SessionLayer`].
pub struct SessionService<S, F> {
    inner: S,
    verify: Arc<F>,
    skip_paths: Arc<Vec<String>>,
    mode: Mode,
}

impl<S: Clone, F> Clone for SessionService<S, F> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            verify: Arc::clone(&self.verify),
            skip_paths: Arc::clone(&self.skip_paths),
            mode: self.mode,
        }
    }
}

impl<S, F, B, T> Service<http::Request<B>> for SessionService<S, F>
where
    S: Service<http::Request<B>, Response = Response<ConnectRpcBody>, Error = Infallible>,
    F: Fn(&str) -> Option<T>,
    T: Send + Sync + Clone + 'static,
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

        let session = req
            .headers()
            .get(http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .and_then(|tok| (self.verify)(tok));

        match (session, self.mode) {
            // Authenticated: insert the session and continue (both modes).
            (Some(session), _) => {
                req.extensions_mut().insert(session);
                ShortCircuitFuture::pass(self.inner.call(req))
            }
            // Soft mode: no/invalid token → pass through unauthenticated.
            (None, Mode::Decode) => ShortCircuitFuture::pass(self.inner.call(req)),
            // Hard mode: no/invalid token → reject.
            (None, Mode::Enforce) => {
                warn!(target: "connectrpc_session", "rejecting unauthenticated request");
                ShortCircuitFuture::denied(deny_response(ConnectError::unauthenticated(
                    "missing or invalid session token",
                )))
            }
        }
    }
}
