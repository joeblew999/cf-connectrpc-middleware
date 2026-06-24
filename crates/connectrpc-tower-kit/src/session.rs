//! The [`Session`] — the AuthN → AuthZ handoff contract.
//!
//! Whatever authenticates a request (a Rauthy JWT via `connectrpc-oidc`, an
//! opaque session/API-key/macaroon via `connectrpc-session`, anything else)
//! inserts ONE of these into `req.extensions()`. The authorization layer
//! (`connectrpc-cedar`) reads it out — it never cares HOW the request was
//! authenticated. Lives here, in the neutral kit, so no AuthN crate depends on
//! another just to name the type.

/// The authenticated principal handed from an AuthN layer to an AuthZ layer.
/// Transport-, OIDC- and Cedar-agnostic on purpose.
#[derive(Debug, Clone, Default)]
pub struct Session {
    /// Stable subject/user id (e.g. the OIDC `sub`). Becomes `User::"<subject>"`.
    pub subject: String,
    pub email: Option<String>,
    pub roles: Vec<String>,
    pub groups: Vec<String>,
    pub scopes: Vec<String>,
}
