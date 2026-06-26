//! The shared full-stack ConnectRPC application — host-agnostic.
//!
//! This is the ONE composition point ([`make`]) for the entire `crates/*`
//! middleware stack, proven on NATIVE (`../server`, hyper) and CLOUDFLARE
//! (`../worker`, `worker::event(fetch)`) from the same `app`. The hosts add
//! only the two platform-specific bits — fetch the JWKS (native `ureq` / CF
//! `worker::Fetch`) and run the serve loop (native `hyper` / CF event) — plus
//! the two platform-specific middleware dependencies they *inject* into
//! [`make`]: the metrics **sink** and the rate **limiter**.
//!
//! ## The composed stack (outermost → innermost)
//!
//! ```text
//! request
//!   │
//!   ▼  1. TracingLayer            (connectrpc-cf-tracing)  transparent per-RPC span
//!   ▼  2. RateLimitLayer::enforce (connectrpc-cf-rate-limit) host-injected limiter, skip /healthz
//!   ▼  3. OidcLayer               (connectrpc-oidc)        verify Rauthy JWT → insert Session
//!   ▼  4. cedar_enforce           (connectrpc-guard)       PATH authz: /demo.v1.Api/X → Action X
//!   ▼  5. ConnectRpcService       (connectrpc)             + MetricsInterceptor (host-injected sink)
//!        └─ ApiImpl                                        + CedarInterceptor   (BODY-aware authz)
//! ```
//!
//! Layers 1–4 are `tower::Layer`s wrapping the service; the two interceptors
//! (5) run on the Connect RPC surface, after the envelope is decoded — which is
//! why the [`CedarInterceptor`] can authorize against the *decoded request
//! body* (`GetDoc`'s `doc_id`), a decision the path-based `cedar_enforce`
//! (layer 4) can never make because it only sees the route.
//!
//! The RPC is a REAL ConnectRPC service: a [`connectrpc::Router`] built from a
//! [`buffa`]-generated proto (`proto/demo/v1/api.proto`), wrapped in a
//! [`connectrpc::ConnectRpcService`] — the wasm-friendly `tower::Service` serve
//! path (no axum). `Read`/`Admin`/`Super` read the verified [`Session`] and
//! echo it; `GetDoc` additionally exercises body-aware Cedar.

use std::convert::Infallible;
use std::sync::{Arc, OnceLock};

use bytes::Bytes;
use cedar_policy::{Context, EntityUid, RestrictedExpression};
use connectrpc::{
    ConnectError, ConnectRpcService, ErrorCode, RequestContext, Response, Router, ServiceRequest,
    ServiceResult, UnaryRequest,
};
use connectrpc_cedar::{CedarRequest, action::action_from_path};
use connectrpc_cedar_interceptor::CedarInterceptor;
use connectrpc_cf_metrics::{MetricSink, MetricsInterceptor};
use connectrpc_cf_rate_limit::{IpKeyExtractor, RateLimitLayer, RateLimiter};
use connectrpc_cf_tracing::{CfFields, TracingLayer};
use connectrpc_guard::{CedarAuthorizer, JwksVerifier, cedar_enforce, load_authorizer};
use connectrpc_oidc::{OidcLayer, Session};
use http::Response as HttpResponse;
use tower::{Layer, Service};

// Re-export so hosts don't need a direct connectrpc dep just to name the type.
pub use connectrpc::ConnectRpcBody;

pub mod proto {
    connectrpc::include_generated!();
}

use proto::demo::v1::*;

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

/// The real RPC implementation. Reaching any method means the token verified
/// (OidcLayer) and Cedar allowed the action.
struct ApiImpl;

/// Echo the verified Session — the shared body of all four methods. Reaching
/// it means Cedar allowed this action; a 200 == allow, a 403 == Cedar deny.
fn reply(ctx: &RequestContext) -> ServiceResult<Reply> {
    // The guard's OidcLayer stamped the verified Session into the
    // http::Request extensions; the connect dispatcher forwarded those
    // into ctx.extensions() verbatim.
    let session = ctx.extensions().get::<Session>().ok_or_else(|| {
        ConnectError::new(
            ErrorCode::Internal,
            "no Session in context - guard misconfigured (OidcLayer must run first)",
        )
    })?;

    // Edition-2023 default presence is EXPLICIT, so scalar fields are
    // Option<_> on owned messages.
    Ok(Response::new(Reply {
        subject: Some(session.subject.clone()),
        roles: session.roles.clone(),
        ..Default::default()
    }))
}

// The generated trait returns `impl Encodable<Resp>`; concretely returning
// `Resp` refines it — intended, and matches the canonical handler pattern.
// `Super` is a Rust keyword, so the codegen names that trait method `super_`.
#[allow(refining_impl_trait)]
impl Api for ApiImpl {
    async fn read(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, Request>,
    ) -> ServiceResult<Reply> {
        reply(&ctx)
    }

