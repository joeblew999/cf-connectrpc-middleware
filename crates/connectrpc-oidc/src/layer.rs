//! The `tower::Layer` that does the AuthN step.
//!
//! Mirrors `connectrpc-cedar`'s `CedarLayer` shape (same `ShortCircuitFuture`,
//! same `deny_response`, same `Error = Infallible`) so the two compose cleanly:
//!
//! ```ignore
//! ServiceBuilder::new()
//!     .layer(OidcLayer::new(verifier))               // verify token → insert Session
//!     .layer(CedarLayer::enforce(authz, extractor))  // read Session → authorize
//!     .service(my_connect_handler);
//! ```
//!
//! On a valid token it inserts a [`Session`] into `req.extensions()` and calls
//! the inner service. On a missing/invalid token it short-circuits with a
//! Connect-protocol `unauthenticated` response (the request never reaches
//! Cedar — you can't authorize an unauthenticated principal). Paths in
//! `skip_paths` (health checks, the OAuth callback) bypass verification.

use std::convert::Infallible;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

use connectrpc::{ConnectError, ConnectRpcBody};
use http::Response;
use tower::{Layer, Service};
use tracing::warn;

use connectrpc_tower_kit::{ShortCircuitFuture, deny_response};

use crate::claims::Session;
use crate::jwks::JwksVerifier;

/// Current unix time in seconds. Split by target because
/// `SystemTime::now()` panics on `wasm32-unknown-unknown` (no clock); on the
/// Worker we read it from JS `Date`.
fn now_unix() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() / 1000.0) as u64
    }
}

/// Tower layer wrapping a service with OIDC bearer-token verification.
#[derive(Clone)]
pub struct OidcLayer {
    verifier: Arc<JwksVerifier>,
    skip_paths: Arc<Vec<String>>,
}

impl OidcLayer {
    pub fn new(verifier: Arc<JwksVerifier>) -> Self {
        Self {
            verifier,
            skip_paths: Arc::new(Vec::new()),
        }
    }

    /// Paths that bypass verification (e.g. `/healthz`, `/oauth/callback`).
    pub fn skip_paths<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.skip_paths = Arc::new(paths.into_iter().map(Into::into).collect());
        self
    }
}

impl<S> Layer<S> for OidcLayer {
    type Service = OidcService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        OidcService {
            inner,
            verifier: Arc::clone(&self.verifier),
            skip_paths: Arc::clone(&self.skip_paths),
        }
    }
}

/// The wrapped service produced by [`OidcLayer`].
pub struct OidcService<S> {
    inner: S,
    verifier: Arc<JwksVerifier>,
    skip_paths: Arc<Vec<String>>,
}

impl<S: Clone> Clone for OidcService<S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            verifier: Arc::clone(&self.verifier),
            skip_paths: Arc::clone(&self.skip_paths),
        }
    }
}

impl<S, B> Service<http::Request<B>> for OidcService<S>
where
    S: Service<http::Request<B>, Response = Response<ConnectRpcBody>, Error = Infallible>,
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

        // Pull the bearer token off the Authorization header.
        let token = req
            .headers()
            .get(http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .map(str::to_owned);

        let Some(token) = token else {
            warn!(target: "connectrpc_oidc", reason = "missing_bearer", "rejecting request");
            return ShortCircuitFuture::denied(deny_response(ConnectError::unauthenticated(
                "missing bearer token",
            )));
        };

        match self.verifier.verify(&token, now_unix()) {
            Ok(claims) => {
                // The AuthN→AuthZ handoff: downstream (connectrpc-cedar's
                // extractor) reads this out of extensions.
                req.extensions_mut().insert(Session::from(claims));
                ShortCircuitFuture::pass(self.inner.call(req))
            }
            Err(err) => {
                warn!(target: "connectrpc_oidc", reason = ?err, "rejecting token");
                ShortCircuitFuture::denied(deny_response(ConnectError::unauthenticated(
                    "invalid token",
                )))
            }
        }
    }
}
