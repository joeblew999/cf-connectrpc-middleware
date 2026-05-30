//! `RateLimiter` trait — the consumer-implemented seam that wraps the
//! CF binding.
//!
//! Consumer wires the binding in ~6 LOC:
//!
//! ```rust,ignore
//! use async_trait::async_trait;
//! use connectrpc_cf_rate_limit::{RateLimiter, RateLimitOutcome};
//! use std::sync::Arc;
//!
//! pub struct CfRateLimiter(pub Arc<worker::RateLimiter>);
//!
//! #[async_trait]
//! impl RateLimiter for CfRateLimiter {
//!     async fn check(&self, key: String) -> RateLimitOutcome {
//!         match self.0.limit(key).await {
//!             Ok(o) if o.success => RateLimitOutcome::Allowed,
//!             Ok(_)              => RateLimitOutcome::Exceeded,
//!             Err(e)             => RateLimitOutcome::Error(e.to_string()),
//!         }
//!     }
//! }
//! ```
//!
//! Three outcomes — `Allowed`, `Exceeded`, `Error` — so the layer can
//! distinguish "limit hit" (block) from "binding errored" (fail-open
//! and log, by design — a broken rate limiter shouldn't stop the world).

use async_trait::async_trait;

/// Outcome of one rate-limit check.
#[derive(Clone, Debug)]
pub enum RateLimitOutcome {
    /// Under the limit. Pass through.
    Allowed,
    /// At or above the limit. Block (in `Mode::Enforce`) or log
    /// (`Mode::Observe`).
    Exceeded,
    /// The binding errored. Fail-open: log + pass through regardless
    /// of mode. The error string is included for diagnostics.
    Error(String),
}

/// Wraps the CF Rate Limiting binding behind a Send + Sync trait so
/// the layer doesn't need to know about `worker::RateLimiter`.
#[async_trait]
pub trait RateLimiter: Send + Sync + 'static {
    /// Check whether `key` is under the limit. Owned `String` because
    /// the CF binding's `limit(key: String)` takes owned (`serde` round-trip
    /// to JS).
    async fn check(&self, key: String) -> RateLimitOutcome;
}
