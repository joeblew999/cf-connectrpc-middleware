//! The shared `oidc → cedar` application — host-agnostic.
//!
//! The `OidcLayer → CedarLayer` composition AND the Session→Cedar mapping now
//! live in [`connectrpc_guard`] — this app supplies only the two things that
//! are genuinely its own: the Cedar **policy files** and the (stub) RPC. The
//! two hosts (`../server`, `../worker`) each still do the platform-specific
//! pair: fetch the JWKS (native `ureq` / CF `worker::Fetch`) → run the service
//! (native `hyper` / CF `worker::event(fetch)`). That's the ConnectRPC promise:
//! one app, both runtimes — and now the guard itself is shared, not copied.

use std::convert::Infallible;
use std::sync::{Arc, OnceLock};

use bytes::Bytes;
use connectrpc_guard::{guard, load_authorizer, CedarAuthorizer, JwksVerifier};
use connectrpc_oidc::Session;
use http::{header::CONTENT_TYPE, Response, StatusCode};
use http_body_util::Full;
use tower::{service_fn, Service};

// Re-export so hosts don't need a direct connectrpc dep just to name the type.
pub use connectrpc::ConnectRpcBody;

/// The Cedar authorizer, loaded once from the bundled demo policies.
pub fn authorizer() -> Arc<CedarAuthorizer> {
    static A: OnceLock<Arc<CedarAuthorizer>> = OnceLock::new();
    A.get_or_init(|| {
        load_authorizer(
            include_str!("../policies/demo.cedarschema"),
            include_str!("../policies/demo.cedar"),
        )
        .expect("bundled demo policies must load")
    })
    .clone()
}

/// Build the full guarded service for a verifier. Generic over the body `B`, so
/// native (`hyper::body::Incoming`) and CF (`worker::Body`) call the identical
/// constructor — the guard composition comes from `connectrpc-guard`.
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

    guard(verifier, authorizer(), "main", &["/healthz"], stub)
}
