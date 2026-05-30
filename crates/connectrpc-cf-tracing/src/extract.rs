//! Typed CF-runtime metadata stamped onto each tracing span.
//!
//! The extractor pattern mirrors `CedarRequestExtractor` in the
//! sibling Cedar crate: the consumer supplies a closure (or a typed
//! struct implementing [`CfFieldsExtractor`]) that converts whatever
//! their host has — `worker::Cf` on CF Workers, `ConnectInfo<SocketAddr>`
//! on axum, nothing on a test harness — into a [`CfFields`].
//!
//! Keeping the extraction outside the layer means the crate stays
//! `worker`-dep-free. Downstream consumers wire `worker::Cf` in ~8 LOC.

use http::Request;
use std::sync::Arc;

/// Typed metadata stamped onto each request's tracing span.
///
/// Every field is optional because the crate doesn't know what host
/// it's running under. On a CF Worker every field will normally be
/// `Some`; in tests / non-CF hosts only the fields the consumer can
/// fill get populated. Missing fields show up in tracing output as
/// the literal `None`, which is a useful signal.
#[derive(Clone, Debug, Default)]
pub struct CfFields {
    /// Resolved Connect-RPC procedure path (e.g. `/pkg.v1.Service/Method`).
    /// Captured at layer time so handlers don't have to re-derive it.
    pub procedure: Option<String>,
    /// Cloudflare data-center colo (e.g. `"SIN"`, `"FRA"`). From
    /// `worker::Cf::colo()`. Always present in production.
    pub colo: Option<String>,
    /// ISO-3166-1 alpha-2 country code (e.g. `"SG"`, `"US"`). From
    /// `worker::Cf::country()`. Absent for some bots / Tor exits.
    pub country: Option<String>,
    /// Autonomous System Number of the inbound IP. From
    /// `worker::Cf::asn()`.
    pub asn: Option<u32>,
    /// Negotiated TLS cipher (e.g. `"AEAD-AES128-GCM-SHA256"`). From
    /// `worker::Cf::tls_cipher()`. Present for HTTPS, blank for HTTP.
    pub tls_cipher: Option<String>,
    /// HTTP protocol version (`"HTTP/1.1"`, `"HTTP/2"`, `"HTTP/3"`).
    /// From `worker::Cf::http_protocol()`.
    pub http_protocol: Option<String>,
    /// Cloudflare's per-request trace id (the `cf-ray` header).
    /// Useful for cross-referencing with Cloudflare's own logs.
    pub ray: Option<String>,
}

impl CfFields {
    /// Empty `CfFields`. Useful in tests and as a fallback when the
    /// extractor can't see any CF runtime data.
    pub fn empty() -> Self {
        Self::default()
    }
}

/// Reads CF runtime data off an inbound request.
///
/// Blanket implemented for any `Fn(&Request<B>) -> CfFields`, so the
/// common case is a closure passed to [`TracingLayer::new`].
///
/// A bespoke implementor is worth it when the extractor needs its own
/// state (e.g. caching a parsed `Cf` across the request lifecycle).
///
/// [`TracingLayer::new`]: crate::TracingLayer::new
pub trait CfFieldsExtractor<B>: Send + Sync + 'static {
    fn extract(&self, req: &Request<B>) -> CfFields;
}

impl<B, F> CfFieldsExtractor<B> for F
where
    F: Fn(&Request<B>) -> CfFields + Send + Sync + 'static,
{
    fn extract(&self, req: &Request<B>) -> CfFields {
        (self)(req)
    }
}

impl<B, E: CfFieldsExtractor<B> + ?Sized> CfFieldsExtractor<B> for Arc<E> {
    fn extract(&self, req: &Request<B>) -> CfFields {
        (**self).extract(req)
    }
}
