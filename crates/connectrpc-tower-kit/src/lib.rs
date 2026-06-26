//! # connectrpc-tower-kit
//!
//! Shared primitives for Connect-RPC `tower::Layer` middleware on
//! Cloudflare Workers. **No middleware lives in this crate.** It only
//! hosts the conventions that every middleware in the family wants:
//!
//! - [`Rollout`] trait + [`log_shadow`] helper — safe-rollout pattern
//!   ("evaluate + log, never reject") generalized so every rejecting
//!   middleware (Cedar, rate-limit, validation, …) can adopt it with
//!   its own enum (`Shadow`/`Enforce`, `Observe`/`Throttle`,
//!   `Warn`/`Reject`).
//! - [`deny_response`] — build a Connect-protocol error response
//!   (`Response<ConnectRpcBody>` with `permission_denied` body), the
//!   same way every short-circuiting Layer needs to.
//! - [`ShortCircuitFuture`] — `pin_project_lite` Future enum
//!   (`Pass { inner }` / `Denied { response }`) usable as
//!   `tower::Service::Future` for any short-circuit layer.
//! - [`ext`] — canonical names for `req.extensions()` entries so
//!   middlewares compose. Documents convention, doesn't lock types.
//!
//! ## Why a kit and not just helpers in `connectrpc-cedar`
//!
//! Three recurring middleware patterns (generic over `B`, soft
//! middleware + handler backstop, canonical extensions) are
//! conventions, not code; the kit documents them. The other three
//! (`Mode` toggles, short-circuit Future, denial-response builder) are
//! code that every middleware re-implements unless extracted. We're
//! extracting.
//!
//! When the family grows past Cedar — `connectrpc-cf-tracing`,
//! `connectrpc-cf-rate-limit`, `connectrpc-validation`, … — those
//! crates depend on `connectrpc-tower-kit`, not on each other.
//!
//! ## CF Workers compatibility
//!
//! Every dep here is wasm32-clean. The kit compiles to
//! `wasm32-unknown-unknown` with no platform feature flag.

#![forbid(unsafe_code)]
// Note on `missing_docs`: not enabled because `pin_project!` generates
// public struct fields the macro can't doc-comment. The `ShortCircuitFuture`
// variants are documented via the `pin_project!` block comment in
// `future.rs`. Re-enable once pin-project-lite supports field docs.

pub mod ext;
pub mod future;
pub mod response;
pub mod rollout;
pub mod session;

pub use future::ShortCircuitFuture;
pub use response::deny_response;
pub use rollout::{Rollout, log_shadow};
pub use session::Session;
