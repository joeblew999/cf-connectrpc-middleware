//! The token claims we decode from a Rauthy JWT, and the [`Session`] we hand
//! downstream. `Session` is the contract with `connectrpc-cedar` — its
//! extractor reads `req.extensions().get::<Session>()`.

use serde::Deserialize;

/// Raw OIDC claims as Rauthy issues them. Only the fields we map into a
/// [`Session`] are listed; `serde` ignores the rest.
///
/// Rauthy emits `roles` and `groups` as part of its access/id tokens when the
/// client is configured with the corresponding scopes. If your client doesn't
/// request them they arrive empty — policies then fall back to whatever the
/// `sub`-only principal grants.
#[derive(Debug, Clone, Deserialize)]
pub struct Claims {
    /// Subject — the stable user id. Becomes the Cedar `User::"<sub>"`.
    pub sub: String,
    /// Issuer — must match the configured `RAUTHY_ISSUER`.
    pub iss: String,
    /// Expiry (unix seconds). Validated by `jsonwebtoken`.
    pub exp: usize,
    #[serde(default)]
    pub email: Option<String>,
    /// Rauthy roles, e.g. `["admin", "coach"]`.
    #[serde(default)]
    pub roles: Vec<String>,
    /// Rauthy groups, e.g. `["asm-u16-boys"]`.
    #[serde(default)]
    pub groups: Vec<String>,
    /// Space-delimited OIDC scopes, split into [`Session::scopes`].
    #[serde(default)]
    pub scope: String,
    /// Audience — OIDC allows a single string OR an array. Validated against
    /// the verifier's configured audience when one is set.
    #[serde(default)]
    pub aud: Option<Aud>,
}

/// `aud` is `string | string[]` per the JWT spec.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Aud {
    One(String),
    Many(Vec<String>),
}

impl Aud {
    pub fn contains(&self, want: &str) -> bool {
        match self {
            Aud::One(a) => a == want,
            Aud::Many(v) => v.iter().any(|a| a == want),
        }
    }
}

// `Session` (what `OidcLayer` inserts into request extensions) is the shared
// AuthN→AuthZ contract — it now lives in the neutral `connectrpc-tower-kit` so
// non-OIDC auth crates (`connectrpc-session`, …) insert one without depending
// on this Rauthy crate. Re-exported here for back-compat.
pub use connectrpc_tower_kit::Session;

/// Map decoded Rauthy [`Claims`] into a [`Session`]. A free function rather than
/// `impl From<Claims> for Session` because `Session` is now a foreign type (in
/// tower-kit) and the orphan rule forbids the impl.
pub fn session_from_claims(c: Claims) -> Session {
    Session {
        subject: c.sub,
        email: c.email,
        roles: c.roles,
        groups: c.groups,
        scopes: c.scope.split_whitespace().map(str::to_owned).collect(),
    }
}
