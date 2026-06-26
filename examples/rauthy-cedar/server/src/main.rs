//! NATIVE host for the shared `rauthy-cedar-app`.
//!
//! All the interesting code — the full `tracing → rate-limit → oidc → cedar →
//! service(+metrics+body-cedar)` stack, the policies, the extractors — lives in
//! `rauthy-cedar-app::make` and is shared verbatim with the CF Worker
//! (`../worker`). This file does only the platform-specific things:
//!   1. fetch JWKS with `ureq` (the Worker uses `worker::Fetch`)
//!   2. serve with `hyper`   (the Worker uses `worker::event(fetch)`)
//!   3. inject the native metrics sink (`NoopSink`) + rate limiter (`AllowAll`)
//!      — the CF Analytics-Engine / Rate-Limiting bindings only exist on the edge.
//!
//! Env: RAUTHY_ISSUER  RAUTHY_JWKS_URL  [RAUTHY_AUD=worker-client]  [PORT=8090]
//!
//!   curl -s -H "Authorization: Bearer $TOKEN" -X POST localhost:8090/demo.v1.Api/Read
//!   curl -s -H "Authorization: Bearer $TOKEN" -X POST localhost:8090/demo.v1.Api/Super

use std::convert::Infallible;
use std::sync::Arc;

use connectrpc_oidc::JwksVerifier;
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;
use tower::ServiceExt;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let issuer = std::env::var("RAUTHY_ISSUER").expect("set RAUTHY_ISSUER");
    let jwks_url = std::env::var("RAUTHY_JWKS_URL").expect("set RAUTHY_JWKS_URL");
    let aud = env_or("RAUTHY_AUD", "worker-client");
    let port: u16 = env_or("PORT", "8090").parse().unwrap();

    // (1) platform-specific: native JWKS fetch.
    println!("fetching JWKS from {jwks_url} ...");
    let jwks = ureq::get(&jwks_url).call()?.into_string()?;
    let verifier = Arc::new(JwksVerifier::from_jwks_json(&issuer, Some(aud), &jwks)?);

    // The shared app builds the ENTIRE crates/* middleware stack. The native
    // host injects the two platform-specific deps: a no-op metrics sink and an
    // always-allow rate limiter (the CF bindings only exist on the Worker).
    let svc = rauthy_cedar_app::make::<hyper::body::Incoming, _, _>(
        verifier,
        connectrpc_cf_metrics::NoopSink::new(),
        connectrpc_cf_rate_limit::AllowAll::new(),
    );

    // (2) platform-specific: hyper serve loop. The shared service's future is
    // !Send (same as on the Worker), so drive it on a current-thread LocalSet
    // with spawn_local rather than the Send-requiring tokio::task::spawn.
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    println!("full-stack server on http://127.0.0.1:{port}  (issuer {issuer})");
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            loop {
                let (stream, _) = listener.accept().await.expect("accept");
                let io = TokioIo::new(stream);
                let hyper_svc =
                    TowerToHyperService::new(svc.clone().map_err(|e: Infallible| match e {}));
                tokio::task::spawn_local(async move {
                    if let Err(e) = http1::Builder::new().serve_connection(io, hyper_svc).await {
                        eprintln!("conn error: {e}");
                    }
                });
            }
        })
        .await
}
