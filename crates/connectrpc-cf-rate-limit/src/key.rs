//! Extracts the rate-limit key from each request. Consumer-pluggable
//! via [`RateLimitKeyExtractor`], with a default impl
//! ([`IpKeyExtractor`]) that uses the standard CF Workers
//! `cf-connecting-ip` header.

use http::Request;

/// Trait the layer calls per request to derive a rate-limit key.
///
/// Returning `None` skips the rate-limit check entirely for this
/// request — useful for endpoints that shouldn't be rate-limited
/// (health checks, OAuth callbacks). For those, prefer
/// [`RateLimitLayer::skip_paths`](crate::RateLimitLayer::skip_paths)
/// — clearer than returning `None` deep in extractor logic.
///
/// Blanket impl for `Fn(&Request<B>) -> Option<String>` so consumers
/// can pass closures: `RateLimitLayer::observe(limiter, |req| ...)`.
pub trait RateLimitKeyExtractor<B>: Send + Sync + 'static {
    fn extract(&self, req: &Request<B>) -> Option<String>;
}

impl<B, F> RateLimitKeyExtractor<B> for F
where
    F: Fn(&Request<B>) -> Option<String> + Send + Sync + 'static,
{
    fn extract(&self, req: &Request<B>) -> Option<String> {
        (self)(req)
    }
}

/// Default extractor — per-IP rate limiting via the standard CF
/// `cf-connecting-ip` request header. Workers set this on every
/// inbound request; falls back to `x-forwarded-for` first hop for
/// non-CF hosts (where this crate also compiles, even if it has no
/// production reason to run there).
///
/// Returns `None` if neither header is present, which makes the
/// layer pass through (fail-open) rather than reject — the
/// alternative (reject every request with no IP) is hostile to
/// healthchecks and local dev.
#[derive(Clone, Debug, Default)]
pub struct IpKeyExtractor;

impl IpKeyExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl<B> RateLimitKeyExtractor<B> for IpKeyExtractor {
    fn extract(&self, req: &Request<B>) -> Option<String> {
        if let Some(ip) = req.headers().get("cf-connecting-ip") {
            return ip.to_str().ok().map(|s| s.to_string());
        }
        // Fallback for non-CF hosts: first hop in `x-forwarded-for`.
        // Don't trust this beyond local dev — header is client-controlled.
        req.headers()
            .get("x-forwarded-for")
            .and_then(|h| h.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|s| s.trim().to_string())
    }
}
