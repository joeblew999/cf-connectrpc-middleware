//! The shared `oidc → cedar` application — host-agnostic.
//!
//! This crate holds EVERYTHING that's the same whether you run native or on
//! Cloudflare: the policies, the Session→Cedar extractor, the stub RPC, and the
//! composed tower stack. The two hosts (`../server`, `../worker`) each do only
//! the two things that genuinely differ by platform:
//!
//!   1. fetch the JWKS (native: `ureq`; CF: `worker::Fetch`) → build a verifier
//!   2. run the service ( native: `hyper`; CF: `worker::event(fetch)` )
//!
//! Everything else — including the `OidcLayer → CedarLayer → stub` composition —
//! comes from [`make`]. That's the ConnectRPC promise: one app, both runtimes.

use std::convert::Infallible;
use std::sync::{Arc, OnceLock};

use bytes::Bytes;
use cedar_policy::{Context, EntityUid, RestrictedExpression};
use connectrpc_cedar::{CedarAuthorizer, CedarLayer, CedarRequest, action::action_from_path};
use connectrpc_oidc::{JwksVerifier, OidcLayer, Session};
use http::{Response, StatusCode, header::CONTENT_TYPE};
use http_body_util::Full;
use tower::{Layer, Service, service_fn};

// Re-export so hosts don't need a direct connectrpc dep just to name the type.
pub use connectrpc::ConnectRpcBody;

/// The Cedar authorizer, loaded once from the bundled policies.
pub fn authorizer() -> Arc<CedarAuthorizer> {
    static A: OnceLock<Arc<CedarAuthorizer>> = OnceLock::new();
    A.get_or_init(|| {
        Arc::new(
            CedarAuthorizer::from_str(
                include_str!("../policies/demo.cedarschema"),
                include_str!("../policies/demo.cedar"),
            )
            .expect("bundled policies must load"),
        )
    })
    .clone()
}

/// Map the authenticated [`Session`] into a Cedar request. Roles ride in
/// `context` (the principal is dynamic — first seen at request time).
pub fn extract<B>(req: &http::Request<B>) -> Option<CedarRequest> {
    let session = req.extensions().get::<Session>()?;
    let action = action_from_path(req.uri().path())?;
    let principal: EntityUid = format!(r#"User::"{}""#, session.subject).parse().ok()?;
    let resource: EntityUid = r#"Api::"main""#.parse().ok()?;
    let context = Context::from_pairs([
        ("roles".to_string(), set(&session.roles)),
        ("scopes".to_string(), set(&session.scopes)),
    ])
    .ok()?;
    Some(CedarRequest {
        principal,
        action,
        resource,
        context,
    })
}

fn set(items: &[String]) -> RestrictedExpression {
    RestrictedExpression::new_set(items.iter().map(|s| RestrictedExpression::new_string(s.clone())))
}

/// Build the full `OidcLayer → CedarLayer → stub` service for a given verifier.
/// Generic over the request body `B`, so the native (hyper `Incoming`) and CF
/// (`worker::Body`) hosts call the EXACT same constructor.
pub fn make<B>(
    verifier: Arc<JwksVerifier>,
) -> impl Service<http::Request<B>, Response = Response<ConnectRpcBody>, Error = Infallible> + Clone
where
    B: 'static,
{
    // Stub "RPC": reaching it means the token verified and Cedar allowed the
    // action. Echo the authorized session. Non-capturing → Clone.
    let stub = service_fn(|req: http::Request<B>| async move {
        let detail = match req.extensions().get::<Session>() {
            Some(s) => format!("sub={} roles={:?}", s.subject, s.roles),
            None => "anonymous (skip path)".to_string(),
        };
        Ok::<_, Infallible>(
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .body(ConnectRpcBody::Full(Full::new(Bytes::from(format!(
                    r#"{{"status":"ok","authorized":"{detail}"}}"#
                )))))
                .unwrap(),
        )
    });

    OidcLayer::new(verifier)
        .skip_paths(["/healthz"])
        .layer(CedarLayer::enforce(authorizer(), extract::<B>).layer(stub))
}
