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
use std::future::{Ready, ready};
use std::sync::{Arc, OnceLock};
use std::task::{Context as TaskContext, Poll};

use bytes::Bytes;
use cedar_policy::{Context, EntityUid, RestrictedExpression};
use connectrpc::{
    ConnectError, ConnectRpcService, ErrorCode, RequestContext, Response, Router, ServiceRequest,
    ServiceResult, UnaryRequest,
};
use connectrpc_cedar::{CedarLayer, CedarRequest, action::action_from_path};
use connectrpc_cedar_interceptor::CedarInterceptor;
use connectrpc_cf_metrics::{MetricSink, MetricsInterceptor};
use connectrpc_cf_rate_limit::{IpKeyExtractor, RateLimitLayer, RateLimiter};
use connectrpc_cf_tracing::{CfFields, TracingLayer};
use connectrpc_guard::{CedarAuthorizer, JwksVerifier, load_authorizer, session_to_cedar};
use connectrpc_oidc::{OidcLayer, Session};
use http::Response as HttpResponse;
use http_body_util::Full;
use tower::{Layer, Service};

// Re-export so hosts don't need a direct connectrpc dep just to name the type.
pub use connectrpc::ConnectRpcBody;

pub mod proto {
    connectrpc::include_generated!();
}

use proto::demo::v1::*;

/// A tiny `tower::Service` that answers `GET /healthz` with a real `200 OK`
/// and passes every other request through to the inner service unchanged.
///
/// This is the ONE shared health mechanism for BOTH hosts. The native server
/// and the CF Worker both call [`make`], so wiring it as the outermost layer
/// here means neither host has to add a health route by hand. It is
/// deliberately NOT `connectrpc-health`: that crate needs the `server` feature,
/// which pulls `mio` and won't compile on `wasm32`. `/healthz` is not a Connect
/// RPC path — without this short-circuit the request would fall through to the
/// `ConnectRpcService` router, which has no `/healthz` route, and 404.
#[derive(Clone)]
struct HealthService<S> {
    inner: S,
}

impl<S, B> Service<http::Request<B>> for HealthService<S>
where
    S: Service<http::Request<B>, Response = HttpResponse<ConnectRpcBody>, Error = Infallible>,
{
    type Response = HttpResponse<ConnectRpcBody>;
    type Error = Infallible;
    // An `Either`-style future with the immediate health reply in one arm and
    // the inner service future in the other. Hand-rolled (not `futures::Either`,
    // which isn't a dep) so it stays minimal and adds NO `Send` bound — the
    // whole stack is `!Send` on wasm32, so a boxed-Send future wouldn't compile
    // on the Worker.
    type Future = HealthFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<B>) -> Self::Future {
        if req.method() == http::Method::GET && req.uri().path() == "/healthz" {
            let mut resp =
                HttpResponse::new(ConnectRpcBody::Full(Full::new(Bytes::from_static(b"ok"))));
            *resp.status_mut() = http::StatusCode::OK;
            resp.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("text/plain; charset=utf-8"),
            );
            HealthFuture::Health(ready(Ok(resp)))
        } else {
            HealthFuture::Inner(self.inner.call(req))
        }
    }
}

// Hand-rolled `Either` future for `HealthService`. Projection is sound because
// neither arm is ever moved out: the enum is only ever pinned and polled in
// place. We use `unsafe` pin-projection rather than pulling in a proc-macro dep.
// `Inner(F)` carries the whole composed-stack future, so it dwarfs the tiny
// `Health(Ready)` variant. Boxing `Inner` to equalize sizes would add a heap
// allocation to EVERY real request just to serve the rare /healthz path — the
// wrong trade for a liveness wrapper. Keep the size asymmetry on purpose.
#[allow(clippy::large_enum_variant)]
enum HealthFuture<F> {
    Health(Ready<Result<HttpResponse<ConnectRpcBody>, Infallible>>),
    Inner(F),
}

impl<F> std::future::Future for HealthFuture<F>
where
    F: std::future::Future<Output = Result<HttpResponse<ConnectRpcBody>, Infallible>>,
{
    type Output = Result<HttpResponse<ConnectRpcBody>, Infallible>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        // SAFETY: we never move the inner field out of the pinned enum; we only
        // re-pin it in place and poll it. The enum variant is never swapped
        // after construction, so the projection is structurally pinned.
        unsafe {
            match self.get_unchecked_mut() {
                HealthFuture::Health(fut) => std::pin::Pin::new_unchecked(fut).poll(cx),
                HealthFuture::Inner(fut) => std::pin::Pin::new_unchecked(fut).poll(cx),
            }
        }
    }
}

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
    //
    // GetDoc is deliberately EXEMPT from the path layer: its action
    // (`demo.v1.Api.GetDoc`) is schema-scoped to a `Doc` resource and its only
    // policy permits `Doc::"public"`, but the path layer can only address the
    // route's `Api::"main"` resource — so evaluating GetDoc here would always
    // default-deny (403) before the body-aware interceptor ever runs. Returning
    // `None` for GetDoc makes the path layer pass it through; the
    // `CedarInterceptor` (layer 5, post-decode) is the genuine, sole decider
    // for GetDoc, reading `doc_id` from the body. Every other path is still
    // path-authorized exactly as before via the shared `session_to_cedar`.
    let cedar = CedarLayer::enforce(Arc::clone(&authorizer), |req: &http::Request<B>| {
        if req.uri().path().ends_with("/GetDoc") {
            return None;
        }
        session_to_cedar(req, "main")
    })
    .layer(service);

    // (3) OIDC AuthN: verify the Rauthy JWT → insert the Session. /healthz is public.
    let authed = OidcLayer::new(verifier)
        .skip_paths(["/healthz"])
        .layer(cedar);

    // (2) Rate limit (host-injected limiter), then (1) tracing.
    let limited = RateLimitLayer::enforce(limiter, IpKeyExtractor::new())
        .skip_paths(["/healthz"])
        .layer(authed);

    // (1) Transparent per-RPC tracing span. Native CF fields are empty (no
    // worker::Cf); on the Worker the host can wrap with its own Cf extractor.
    let traced = TracingLayer::new(|_req: &http::Request<B>| CfFields::empty()).layer(limited);

    // (0) OUTERMOST: real `GET /healthz` → 200 for BOTH hosts (native + Worker),
    // wired once here so neither host adds a health route by hand. Everything
    // else passes straight through to the tracing layer above. See
    // [`HealthService`] for why this isn't `connectrpc-health` (wasm/mio).
    HealthService { inner: traced }
}
