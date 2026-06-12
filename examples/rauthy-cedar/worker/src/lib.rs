//! Cloudflare Worker running the `oidc → cedar` tower middleware on the edge.
//!
//! The SAME two layers as the native `server/` example — only the host differs:
//! `worker::event(fetch)` instead of hyper, and `worker::Fetch` instead of ureq
//! for the boot-time JWKS load (the `worker-jwks` feature). The middleware,
//! policies, and extractor are identical.
//!
//!   request → OidcLayer (verify Rauthy JWT) → CedarLayer (authorize) → stub
//!
//! Vars (wrangler.toml): RAUTHY_ISSUER, RAUTHY_JWKS_URL, RAUTHY_AUD.

use std::convert::Infallible;
use std::sync::{Arc, OnceLock};

use bytes::Bytes;
use cedar_policy::{Context, EntityUid, RestrictedExpression};
use connectrpc::ConnectRpcBody;
use connectrpc_cedar::{CedarAuthorizer, CedarLayer, CedarRequest, action::action_from_path};
use connectrpc_oidc::{JwksVerifier, OidcLayer, Session};
use http::{Response, StatusCode};
use http_body_util::Full;
use tokio::sync::OnceCell;
use tower::{Layer, Service, service_fn};
use worker::{Context as WorkerCtx, Env, HttpRequest, event};

/// JWKS is fetched once per isolate and cached. worker::Fetch futures are
/// `!Send`; `OnceCell::get_or_try_init` doesn't require Send, and Workers are
/// single-threaded, so this is sound.
static VERIFIER: OnceCell<Arc<JwksVerifier>> = OnceCell::const_new();

async fn verifier(env: &Env) -> worker::Result<Arc<JwksVerifier>> {
    VERIFIER
        .get_or_try_init(|| async {
            let issuer = env.var("RAUTHY_ISSUER")?.to_string();
            let jwks_url = env.var("RAUTHY_JWKS_URL")?.to_string();
            let aud = env.var("RAUTHY_AUD").ok().map(|v| v.to_string());
            let jwks = connectrpc_oidc::fetch::fetch_jwks(&jwks_url).await?;
            JwksVerifier::from_jwks_json(issuer, aud, &jwks)
                .map(Arc::new)
                .map_err(|e| worker::Error::RustError(format!("jwks: {e:?}")))
        })
        .await
        .cloned()
}

fn authorizer() -> Arc<CedarAuthorizer> {
    static A: OnceLock<Arc<CedarAuthorizer>> = OnceLock::new();
    A.get_or_init(|| {
        Arc::new(
            CedarAuthorizer::from_str(
                include_str!("../policies/demo.cedarschema"),
                include_str!("../policies/demo.cedar"),
            )
            .expect("policies must load"),
        )
    })
    .clone()
}

/// Session → Cedar request; roles ride in context (dynamic principal).
fn extract<B>(req: &http::Request<B>) -> Option<CedarRequest> {
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

#[event(fetch, respond_with_errors)]
async fn fetch(
    req: HttpRequest,
    env: Env,
    _ctx: WorkerCtx,
) -> worker::Result<Response<ConnectRpcBody>> {
    let verifier = verifier(&env).await?;

    // Stub "RPC": reaching it means OidcLayer verified the token and CedarLayer
    // allowed the action. Echo the authorized session.
    let stub = service_fn(|req: HttpRequest| async move {
        let detail = match req.extensions().get::<Session>() {
            Some(s) => format!("sub={} roles={:?}", s.subject, s.roles),
            None => "anonymous (skip path)".to_string(),
        };
        Ok::<_, Infallible>(
            Response::builder()
                .status(StatusCode::OK)
                .header(http::header::CONTENT_TYPE, "application/json")
                .body(ConnectRpcBody::Full(Full::new(Bytes::from(format!(
                    r#"{{"status":"ok","authorized":"{detail}"}}"#
                )))))
                .unwrap(),
        )
    });

    let mut svc = OidcLayer::new(verifier)
        .skip_paths(["/healthz"])
        .layer(CedarLayer::enforce(authorizer(), extract).layer(stub));

    Ok(svc.call(req).await.unwrap())
}
