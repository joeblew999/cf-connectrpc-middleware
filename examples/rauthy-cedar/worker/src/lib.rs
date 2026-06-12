//! CLOUDFLARE WORKER host for the shared `rauthy-cedar-app`.
//!
//! Identical to the native `../server` except for the two platform bits:
//!   1. fetch JWKS with `worker::Fetch` (the server uses `ureq`)
//!   2. serve via `worker::event(fetch)` (the server uses `hyper`)
//!
//! The `oidc → cedar` stack, the policies, and the extractor are the SAME
//! `rauthy_cedar_app::make` the native host calls. That's ConnectRPC's promise:
//! one app, native and edge.
//!
//! Vars (wrangler.toml): RAUTHY_ISSUER, RAUTHY_JWKS_URL, RAUTHY_AUD.

use std::sync::Arc;

use connectrpc_oidc::JwksVerifier;
use rauthy_cedar_app::{ConnectRpcBody, make};
use tokio::sync::OnceCell;
use tower::Service;
use worker::{Context, Env, HttpRequest, event};

/// JWKS fetched once per isolate, cached. worker::Fetch futures are `!Send`;
/// `OnceCell::get_or_try_init` doesn't require Send and Workers are
/// single-threaded, so this is sound.
static VERIFIER: OnceCell<Arc<JwksVerifier>> = OnceCell::const_new();

async fn verifier(env: &Env) -> worker::Result<Arc<JwksVerifier>> {
    VERIFIER
        .get_or_try_init(|| async {
            let issuer = env.var("RAUTHY_ISSUER")?.to_string();
            let jwks_url = env.var("RAUTHY_JWKS_URL")?.to_string();
            let aud = env.var("RAUTHY_AUD").ok().map(|v| v.to_string());
            // (1) platform-specific: CF JWKS fetch via worker::Fetch.
            let jwks = connectrpc_oidc::fetch::fetch_jwks(&jwks_url).await?;
            JwksVerifier::from_jwks_json(issuer, aud, &jwks)
                .map(Arc::new)
                .map_err(|e| worker::Error::RustError(format!("jwks: {e:?}")))
        })
        .await
        .cloned()
}

#[event(fetch, respond_with_errors)]
async fn fetch(
    req: HttpRequest,
    env: Env,
    _ctx: Context,
) -> worker::Result<http::Response<ConnectRpcBody>> {
    let verifier = verifier(&env).await?;
    // Same shared stack as the native host.
    let mut svc = make::<worker::Body>(verifier);
    Ok(svc.call(req).await.unwrap())
}
