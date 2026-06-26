//! # connectrpc-oidc
//!
//! OIDC/JWT verification middleware for ConnectRPC handlers on Cloudflare
//! Workers (or any `tower::Service` stack). This is the **AuthN** layer that
//! `connectrpc-cedar` refers to as "AuthLayer (provided by your worker)":
//!
//! ```text
//!   request ─► OidcLayer  ─► CedarLayer  ─► your service
//!              (this crate)   (authorizes)   (business logic)
//!              verifies the    reads the
//!              Rauthy JWT,      Session and
//!              inserts Session  authorizes
//! ```
//!
//! ## Responsibilities
//!
//! 1. Pull the bearer token off the inbound request (`Authorization: Bearer …`).
//! 2. Validate it against the issuer's JWKS (signature, `iss`, `aud`, `exp`).
//! 3. Decode the claims into a [`Session`] and insert it into
//!    `req.extensions()` so downstream layers — notably `connectrpc-cedar`'s
//!    extractor — can map it into a Cedar principal + context.
//!
//! It deliberately does **not** know about Cedar. The seam is the [`Session`]
//! struct in request extensions, exactly as `connectrpc-cedar`'s docs expect.
//!
//! ## Mapping Rauthy → Cedar
//!
//! Rauthy issues standard OIDC claims plus `roles` and `groups`. The
//! cedar-side extractor turns those into the principal hierarchy:
//!
//! ```text
//!   sub     → principal  User::"<sub>"
//!   roles   → principal attribute / parent  Role::"COACH"
//!   groups  → parent entities               Group::"asm-u16-boys"
//!   scope   → context.scopes
//! ```
//!
//! See `examples/rauthy-cedar/` for the policy set this shape drives.

mod claims;
pub mod fetch;
mod jwks;
mod layer;

pub use claims::{Aud, Claims, Session, session_from_claims};
pub use jwks::{JwksError, JwksVerifier};
pub use layer::{OidcLayer, OidcService};