    async fn admin(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, Request>,
    ) -> ServiceResult<Reply> {
        reply(&ctx)
    }

    async fn super_(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, Request>,
    ) -> ServiceResult<Reply> {
        reply(&ctx)
    }

    // Body-aware: reaching here means the CedarInterceptor authorized the
    // specific Doc built from `doc_id`. Echo the Session like the others.
    async fn get_doc(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetDocRequest>,
    ) -> ServiceResult<Reply> {
        reply(&ctx)
    }
}

fn to_set(items: &[String]) -> RestrictedExpression {
    RestrictedExpression::new_set(
        items
            .iter()
            .map(|s| RestrictedExpression::new_string(s.clone())),
    )
}

/// Body-aware Cedar extractor for `GetDoc`: pull `doc_id` off the decoded
/// [`GetDocRequest`] and authorize action `demo.v1.Api.GetDoc` on the specific
/// `Doc::"<doc_id>"`. The path-based layer (`cedar_enforce`) can only address
/// the route, so this is the interceptor's genuine, non-redundant job.
///
/// Returns `None` for any other procedure — the interceptor then passes
/// through (those routes are already authorized by the path layer).
fn getdoc_cedar(req: &UnaryRequest) -> Option<CedarRequest> {
    let path = req.ctx.path()?;
    // Only GetDoc is body-aware; everything else is the path layer's job.
    if !path.ends_with("/GetDoc") {
        return None;
    }
    let action = action_from_path(path)?;
    let session = req.ctx.extensions().get::<Session>()?;
    let msg = req.payload.message::<GetDocRequest>().ok()?;
    let doc_id = msg.doc_id.clone().unwrap_or_default();

    let principal: EntityUid = format!(r#"User::"{}""#, session.subject).parse().ok()?;
    let resource: EntityUid = format!(r#"Doc::"{doc_id}""#).parse().ok()?;
    let context = Context::from_pairs([
        ("roles".to_string(), to_set(&session.roles)),
        ("scopes".to_string(), to_set(&session.scopes)),
    ])
    .ok()?;
    Some(CedarRequest {
        principal,
        action,
        resource,
        context,
    })
}

/// Build the full middleware stack for a verifier — the ONE composition point.
///
/// Generic over the body `B`, so native (`hyper::body::Incoming`) and CF
/// (`worker::Body`) call the identical constructor. The two platform-specific
/// middleware dependencies are *injected* by the host, not chosen here:
///
/// - `sink: M` — the metrics [`MetricSink`]. Native passes
///   `connectrpc_cf_metrics::NoopSink`; the Worker passes an Analytics Engine
///   sink built from its `env` binding.
/// - `limiter: L` — the [`RateLimiter`]. Native passes
///   `connectrpc_cf_rate_limit::AllowAll`; the Worker passes a CF Rate Limiting
///   sink built from its `env` binding.
///
/// See the module docs for the full layer order.
pub fn make<B, M, L>(
    verifier: Arc<JwksVerifier>,
    sink: M,
    limiter: L,
) -> impl Service<http::Request<B>, Response = HttpResponse<ConnectRpcBody>, Error = Infallible> + Clone
where
    B: http_body::Body<Data = Bytes> + Send + 'static,
    B::Error: std::error::Error + Send + Sync + 'static,
    M: MetricSink,
    L: RateLimiter,
{
    let authorizer = authorizer();

    // (5) The REAL ConnectRPC service. Two interceptors run on the RPC surface
    // after envelope decode:
    //   - MetricsInterceptor: time each RPC → host-injected sink.
    //   - CedarInterceptor: BODY-aware authz (reads GetDoc's doc_id) — the job
    //     the path layer can't do. Enforce mode; /healthz isn't an RPC path so
    //     skip_paths is a belt-and-braces guard.
    let router = Arc::new(ApiImpl).register(Router::new());
    let service = ConnectRpcService::new(router)
        .with_interceptor(MetricsInterceptor::new(sink))
        .with_interceptor(
            CedarInterceptor::enforce(Arc::clone(&authorizer), getdoc_cedar)
                .skip_paths(["/healthz"]),
        );

    // (4) PATH authz: /demo.v1.Api/X → Action::"demo.v1.Api.X" on Api::"main".
    let cedar = cedar_enforce(Arc::clone(&authorizer), "main", service);

    // (3) OIDC AuthN: verify the Rauthy JWT → insert the Session. /healthz is public.
    let authed = OidcLayer::new(verifier)
        .skip_paths(["/healthz"])
        .layer(cedar);

    // (2) Rate limit (host-injected limiter), then (1) tracing — outermost.
    let limited = RateLimitLayer::enforce(limiter, IpKeyExtractor::new())
        .skip_paths(["/healthz"])
        .layer(authed);

    // (1) Transparent per-RPC tracing span. Native CF fields are empty (no
    // worker::Cf); on the Worker the host can wrap with its own Cf extractor.
    TracingLayer::new(|_req: &http::Request<B>| CfFields::empty()).layer(limited)
}
