//! Native HTTP server running the real `oidc → cedar` tower middleware.
//!
//!   request → OidcLayer (verify Rauthy JWT, insert Session)
//!           → CedarLayer (map Session→Cedar, authorize)
//!           → stub RPC (echoes the authorized session)
//!
//! Same two crates a Worker uses, hosted on hyper so you can curl it. The CF
//! Worker version reuses these exact layers; only the host (hyper vs
//! worker::event) and the JWKS fetch (ureq vs worker::Fetch) differ.
//!
//! Env:  RAUTHY_ISSUER  RAUTHY_JWKS_URL  [RAUTHY_AUD=worker-client]  [PORT=8090]
//!
//! Try it (after `e2e`-style minting, or a token from the GUI):
//!   curl -s localhost:8090/healthz
//!   curl -s -H "Authorization: Bearer $TOKEN" -X POST localhost:8090/demo.v1.Api/Read
//!   curl -s -H "Authorization: Bearer $TOKEN" -X POST localhost:8090/demo.v1.Api/Admin

use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use cedar_policy::{Context, EntityUid, RestrictedExpression};
use connectrpc::ConnectRpcBody;
use connectrpc_cedar::{CedarAuthorizer, CedarLayer, CedarRequest, action::action_from_path};
use connectrpc_oidc::{JwksVerifier, OidcLayer, Session};
use http::{Response, StatusCode};
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;
use tower::{Layer, ServiceExt, service_fn};

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Map the authenticated Session into a Cedar request. Roles ride in `context`
/// (the principal is dynamic — first seen at request time), like remy-sport.
fn extract(req: &http::Request<Incoming>) -> Option<CedarRequest> {
    let session = req.extensions().get::<Session>()?;
    let action = action_from_path(req.uri().path())?;
    let principal: EntityUid = format!(r#"User::"{}""#, session.subject).parse().ok()?;
    let resource: EntityUid = r#"Api::"main""#.parse().ok()?;
    let context = Context::from_pairs([
        (
            "roles".to_string(),
            RestrictedExpression::new_set(
                session
                    .roles
                    .iter()
                    .map(|r| RestrictedExpression::new_string(r.clone())),
            ),
        ),
        (
            "scopes".to_string(),
            RestrictedExpression::new_set(
                session
                    .scopes
                    .iter()
                    .map(|s| RestrictedExpression::new_string(s.clone())),
            ),
        ),
    ])
    .ok()?;
    Some(CedarRequest {
        principal,
        action,
        resource,
        context,
    })
}

fn json(status: StatusCode, body: String) -> Response<ConnectRpcBody> {
    Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(ConnectRpcBody::Full(Full::new(Bytes::from(body))))
        .unwrap()
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let issuer = std::env::var("RAUTHY_ISSUER").expect("set RAUTHY_ISSUER");
    let jwks_url = std::env::var("RAUTHY_JWKS_URL").expect("set RAUTHY_JWKS_URL");
    let aud = env_or("RAUTHY_AUD", "worker-client");
    let port: u16 = env_or("PORT", "8090").parse().unwrap();

    // Boot-time JWKS fetch (native path — ureq). The Worker swaps this one line
    // for connectrpc_oidc::fetch::fetch_jwks (worker::Fetch).
    println!("fetching JWKS from {jwks_url} ...");
    let jwks = ureq::get(&jwks_url).call()?.into_string()?;
    let verifier = Arc::new(
        JwksVerifier::from_jwks_json(&issuer, Some(aud), &jwks).expect("build verifier"),
    );
    let authz = Arc::new(
        CedarAuthorizer::from_str(
            include_str!("../policies/demo.cedarschema"),
            include_str!("../policies/demo.cedar"),
        )
        .expect("load policies"),
    );

    // The stub "RPC": if a request reaches it, OidcLayer verified the token and
    // CedarLayer allowed the action. Echo the authorized session.
    let stub = service_fn(|req: http::Request<Incoming>| async move {
        let detail = match req.extensions().get::<Session>() {
            Some(s) => format!("sub={} roles={:?}", s.subject, s.roles),
            None => "anonymous (skip path)".to_string(),
        };
        Ok::<_, Infallible>(json(
            StatusCode::OK,
            format!(r#"{{"status":"ok","authorized":"{detail}"}}"#),
        ))
    });

    // oidc (verify) → cedar (authorize) → stub. /healthz bypasses auth.
    let svc = OidcLayer::new(verifier)
        .skip_paths(["/healthz"])
        .layer(CedarLayer::enforce(authz, extract).layer(stub));

    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    println!("oidc→cedar server on http://127.0.0.1:{port}  (issuer {issuer})");
    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let hyper_svc = TowerToHyperService::new(svc.clone().map_err(|e: Infallible| match e {}));
        tokio::task::spawn(async move {
            if let Err(e) = http1::Builder::new().serve_connection(io, hyper_svc).await {
                eprintln!("conn error: {e}");
            }
        });
    }
}
