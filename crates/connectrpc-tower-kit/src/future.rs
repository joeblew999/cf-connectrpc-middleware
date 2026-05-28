//! `pin_project_lite` Future enum for short-circuiting Layers.
//!
//! A short-circuiting Layer's `Service::call` returns either:
//!
//! - the inner service's future (pass-through), or
//! - a ready-to-poll completed response (the denial built by
//!   [`super::deny_response`]).
//!
//! These have different concrete types, so we need an enum that
//! implements `Future` either way. `pin_project_lite` gets us safe
//! field projection without unsafe Pin destructuring.
//!
//! Constrained to `Output = Result<Response<ConnectRpcBody>, Infallible>`
//! because that's `ConnectRpcService`'s signature — every short-circuit
//! Layer in this family targets it.

use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

use connectrpc::ConnectRpcBody;
use http::Response;
use pin_project_lite::pin_project;

pin_project! {
    /// The shared Future returned by every short-circuiting Layer's
    /// `Service::call`. `Pass { inner }` polls the wrapped inner
    /// service; `Denied { response }` returns the pre-built denial
    /// response on the next poll.
    ///
    /// `F` is the inner service's `Future` type — typically opaque.
    #[project = ShortCircuitFutureProj]
    pub enum ShortCircuitFuture<F> {
        /// Pass through to the inner service.
        Pass {
            #[pin]
            inner: F,
        },
        /// Short-circuit with a pre-built response.
        ///
        /// `Option<...>` so we can `.take()` on the first poll without
        /// requiring `Response<ConnectRpcBody>: Default`.
        Denied {
            response: Option<Response<ConnectRpcBody>>,
        },
    }
}

impl<F> ShortCircuitFuture<F> {
    /// Construct a pass-through variant wrapping the inner future.
    pub fn pass(inner: F) -> Self {
        Self::Pass { inner }
    }

    /// Construct a denial variant with the pre-built response.
    pub fn denied(response: Response<ConnectRpcBody>) -> Self {
        Self::Denied {
            response: Some(response),
        }
    }
}

impl<F> std::future::Future for ShortCircuitFuture<F>
where
    F: std::future::Future<Output = Result<Response<ConnectRpcBody>, Infallible>>,
{
    type Output = Result<Response<ConnectRpcBody>, Infallible>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project() {
            ShortCircuitFutureProj::Pass { inner } => inner.poll(cx),
            ShortCircuitFutureProj::Denied { response } => Poll::Ready(Ok(response
                .take()
                .expect("ShortCircuitFuture::Denied polled after completion"))),
        }
    }
}
