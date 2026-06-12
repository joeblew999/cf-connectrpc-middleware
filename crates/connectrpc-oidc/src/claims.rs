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
}

/// What `OidcLayer` inserts into request extensions after a token validates.
/// This is the AuthN→AuthZ handoff: `connectrpc-cedar`'s extractor consumes
/// it. Keep it transport- and Cedar-agnostic so other authorizers can reuse it.
#[derive(Debug, Clone)]
pub struct Session {
    pub subject: String,
    pub email: Option<String>,
    pub roles: Vec<String>,
    pub groups: Vec<String>,
    pub scopes: Vec<String>,
}

impl From<Claims> for Session {
    fn from(c: Claims) -> Self {
        Session {
            subject: c.sub,
            email: c.email,
            roles: c.roles,
            groups: c.groups,
            scopes: c.scope.split_whitespace().map(str::to_owned).collect(),
        }
    }
}
