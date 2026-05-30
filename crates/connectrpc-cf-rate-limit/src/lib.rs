//! Cloudflare Rate Limiting binding wrapped as a Connect-RPC `tower::Layer`.
//!
//! ## What this crate does
//!
//! Provides [`RateLimitLayer`] — a short-circuit `tower::Layer` that
//! calls Cloudflare's Rate Limiting binding before the request reaches
//! the inner `ConnectRpcService`. On `success = false`, the layer
//! short-circuits with a Connect-protocol `resource_exhausted` error
//! response.
//!
//! Adopts the kit's [`Rollout`](connectrpc_tower_kit::Rollout) trait
//! via [`Mode::Observe`] (call the binding, log the decision, never
//! block — for shadow rollout to verify key derivation) and
//! [`Mode::Enforce`] (block on exceeded). See MIDDLEWARES.md §6
//! pattern 3 for the safe-rollout pattern.
//!
//! ## CF Workers compatibility
//!
//! - **Builds on `wasm32-unknown-unknown`**: yes.
//! - **CF binding required**: Rate Limiting. Provision in
//!   `wrangler.toml`:
//!   ```toml
//!   [[unsafe.bindings]]
//!   name = "RL"
//!   type = "ratelimit"
//!   namespace_id = "1001"
//!   simple = { limit = 100, period = 60 }
//!   ```
//! - **Crate-level `worker` dep**: none. Consumer implements the
//!   [`RateLimiter`] trait wrapping their `env.RL` binding — same
//!   pattern as `connectrpc-cf-tracing`'s extractor closure.
//!
//! ## Why short-circuit
//!
//! Rate limiting rejects requests — surface #2 in MIDDLEWARES.md §1.
//! The Layer pins `S::Response = Response<ConnectRpcBody>` and
//! `S::Error = Infallible` so the denial response can be constructed
//! without invoking `S`. Same shape as `connectrpc-cedar`'s Enforce
//! mode.

pub mod key;
pub mod layer;
pub mod limiter;
pub mod mode;

pub use key::{IpKeyExtractor, RateLimitKeyExtractor};
pub use layer::{RateLimitLayer, RateLimitService};
pub use limiter::{RateLimitOutcome, RateLimiter};
pub use mode::Mode;
