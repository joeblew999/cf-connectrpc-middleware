//! # connectrpc-cedar
//!
//! Cedar policy authorization middleware for ConnectRPC handlers on Cloudflare
//! Workers (or any `tower::Service` stack).
//!
//! ## Composition
//!
//! ```text
//!   request ─► AuthLayer  ─► CedarLayer  ─► your service
//!              (verifies)    (authorizes)    (business logic)
//! ```
//!
//! `AuthLayer` (provided by your worker) inserts a session struct into
//! `req.extensions()`. `CedarLayer` reads that, builds a [`CedarRequest`]
//! via your [`CedarRequestExtractor`], evaluates against the loaded
//! [`CedarAuthorizer`], and either passes through (shadow mode / allow)
//! or short-circuits with `permission_denied` (enforce mode + deny).
//!
//! ## Shadow mode
//!
//! For first rollouts: run alongside an existing hand-rolled `require_*`
//! layer with `CedarLayer::shadow(...)`. Cedar evaluates every request
//! and logs its decision via `tracing` but never rejects. Operators
//! compare Cedar's would-have-done log against the actual responses
//! driven by the hand-rolled layer. After N days of zero mismatch, flip
//! to `CedarLayer::enforce(...)` and remove the hand-rolled layer.
//!
//! ## Quick start
//!
//! ```ignore
//! use std::sync::Arc;
//! use connectrpc_cedar::{CedarAuthorizer, CedarLayer, CedarRequest, Mode};
//!
//! // 1. Load policies + schema (at worker boot).
//! let authorizer = Arc::new(CedarAuthorizer::from_str(
//!     include_str!("../policies/schema.cedarschema"),
//!     include_str!("../policies/all.cedar"),
//! )?);
//!
//! // 2. Define how to map your session into a Cedar request.
//! //    `Session` here is whatever type your AuthN layer inserts —
//! //    `connectrpc_tower_kit::Session` for the common case, or your own.
//! let extractor = |req: &http::Request<_>| -> Option<CedarRequest> {
//!     let session = req.extensions().get::<Session>()?;
//!     // Build CedarRequest::{principal, action, resource, context}
//!     // from `session` + the URL path.
//!     ...
//! };
//!
//! // 3. Add to your tower stack (after AuthLayer).
//! let layer = CedarLayer::shadow(authorizer, extractor)
//!     .skip_paths(["/healthz", "/oauth/callback"]);
//! ```

pub mod action;
pub mod authorizer;
pub mod extract;
pub mod layer;

pub use authorizer::{CedarAuthorizer, CedarAuthorizerError};
pub use extract::{CedarRequest, CedarRequestExtractor};
pub use layer::{CedarLayer, CedarService, Mode};

// Re-export the cedar_policy types consumers most commonly need so they
// don't have to add cedar-policy as a direct dependency for simple uses.
pub use cedar_policy::{Context, Decision, EntityUid};
