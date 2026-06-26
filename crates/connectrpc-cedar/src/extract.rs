//! Pluggable session → Cedar request extraction.
//!
//! `CedarLayer` is generic over how the consumer's session shape maps
//! into Cedar's (principal, action, resource, context) tuple. Consumers
//! implement [`CedarRequestExtractor`] (or pass a closure that satisfies
//! the trait's blanket impl) and `CedarLayer` calls it per-request.
//!
//! This keeps the crate framework-agnostic: it doesn't depend on any
//! particular session type. Each project defines its own session shape
//! and adapter (e.g. `connectrpc_tower_kit::Session`, or its own struct).

use cedar_policy::{Context, EntityUid};

/// A request prepared for Cedar evaluation. The extractor builds this
/// from the inbound HTTP request + your session struct + any per-request
/// resource lookup you need.
///
/// `principal`, `action`, `resource` are required by Cedar. `context` is
/// optional in policies but always passed — use `Context::empty()` if
/// you have nothing to put there.
#[derive(Debug, Clone)]
pub struct CedarRequest {
    pub principal: EntityUid,
    pub action: EntityUid,
    pub resource: EntityUid,
    pub context: Context,
}

/// How `CedarLayer` extracts a [`CedarRequest`] from each inbound HTTP
/// request.
///
/// Returning `None` means "this request doesn't have a Cedar-relevant
/// shape" — for example a request with no session (anonymous endpoint)
/// or one targeting a path that doesn't map to a known action. The
/// layer then passes through to the inner service without evaluating.
///
/// The blanket impl below lets you pass a closure directly:
///
/// ```ignore
/// let extractor = |req: &http::Request<_>| -> Option<CedarRequest> {
///     // ...
/// };
/// let layer = CedarLayer::shadow(authorizer, extractor);
/// ```
pub trait CedarRequestExtractor<B>: Send + Sync + 'static {
    fn extract(&self, req: &http::Request<B>) -> Option<CedarRequest>;
}

impl<F, B> CedarRequestExtractor<B> for F
where
    F: Fn(&http::Request<B>) -> Option<CedarRequest> + Send + Sync + 'static,
{
    fn extract(&self, req: &http::Request<B>) -> Option<CedarRequest> {
        (self)(req)
    }
}
