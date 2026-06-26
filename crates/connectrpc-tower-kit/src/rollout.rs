//! Safe-rollout abstraction. Generalizes `CedarLayer`'s `Mode::Shadow` /
//! `Mode::Enforce` pattern so every middleware that *can reject* can
//! ship the same "evaluate + log, never reject" rollout knob.
//!
//! # Why a trait, not an enum
//!
//! An earlier design put `Mode { Shadow, Enforce }` in the kit and made
//! every middleware reuse the enum. That over-fit Cedar:
//!
//! - Cedar wants `Shadow` / `Enforce`.
//! - A rate limiter wants `Observe` / `Throttle`.
//! - A validation interceptor wants `Warn` / `Reject`.
//! - A tracing sampler wants `Sample(rate)` / `All`.
//!
//! The mode *concept* is universal; the *enum* is per-middleware. Each
//! middleware defines its own enum (named for its domain) and impls
//! [`Rollout`]. The kit's [`log_shadow`] helper then writes a consistent
//! `target = "connectrpc_middleware"` event for shadow decisions.
//!
//! # Example
//!
//! ```
//! use connectrpc_tower_kit::Rollout;
//!
//! #[derive(Clone, Copy, Debug)]
//! enum CedarMode { Shadow, Enforce }
//!
//! impl Rollout for CedarMode {
//!     fn is_enforcing(&self) -> bool {
//!         matches!(self, CedarMode::Enforce)
//!     }
//!     fn name(&self) -> &'static str {
//!         match self {
//!             CedarMode::Shadow  => "shadow",
//!             CedarMode::Enforce => "enforce",
//!         }
//!     }
//! }
//! ```

/// A rollout mode for a rejecting middleware.
///
/// Implementors are typically `Copy + Debug + Eq` enums named for the
/// middleware's domain (`CedarMode`, `RateLimitMode`, `ValidationMode`,
/// …). See module docs.
pub trait Rollout: std::fmt::Debug + Send + Sync + 'static {
    /// `true` if the middleware should *act* on a negative decision
    /// (reject, throttle, drop). `false` if it should evaluate +
    /// log + pass through (shadow / observe / warn mode).
    fn is_enforcing(&self) -> bool;

    /// Short string for the `mode = "..."` field in shadow-decision
    /// log events. Typically lowercase: `"shadow"`, `"enforce"`,
    /// `"observe"`, `"throttle"`.
    fn name(&self) -> &'static str;
}

/// Helper for the shadow-decision log line every rejecting middleware
/// emits. Centralizing the format keeps `mode = "shadow"` consistent
/// across the family so operators can `wrangler tail | grep` once and
/// see every middleware's shadow output.
///
/// `middleware` is a short name like `"cedar"`, `"rate-limit"`,
/// `"validation"`. `decision` is the rejected outcome (`"deny"`,
/// `"throttle"`, `"invalid"`). `details` is a free-form string the
/// middleware can use for whatever context matters (Cedar reasons,
/// rate-limit bucket, validation violations).
pub fn log_shadow(
    middleware: &'static str,
    mode: &'static str,
    decision: &'static str,
    details: &str,
) {
    // `target = "connectrpc_middleware"` so `RUST_LOG=connectrpc_middleware=info`
    // catches every shadow-mode middleware in the family with one filter.
    tracing::warn!(
        target: "connectrpc_middleware",
        middleware = middleware,
        mode = mode,
        decision = decision,
        details = details,
        "shadow: would-have-{decision} in enforce mode",
    );
}
