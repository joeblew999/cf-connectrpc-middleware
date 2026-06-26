//! `Mode::{Observe, Enforce}` — adopts the kit's `Rollout` trait so
//! consumers can roll out rate limiting safely:
//!
//! 1. Deploy in [`Mode::Observe`]. Layer calls the binding, logs every
//!    "would have blocked" event via `tracing`, **but always calls
//!    inner**. Operators watch the logs for false positives (wrong
//!    key derivation, mis-tuned limits).
//! 2. Flip to [`Mode::Enforce`]. Same code path, denials now actually
//!    short-circuit with `ConnectError::resource_exhausted`.
//!
//! The pattern parallels [`connectrpc_cedar::Mode`] — same `Rollout`
//! trait, different enum, different semantics.

use connectrpc_tower_kit::Rollout;

/// Rollout mode for the rate-limit layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Call the binding, log the decision, never block. Use during
    /// rollout to validate key derivation against real traffic.
    Observe,
    /// Call the binding, block on `success = false`. Production mode.
    Enforce,
}

impl Rollout for Mode {
    fn is_enforcing(&self) -> bool {
        matches!(self, Mode::Enforce)
    }

    fn name(&self) -> &'static str {
        match self {
            Mode::Observe => "observe",
            Mode::Enforce => "enforce",
        }
    }
}
