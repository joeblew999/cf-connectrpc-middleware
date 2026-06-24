//! The shared Rauthy-OIDC → Cedar guard for ConnectRPC projects.
//!
//! The whole point of the auth/authz middleware: a project brings its services
//! and its Cedar policy files, and gets authentication + authorization **for
//! free** — it should never hand-write the guard composition or the
//! Session→Cedar mapping again (they were being copy-pasted per project).
//!
//! - [`load_authorizer`] reads the project's `.cedarschema` + `.cedar` strings.
//! - [`session_to_cedar`] maps the verified [`Session`] into a Cedar request —
//!   dynamic `User` principal, roles+scopes in `context`, action from the RPC
//!   path. Generic; the project passes only its `resource` name.
//! - [`guard`] composes `OidcLayer` (verify the JWT) → `CedarLayer` (enforce)
//!   around the project's ConnectRPC service.

use std::convert::Infallible;
use std::sync::Arc;

use cedar_policy::{Context, EntityUid, RestrictedExpression};
use connectrpc::ConnectRpcBody;
use connectrpc_cedar::{action::action_from_path, CedarLayer, CedarRequest};
use connectrpc_oidc::{OidcLayer, Session};
use http::Response;
use tower::{Layer, Service};

// Re-exports so a host names these without depending on the sub-crates directly.
pub use connectrpc_cedar::{CedarAuthorizer, CedarAuthorizerError};
pub use connectrpc_oidc::JwksVerifier;

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

/// Compose the `OidcLayer → CedarLayer` guard around a ConnectRPC service:
/// verify the Rauthy JWT, then enforce the project's Cedar policies. `skip_paths`
/// are public (health checks, etc.). A project supplies only its `verifier`,
/// `authorizer` (its policy files), a `resource` name, and the inner service —
/// the host stays host-agnostic, so native (`hyper::body::Incoming`) and CF
/// (`worker::Body`) call the identical constructor.
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
        .layer(
            CedarLayer::enforce(authorizer, move |req: &http::Request<B>| {
                session_to_cedar(req, resource)
            })
            .layer(inner),
        )
}
