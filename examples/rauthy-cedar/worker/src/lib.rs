//! CLOUDFLARE WORKER host for the shared `rauthy-cedar-app`.
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
use connectrpc::client::ClientConfig;
use connectrpc_cf_metrics::MetricSink;
use connectrpc_cf_rate_limit::{RateLimitOutcome, RateLimiter};
use connectrpc_oidc::JwksVerifier;
use connectrpc_workers::FetchTransport;
use rauthy_cedar_app::proto::demo::v1::{ApiClient, Request as ApiRequest};
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
struct CfRateLimiter(Arc<CfRl>);

#[async_trait]
impl RateLimiter for CfRateLimiter {
    async fn check(&self, key: String) -> RateLimitOutcome {
        match self.0.limit(key).await {
            Ok(o) if o.success => RateLimitOutcome::Allowed,
            Ok(_) => RateLimitOutcome::Exceeded,
            Err(e) => RateLimitOutcome::Error(e.to_string()),
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

/// Build a plain `200` JSON response in the Worker's body type.
fn json_response(json: String) -> http::Response<ConnectRpcBody> {
    let mut resp = http::Response::new(ConnectRpcBody::Full(http_body_util::Full::new(
        bytes::Bytes::from(json),
    )));
    resp.headers_mut().insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/json"),
    );
    resp
}

/// `/client-demo` — the CLIENT half of "Connect on Workers".
///
/// This route is OUTSIDE the guarded `make()` server stack: it is not
/// auth-gated, because it demonstrates the Worker acting as a Connect *client*,
/// not serving the protected API. It builds a [`FetchTransport`] (connyay
/// `connectrpc-workers`, wrapping the global `worker::Fetch`) pointed at the
/// `CLIENT_DEMO_TARGET` Connect service, constructs the generated `ApiClient`
/// from the SHARED app's proto, and calls `demo.v1.Api/Read`.
///
/// `worker::Fetch` futures are `!Send`; the crate's `FetchTransport` already
/// wraps the call in `SendFuture`/`SendWrapper` to satisfy `ClientTransport`'s
/// `Send` bound, and the Workers isolate is single-threaded, so no extra
/// `.into_send()` is needed here.
async fn client_demo(env: &Env) -> worker::Result<http::Response<ConnectRpcBody>> {
    let target = env
        .var("CLIENT_DEMO_TARGET")
        .ok()
        .map(|v| v.to_string())
        .filter(|s| !s.is_empty());

    let Some(target) = target else {
        return Ok(json_response(
            "{\"client_demo\":\"unset\",\"hint\":\"set CLIENT_DEMO_TARGET in \
             wrangler.toml [vars] to a Connect service URL (e.g. \
             http://localhost:8090 or this Worker's own public origin) and call \
             /client-demo again to drive demo.v1.Api/Read via the \
             connectrpc-workers FetchTransport\"}"
                .to_string(),
        ));
    };

    let uri: http::Uri = target
        .parse()
        .map_err(|e| worker::Error::RustError(format!("bad CLIENT_DEMO_TARGET: {e}")))?;
    let transport = FetchTransport::new(uri)?;
    // ClientConfig::new defaults to Protocol::Connect + proto codec — exactly
    // what Workers fetch subrequests support (no raw HTTP/2 / gRPC trailers).
    let config = ClientConfig::new(target.parse().map_err(|e| {
        worker::Error::RustError(format!("bad CLIENT_DEMO_TARGET uri: {e}"))
    })?);
    let client = ApiClient::new(transport, config);

    // demo.v1.Api/Read takes an empty Request{}. This is a REAL outbound
    // Connect call; an unauthenticated target will return a Connect error,
    // which we surface verbatim so the round trip is observable either way.
    match client.read(ApiRequest::default()).await {
        Ok(resp) => {
            let view = resp.view();
            let roles = view
                .roles
                .iter()
                .map(|r| format!("\"{}\"", r.replace('"', "'")))
                .collect::<Vec<_>>()
                .join(",");
            Ok(json_response(format!(
                "{{\"client_demo\":\"ok\",\"target\":\"{}\",\"subject\":\"{}\",\"roles\":[{}]}}",
                target,
                view.subject.unwrap_or_default().replace('"', "'"),
                roles,
            )))
        }
        Err(e) => Ok(json_response(format!(
            "{{\"client_demo\":\"error\",\"target\":\"{}\",\"error\":\"{}\"}}",
            target,
            e.to_string().replace('"', "'"),
        ))),
    }
}

#[event(fetch, respond_with_errors)]
async fn fetch(
    req: HttpRequest,
    env: Env,
    _ctx: Context,
) -> worker::Result<http::Response<ConnectRpcBody>> {
    // CLIENT-transport demo: handled BEFORE the guarded server stack so it is
    // not auth-gated. Proves the Worker can call a Connect service outbound.
    if req.uri().path() == "/client-demo" {
        return client_demo(&env).await;
    }

    let verifier = verifier(&env).await?;
    // (3) platform-specific: build the CF metrics sink + rate limiter from the
    // env bindings and inject them into the SAME shared stack the native host
    // builds. The bindings (AE, RL) are declared in wrangler.toml.
    let sink = AeMetricSink(Arc::new(env.analytics_engine("AE")?));
    let limiter = CfRateLimiter(Arc::new(env.rate_limiter("RL")?));
    let mut svc = make::<worker::Body, _, _>(verifier, sink, limiter);
    Ok(svc.call(req).await.unwrap())
}
