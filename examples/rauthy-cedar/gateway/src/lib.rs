//! GATEWAY Worker — the front door of the rauthy-cedar multi-Worker shape.
//!
//! This Worker SERVES a tiny `gateway.v1.GatewayService` (one RPC: `ProxyRead`)
//! and, in the handler, turns around and CALLS the auth-protected BACKEND
//! Worker's `demo.v1.Api/Read` over a Cloudflare `[[services]]` binding using
//! connyay's [`FetcherTransport`]. No DNS, no TLS, no public-internet hop — the
//! runtime routes the subrequest straight to the backend via the binding.
//!
//! It is the REAL inter-Worker ConnectRPC demonstration (mirrors connyay's
//! `examples/multi/gateway-worker`): the gateway enforces nothing itself; it
//! forwards the caller's `Authorization: Bearer` header onto the backend call,
//! so the backend's full OIDC -> Cedar stack (`rauthy_cedar_app::make`) is what
//! actually authorizes the request. A valid token => backend 200 => gateway
//! returns the echoed Session; no token => backend 401 propagates back through
//! the gateway as a Connect `unauthenticated` error.
//!
//! Why a per-request client: `worker::Fetcher` (from `env.service`) is not
//! `Clone`/`'static`-shareable across the isolate the way a long-lived client
//! would need, and the forwarded auth token differs per request — so we build
//! the `ApiClient` inside the handler from the binding + the caller's header.
//!
//! Bindings (wrangler.toml): `[[services]] binding = "API"` -> the backend
//! Worker's `name` (rauthy-cedar-api).

#![allow(refining_impl_trait)]

use std::sync::Arc;

use connectrpc::client::{CallOptions, ClientConfig};
use connectrpc::{
    ConnectError, ConnectRpcBody, ConnectRpcService, RequestContext, Response, Router as RpcRouter,
    ServiceRequest, ServiceResult,
};
use tower::Service;
use worker::{Context, Env, HttpRequest, event};

use connectrpc_workers::FetcherTransport;
// The backend client type is REUSED from the shared app — the gateway never
// re-generates demo.v1; it depends on the app for `ApiClient` + `Request`.
use rauthy_cedar_app::proto::demo::v1::{ApiClient, Request as ApiRequest};

// The gateway's OWN generated front-door service (gateway.v1), compiled by
// build.rs from proto/gateway/v1/gateway.proto.
mod proto {
    connectrpc::include_generated!();
}
use proto::gateway::v1::{
    GatewayService, GatewayServiceExt, ProxyReadRequest, ProxyReadResponse,
};

/// Service-binding name in `wrangler.toml` for the upstream backend (the api
/// Worker that runs the OIDC -> Cedar middleware stack).
const API_BINDING: &str = "API";

/// Sentinel base URI for the backend client. The authority is irrelevant for
/// service-binding fetches — the runtime routes via the binding, not DNS — but
/// ConnectRPC needs a syntactically-valid base URI for path construction.
const API_BASE_URI: &str = "http://api/";

/// Constant label so the e2e can assert the request really traversed the
/// service binding to the backend.
const UPSTREAM_LABEL: &str = "rauthy-cedar-api (service binding)";

struct GatewayImpl {
    env: Env,
}

impl GatewayService for GatewayImpl {
    async fn proxy_read(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ProxyReadRequest>,
    ) -> ServiceResult<ProxyReadResponse> {
        // Build the backend client from the service binding. `env.service`
        // resolves the `[[services]] binding = "API"` stanza.
        let fetcher = self
            .env
            .service(API_BINDING)
            .map_err(|e| ConnectError::unavailable(format!("API service binding: {e}")))?;
        let transport = FetcherTransport::new(fetcher);
        let config = ClientConfig::new(API_BASE_URI.parse().unwrap());
        let client = ApiClient::new(transport, config);

        // Forward the caller's Authorization header onto the backend call, so
        // the backend's OidcLayer verifies the SAME Rauthy token. Without it the
        // backend returns 401, which we propagate verbatim (see below).
        let mut options = CallOptions::default();
        if let Some(auth) = ctx.header(http::header::AUTHORIZATION) {
            options = options
                .try_with_header(http::header::AUTHORIZATION, auth.clone())
                .map_err(|e| ConnectError::internal(format!("forward auth header: {e}")))?;
        }

        // The REAL inter-Worker call: gateway -> service binding -> backend
        // `demo.v1.Api/Read`. The backend runs OIDC + Cedar; a deny surfaces
        // here as a Connect error (e.g. unauthenticated / permission_denied),
        // which `respond_with_errors` maps back to the proper HTTP status — so
        // the backend's 401/403 propagates THROUGH the gateway unchanged.
        let reply = client
            .read_with_options(ApiRequest::default(), options)
            .await?;

        let view = reply.view();
        Response::ok(ProxyReadResponse {
            // Edition-2023 default presence is EXPLICIT, so scalar fields are
            // Option<_> on owned messages.
            subject: Some(view.subject.unwrap_or_default().to_string()),
            roles: view.roles.iter().map(|r| r.to_string()).collect(),
            upstream: Some(UPSTREAM_LABEL.to_string()),
            ..Default::default()
        })
    }
}

#[event(fetch, respond_with_errors)]
async fn fetch(
    req: HttpRequest,
    env: Env,
    _ctx: Context,
) -> worker::Result<http::Response<ConnectRpcBody>> {
    let gateway = GatewayImpl { env };
    let router = Arc::new(gateway).register(RpcRouter::new());
    let mut svc = ConnectRpcService::new(router);
    Ok(svc.call(req).await.unwrap())
}
