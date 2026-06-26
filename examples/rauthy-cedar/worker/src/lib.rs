//! CLOUDFLARE WORKER host for the shared `rauthy-cedar-app` — the BACKEND (api)
//! Worker. wrangler `name = "rauthy-cedar-api"`. It SERVES the guarded
//! `demo.v1.Api` (+ Health + Reflection) with the full OIDC -> Cedar middleware
//! stack. The sibling `../gateway` Worker fronts it over a `[[services]]`
//! binding and proxies `ProxyRead` here via connyay's `FetcherTransport` — see
//! `../gateway` and `../README.md` for that multi-Worker shape.
//!
//! Identical to the native `../server` except for the platform bits:
//!   1. fetch JWKS with `worker::Fetch` (the server uses `ureq`)
//!   2. serve via `worker::event(fetch)` (the server uses `hyper`)
//!   3. inject the two CF-specific middleware deps into `make()`:
//!      - an Analytics Engine [`MetricSink`] (server passes `NoopSink`)
//!      - a CF Rate Limiting [`RateLimiter`] (server passes `AllowAll`)
//!
//! Everything else — the full `tracing → rate-limit → oidc → cedar →
//! service(+metrics+body-cedar)` stack, the policies, the extractors — is the
//! SAME `rauthy_cedar_app::make` the native host calls. That's ConnectRPC's
//! promise: one app, native and edge.
//!
//! Bindings (wrangler.toml): RAUTHY_ISSUER, RAUTHY_JWKS_URL, RAUTHY_AUD (vars);
//! AE (analytics_engine_datasets); RL (ratelimit).

use std::sync::Arc;

use async_trait::async_trait;
use connectrpc_cf_metrics::MetricSink;
use connectrpc_cf_rate_limit::{RateLimitOutcome, RateLimiter};
use connectrpc_oidc::JwksVerifier;
use rauthy_cedar_app::{ConnectRpcBody, make};
use tokio::sync::OnceCell;
use tower::Service;
use worker::{
    AnalyticsEngineDataPointBuilder, AnalyticsEngineDataset, Context, Env, HttpRequest,
    RateLimiter as CfRl, event,
};

/// Analytics Engine sink — the CF impl of [`MetricSink`]. AE's
/// `write_data_point` is synchronous; the async block resolves immediately.
/// AE schema here: blobs[0]=metric_name, blobs[1..]=label values,
/// doubles[0]=value.
struct AeMetricSink(Arc<AnalyticsEngineDataset>);

#[async_trait]
impl MetricSink for AeMetricSink {
    async fn counter(&self, name: &str, value: u64, labels: &[(&str, &str)]) {
        let mut point = AnalyticsEngineDataPointBuilder::new().add_blob(name);
        for (_, v) in labels {
            point = point.add_blob(*v);
        }
        let _ = self.0.write_data_point(&point.add_double(value as f64).build());
    }

    async fn histogram(&self, name: &str, value: f64, labels: &[(&str, &str)]) {
        let mut point = AnalyticsEngineDataPointBuilder::new().add_blob(name);
        for (_, v) in labels {
            point = point.add_blob(*v);
        }
        let _ = self.0.write_data_point(&point.add_double(value).build());
    }
}

/// CF Rate Limiting binding — the CF impl of [`RateLimiter`]. Maps the
/// binding's `{ success }` outcome onto the crate's three-way outcome.
///
/// Falls back to [`AllowAll`] when the `RL` binding is absent. This matters for
/// the MULTI-WORKER local dev shape (../gateway): when this backend runs as an
/// AUXILIARY Worker under `wrangler dev -c gateway -c worker`, miniflare does
/// NOT provision the auxiliary Worker's `[[ratelimits]]` binding, so
/// `env.rate_limiter("RL")` errors. Rather than 500 the whole request before
/// OIDC/Cedar even run, we degrade rate-limiting to allow-all — exactly what
/// `AllowAll`'s own docs sanction for "hosts that don't provision a CF
/// rate-limit binding (the native server, local dev)". In production and in the
/// single-worker dev the `RL` binding is present, so the real limiter is used.
enum CfRateLimiter {
    Cf(Arc<CfRl>),
    Allow,
}

#[async_trait]
impl RateLimiter for CfRateLimiter {
    async fn check(&self, key: String) -> RateLimitOutcome {
        match self {
            CfRateLimiter::Cf(rl) => match rl.limit(key).await {
                Ok(o) if o.success => RateLimitOutcome::Allowed,
                Ok(_) => RateLimitOutcome::Exceeded,
                Err(e) => RateLimitOutcome::Error(e.to_string()),
            },
            CfRateLimiter::Allow => RateLimitOutcome::Allowed,
        }
    }
}

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
    // (3) platform-specific: build the CF metrics sink + rate limiter from the
    // env bindings and inject them into the SAME shared stack the native host
    // builds. The bindings (AE, RL) are declared in wrangler.toml.
    let sink = AeMetricSink(Arc::new(env.analytics_engine("AE")?));
    // The `RL` binding is absent when this backend runs as an AUXILIARY Worker
    // under the multi-Worker `wrangler dev -c gateway -c worker` (miniflare
    // doesn't provision auxiliary-Worker rate-limit bindings). Degrade to
    // allow-all there instead of 500-ing before OIDC/Cedar — see CfRateLimiter.
    let limiter = match env.rate_limiter("RL") {
        Ok(rl) => CfRateLimiter::Cf(Arc::new(rl)),
        Err(_) => CfRateLimiter::Allow,
    };
    let mut svc = make::<worker::Body, _, _>(verifier, sink, limiter);
    Ok(svc.call(req).await.unwrap())
}
