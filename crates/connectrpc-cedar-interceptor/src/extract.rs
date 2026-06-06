//! Body-aware session → Cedar request extraction.
//!
//! This is the Interceptor-surface analogue of `connectrpc-cedar`'s
//! [`CedarRequestExtractor`](connectrpc_cedar::CedarRequestExtractor).
//! The Layer extractor only sees the raw `http::Request` (headers +
//! extensions, no decoded body). This one runs *after* the Connect
//! envelope is decoded, so the closure receives a [`UnaryRequest`] and
//! can read BOTH:
//!
//! - `req.ctx` — the [`RequestContext`](connectrpc::RequestContext):
//!   `path()`, `headers()`, and `extensions()` (e.g. the `SessionContext`
//!   an upstream `AuthLayer` inserted — the dispatcher copies
//!   `http::Request::extensions` → `Context::extensions` automatically,
//!   per MIDDLEWARES.md §6 pattern 2); and
//! - `req.payload` — the decoded request message, via
//!   `req.payload.message::<YourProtoMsg>()`, for body-aware decisions
//!   (e.g. authorize against the `org_id` carried in the request body).
//!
//! Returning `None` means "no Cedar-relevant shape" (anonymous endpoint,
//! unmapped procedure) — the interceptor passes through without
//! evaluating, exactly like the Layer.

use connectrpc::UnaryRequest;
use connectrpc_cedar::CedarRequest;

/// How [`CedarInterceptor`](crate::CedarInterceptor) turns a decoded
/// unary request into a [`CedarRequest`].
///
/// The blanket impl lets you pass a closure directly:
///
/// ```ignore
/// let extractor = |req: &connectrpc::UnaryRequest| -> Option<CedarRequest> {
///     let session = req.ctx.extensions().get::<SessionContext>()?;
///     let path = req.ctx.path()?;
///     // body-aware: read a field off the decoded message
///     // let msg = req.payload.message::<CreateOrgRequest>().ok()?;
///     Some(CedarRequest { principal, action, resource, context })
/// };
/// let interceptor = CedarInterceptor::shadow(authorizer, extractor);
/// ```
pub trait CedarUnaryExtractor: Send + Sync + 'static {
    fn extract(&self, req: &UnaryRequest) -> Option<CedarRequest>;
}

impl<F> CedarUnaryExtractor for F
where
    F: Fn(&UnaryRequest) -> Option<CedarRequest> + Send + Sync + 'static,
{
    fn extract(&self, req: &UnaryRequest) -> Option<CedarRequest> {
        (self)(req)
    }
}
