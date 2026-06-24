//! The shared auth/authz guard for ConnectRPC projects — **AuthN-pluggable**.
//!
//! The whole point of the middleware: a project brings its services + Cedar
//! policy files and gets authentication + authorization **for free** — never
//! hand-writing the guard composition or the Session→Cedar mapping again.
//!
//! The authz half is decoupled from the authn half via the shared [`Session`]
//! contract (`subject/roles/groups/scopes` in request extensions). So BOTH
//! Rauthy and non-Rauthy auth are supported:
//!
//! - **Rauthy / OIDC:** [`guard`] = `OidcLayer` (verify the Rauthy JWT) →
//!   [`cedar_enforce`]. The common case; one call.
//! - **Non-Rauthy:** bring any `tower::Layer` that inserts a [`Session`] (your
//!   own session/token/macaroon layer) and wrap [`cedar_enforce`] yourself:
//!   `my_auth_layer.layer(cedar_enforce(authorizer, resource, inner))`. The
//!   authz (Cedar) is identical — it only reads the `Session`, not how it got
//!   there.
//!
//! - [`load_authorizer`] reads the project's `.cedarschema` + `.cedar`.
//! - [`session_to_cedar`] maps a [`Session`] into a Cedar request (generic).

use std::convert::Infallible;
use std::sync::Arc;

use cedar_policy::{Context, EntityUid, RestrictedExpression};
use connectrpc::ConnectRpcBody;
use connectrpc_cedar::{action::action_from_path, CedarLayer, CedarRequest};
use connectrpc_oidc::OidcLayer;
use http::Response;
use tower::{Layer, Service};

// Re-exports so a host names these without depending on the sub-crates directly.
pub use connectrpc_cedar::{CedarAuthorizer, CedarAuthorizerError};
// `Session` is the AuthN→AuthZ contract: ANY auth layer (Rauthy or not) inserts
// one of these into request extensions; `cedar_enforce` reads it.
pub use connectrpc_oidc::{JwksVerifier, Session};

/// Load a Cedar authorizer from a project's schema + policy strings (typically
/// `include_str!("policies/<proj>.cedarschema")` and `…/<proj>.cedar`).
pub fn load_authorizer(
    schema: &str,
    policies: &str,
) -> Result<Arc<CedarAuthorizer>, CedarAuthorizerError> {
    CedarAuthorizer::from_str(schema, policies).map(Arc::new)
}

fn to_set(items: &[String]) -> RestrictedExpression {
    RestrictedExpression::new_set(
        items.iter().map(|s| RestrictedExpression::new_string(s.clone())),
    )
}

/// Map the verified [`Session`] (inserted into request extensions by
/// `OidcLayer`) into a Cedar request: a dynamic `User::"<sub>"` principal, the
/// project's `Api::"<resource>"`, the action derived from the RPC path, and
/// roles+scopes in `context`. This is the one mapping every project used to
/// re-copy by hand.
pub fn session_to_cedar<B>(req: &http::Request<B>, resource: &str) -> Option<CedarRequest> {
    let session = req.extensions().get::<Session>()?;
    let action = action_from_path(req.uri().path())?;
    let principal: EntityUid = format!(r#"User::"{}""#, session.subject).parse().ok()?;
    let resource: EntityUid = format!(r#"Api::"{resource}""#).parse().ok()?;
    let context = Context::from_pairs([
        ("roles".to_string(), to_set(&session.roles)),
        ("scopes".to_string(), to_set(&session.scopes)),
    ])
    .ok()?;
    Some(CedarRequest { principal, action, resource, context })
}

/// The AUTHZ half — **auth-mechanism-agnostic**. Wrap a ConnectRPC service with
/// Cedar policy enforcement that reads the [`Session`] from request extensions,
/// regardless of which AuthN layer put it there. Use this directly for
/// **non-Rauthy** auth: `my_auth_layer.layer(cedar_enforce(authorizer, resource, inner))`.
pub fn cedar_enforce<S, B>(
    authorizer: Arc<CedarAuthorizer>,
    resource: &'static str,
    inner: S,
) -> impl Service<http::Request<B>, Response = Response<ConnectRpcBody>, Error = Infallible> + Clone
where
    S: Service<http::Request<B>, Response = Response<ConnectRpcBody>, Error = Infallible> + Clone,
    B: 'static,
{
    CedarLayer::enforce(authorizer, move |req: &http::Request<B>| {
        session_to_cedar(req, resource)
    })
    .layer(inner)
}

/// Compose the full guard for the **Rauthy / OIDC** case: `OidcLayer` (verify
/// the Rauthy JWT → insert a [`Session`]) → [`cedar_enforce`]. `skip_paths` are
/// public (health checks, etc.). A project supplies only its `verifier`,
/// `authorizer` (its policy files), a `resource` name, and the inner service —
/// host-agnostic, so native (`hyper::body::Incoming`) and CF (`worker::Body`)
/// call the identical constructor. For non-Rauthy auth, use [`cedar_enforce`]
/// under your own AuthN layer instead.
pub fn guard<S, B>(
    verifier: Arc<JwksVerifier>,
    authorizer: Arc<CedarAuthorizer>,
    resource: &'static str,
    skip_paths: &'static [&'static str],
    inner: S,
) -> impl Service<http::Request<B>, Response = Response<ConnectRpcBody>, Error = Infallible> + Clone
where
    S: Service<http::Request<B>, Response = Response<ConnectRpcBody>, Error = Infallible> + Clone,
    B: 'static,
{
    OidcLayer::new(verifier)
        .skip_paths(skip_paths.iter().copied())
        .layer(cedar_enforce(authorizer, resource, inner))
}
