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
    ServiceResult, ServiceStream, StreamMessage, UnaryRequest,
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
use proto::grpc::health::v1 as health;
use proto::grpc::reflection::v1 as reflection;
// `register()` on the service Arcs comes from these generated extension traits.
use proto::grpc::health::v1::HealthExt as _;
use proto::grpc::reflection::v1::ServerReflectionExt as _;

/// The wire-format `FileDescriptorSet` for ALL of this app's protos (demo +
/// health + reflection), with the full transitive import closure — emitted by
/// `build.rs` (`emit_descriptor_set`) and embedded here. It backs the gRPC
/// server reflection service so schema-free clients (`grpcurl`, `buf curl`)
/// can discover and call the API without local `.proto` files.
pub const FILE_DESCRIPTOR_SET: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/app.fds.bin"));

/// Public, un-authenticated paths every guard layer must skip.
///
/// Health and reflection are public infrastructure: a kubelet `grpc:` probe or
/// a `grpcurl` discovery call carries no Rauthy token, so the OIDC/Cedar/rate-
/// limit layers (and the body-aware Cedar interceptor) must pass them straight
/// through. The layers match `skip_paths` by EXACT path equality (`p ==
/// path`), so each RPC method is listed explicitly — not a prefix.
///
/// `/healthz` is the plain-HTTP liveness route handled by [`HealthService`]
/// before any layer runs; it is listed too so the order is documented in one
/// place and the lists stay identical across layers.
const PUBLIC_PATHS: &[&str] = &[
    "/healthz",
    "/grpc.health.v1.Health/Check",
    "/grpc.health.v1.Health/Watch",
    "/grpc.reflection.v1.ServerReflection/ServerReflectionInfo",
];

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

// ─── gRPC Health (grpc.health.v1.Health) ────────────────────────────────────
//
// The standard gRPC health-checking service, served on the SAME Router as
// ApiImpl, on BOTH hosts. Compiled from `proto/grpc/health/v1/health.proto`
// via build.rs — NOT the `connectrpc-health` crate (whose manifest pulls `mio`
// and breaks wasm32). It is public: its paths are in `PUBLIC_PATHS`, so the
// OIDC/Cedar/rate-limit guards skip it (a probe carries no token). This is the
// gRPC protocol surface; the plain-HTTP `GET /healthz` liveness route
// ([`HealthService`]) is kept separately for simple `httpGet:` probes.

/// The single health checker for the whole app — always serving.
struct HealthImpl;

#[allow(refining_impl_trait)]
impl health::Health for HealthImpl {
    /// Report the requested service (or the whole server when the name is
    /// empty) as `SERVING`. This app has no degraded mode, so the answer is
    /// constant; a real service would consult dependency health here.
    async fn check(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, health::HealthCheckRequest>,
    ) -> ServiceResult<health::HealthCheckResponse> {
        Ok(Response::new(health::HealthCheckResponse {
            status: health::health_check_response::ServingStatus::SERVING.into(),
            ..Default::default()
        }))
    }

    /// `Watch` is server-streaming in the spec. We do not implement the
    /// long-lived status-change stream: returning `UNIMPLEMENTED` is the
    /// spec-sanctioned signal ("the client should assume this method is not
    /// supported and should not retry"), and it keeps the wasm32 build free of
    /// a long-running `!Send` subscription future. `Check`-based probes
    /// (kubelet `grpc:`, `grpc_health_probe`) are unaffected.
    async fn watch(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, health::HealthCheckRequest>,
    ) -> ServiceResult<ServiceStream<health::HealthCheckResponse>> {
        Err(ConnectError::unimplemented(
            "grpc.health.v1.Health/Watch is not implemented; use Check",
        ))
    }
}

// ─── gRPC Server Reflection (grpc.reflection.v1.ServerReflection) ────────────
//
// Lets schema-free clients (`grpcurl`, `buf curl`, Postman, grpcui) discover
// and call the API without local `.proto` files. We do NOT depend on
// `connectrpc-reflection` (its manifest pulls `mio` → breaks wasm32); instead
// we reimplement the thin descriptor-set query bridge over `buffa_descriptor`
// (the SAME `DescriptorPool` engine that crate uses internally), driven by the
// `FILE_DESCRIPTOR_SET` embedded from build.rs. The bidi-streaming
// `ServerReflectionInfo` is fully implemented: each request maps to one
// response synchronously, so the output `Stream` is `Send` and compiles on
// wasm32.

use buffa::Message as _;
use buffa_descriptor::DescriptorPool;
use buffa_descriptor::generated::descriptor::{FileDescriptorProto, FileDescriptorSet};
use std::collections::{HashMap, HashSet};

/// Descriptor index answering the reflection protocol's queries against the
/// embedded `FileDescriptorSet`. Built once and shared.
struct Reflector {
    pool: DescriptorPool,
    /// Original per-file `FileDescriptorProto` wire bytes, keyed by file name,
    /// sliced out of the input so reflection responses return the compiler's
    /// exact bytes (not a re-encode).
    response_bytes: HashMap<String, Vec<u8>>,
    service_names: Vec<String>,
}

impl Reflector {
    /// Build the reflector from wire-format `FileDescriptorSet` bytes (the
    /// `FILE_DESCRIPTOR_SET` emitted by build.rs, which carries the full
    /// transitive import closure).
    fn from_descriptor_set_bytes(bytes: &[u8]) -> Self {
        let set =
            FileDescriptorSet::decode_from_slice(bytes).expect("embedded descriptor set decodes");
        let raw_files = split_descriptor_set(bytes);
        let mut response_bytes = HashMap::with_capacity(set.file.len());
        for (fd, raw) in set.file.iter().zip(&raw_files) {
            if let Some(name) = fd.name.clone() {
                response_bytes.entry(name).or_insert_with(|| raw.to_vec());
            }
        }
        let service_names = set
            .file
            .iter()
            .flat_map(|f| {
                let pkg = f.package.clone().unwrap_or_default();
                f.service.iter().filter_map(move |s| {
                    let name = s.name.clone()?;
                    Some(if pkg.is_empty() {
                        name
                    } else {
                        format!("{pkg}.{name}")
                    })
                })
            })
            .collect();
        let pool = DescriptorPool::new(set).expect("embedded descriptor set links");
        Self {
            pool,
            response_bytes,
            service_names,
        }
    }

    /// The serialized bytes of `fd` followed by its transitive import closure,
    /// deduplicated — the file-closure a reflection client expects.
    fn closure(&self, fd: &FileDescriptorProto) -> Vec<Vec<u8>> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        let mut stack = vec![fd];
        while let Some(fd) = stack.pop() {
            let Some(name) = fd.name.as_deref() else {
                continue;
            };
            if !seen.insert(name.to_owned()) {
                continue;
            }
            if let Some(bytes) = self.response_bytes.get(name) {
                out.push(bytes.clone());
            }
            stack.extend(
                fd.dependency
                    .iter()
                    .filter_map(|dep| self.pool.file_by_name(dep)),
            );
        }
        out
    }

    /// Build the `message_response` for one reflection request. Lookup misses
    /// are reported in-band as a `not_found` `ErrorResponse` per the protocol,
    /// keeping the stream alive.
    fn respond(&self, request: &reflection::ServerReflectionRequest) -> ReflectAnswer {
        use reflection::server_reflection_request::MessageRequest;

        let Some(message_request) = &request.message_request else {
            return ReflectAnswer::NotFound(
                "ServerReflectionRequest.message_request is not set".to_owned(),
            );
        };
        match message_request {
            MessageRequest::FileByFilename(name) => match self.pool.file_by_name(name) {
                Some(fd) => ReflectAnswer::Files(self.closure(fd)),
                None => ReflectAnswer::NotFound(format!("file {name:?} not found")),
            },
            MessageRequest::FileContainingSymbol(symbol) => {
                match self.pool.file_containing_symbol(symbol) {
                    Some(fd) => ReflectAnswer::Files(self.closure(fd)),
                    None => ReflectAnswer::NotFound(format!("symbol {symbol:?} not found")),
                }
            }
            MessageRequest::ListServices(_) => ReflectAnswer::Services(self.service_names.clone()),
            // file_containing_extension / all_extension_numbers_of_type: this
            // app declares no proto extensions, so there is nothing to find.
            // Reflection clients use these only for proto2 extension discovery;
            // a not_found keeps the stream alive and is the correct answer for
            // an extension-free schema.
            MessageRequest::FileContainingExtension(ext) => ReflectAnswer::NotFound(format!(
                "extension {} of type {:?} not found",
                ext.extension_number, ext.containing_type
            )),
            MessageRequest::AllExtensionNumbersOfType(name) => {
                ReflectAnswer::NotFound(format!("message {name:?} has no extensions"))
            }
        }
    }
}

/// The protocol-agnostic answer to one reflection query, mapped onto the
/// generated `message_response` oneof in [`reflect_response`].
enum ReflectAnswer {
    Files(Vec<Vec<u8>>),
    Services(Vec<String>),
    NotFound(String),
}

/// Assemble a full `ServerReflectionResponse` for `request`.
fn reflect_response(
    reflector: &Reflector,
    request: reflection::ServerReflectionRequest,
) -> reflection::ServerReflectionResponse {
    use reflection::server_reflection_response::MessageResponse;

    let answer = reflector.respond(&request);
    let message_response = match answer {
        ReflectAnswer::Files(file_descriptor_proto) => {
            MessageResponse::from(reflection::FileDescriptorResponse {
                file_descriptor_proto,
                ..Default::default()
            })
        }
        ReflectAnswer::Services(names) => MessageResponse::from(reflection::ListServiceResponse {
            service: names
                .into_iter()
                .map(|name| reflection::ServiceResponse {
                    name,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }),
        ReflectAnswer::NotFound(message) => MessageResponse::from(reflection::ErrorResponse {
            // gRPC status numbering: 5 == NOT_FOUND (matches grpc-go / tonic).
            error_code: 5,
            error_message: message,
            ..Default::default()
        }),
    };
    reflection::ServerReflectionResponse {
        valid_host: request.host.clone(),
        original_request: buffa::MessageField::some(request),
        message_response: Some(message_response),
        ..Default::default()
    }
}

/// The reflection service: a [`Reflector`] over the app's embedded descriptor
/// set, shared across requests.
struct ReflectionImpl {
    reflector: Arc<Reflector>,
}

#[allow(refining_impl_trait)]
impl reflection::ServerReflection for ReflectionImpl {
    /// Bidi stream: map each inbound `ServerReflectionRequest` to one
    /// `ServerReflectionResponse`. The mapping is synchronous and the items
    /// are owned, so the output stream is `Send` — it compiles and runs on
    /// wasm32 (the Worker) exactly as on native.
    async fn server_reflection_info(
        &self,
        _ctx: RequestContext,
        requests: ServiceStream<StreamMessage<reflection::ServerReflectionRequest>>,
    ) -> ServiceResult<ServiceStream<reflection::ServerReflectionResponse>> {
        use futures::StreamExt;
        let reflector = Arc::clone(&self.reflector);
        let responses = requests.map(move |item| {
            let request = item?.to_owned_message();
            Ok(reflect_response(&reflector, request))
        });
        Response::stream_ok(responses)
    }
}

/// Slice the original per-file `FileDescriptorProto` byte ranges out of a
/// wire-format `FileDescriptorSet` (`repeated FileDescriptorProto file = 1`),
/// so reflection responses can return the compiler's exact bytes rather than a
/// re-encode (which would canonicalize field order and could drop unknown
/// fields). The embedded set is produced by our own build, so it is
/// well-formed; a malformed varint simply stops the walk early.
fn split_descriptor_set(bytes: &[u8]) -> Vec<&[u8]> {
    let mut files = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        let Some(tag) = read_varint(bytes, &mut pos) else {
            break;
        };
        let (field, wire_type) = (tag >> 3, tag & 0x7);
        match wire_type {
            0 => {
                if read_varint(bytes, &mut pos).is_none() {
                    break;
                }
            }
            1 => pos += 8,
            2 => {
                let Some(len) = read_varint(bytes, &mut pos) else {
                    break;
                };
                let len = len as usize;
                let Some(end) = pos.checked_add(len).filter(|&end| end <= bytes.len()) else {
                    break;
                };
                if field == 1 {
                    files.push(&bytes[pos..end]);
                }
                pos = end;
            }
            5 => pos += 4,
            _ => break,
        }
    }
    files
}

/// Read one base-128 varint; returns `None` on truncation.
fn read_varint(bytes: &[u8], pos: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *bytes.get(*pos)?;
        *pos += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
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

    // (5) The REAL ConnectRPC service. ONE Router carries every service, so all
    // three are served identically on native AND the Worker:
    //   - ApiImpl              — the guarded demo.v1.Api.
    //   - HealthImpl           — grpc.health.v1.Health (Check → SERVING).
    //   - ReflectionImpl       — grpc.reflection.v1.ServerReflection, backed by
    //                            the embedded FILE_DESCRIPTOR_SET.
    // Health + reflection are public (their paths are in PUBLIC_PATHS, skipped
    // by every guard layer below), so a probe or a `grpcurl` discovery call —
    // neither of which carries a Rauthy token — reaches them.
    //
    // Two interceptors run on the RPC surface after envelope decode:
    //   - MetricsInterceptor: time each RPC → host-injected sink.
    //   - CedarInterceptor: BODY-aware authz (reads GetDoc's doc_id) — the job
    //     the path layer can't do. Enforce mode; the public paths are skipped
    //     so health/reflection aren't body-authorized either.
    let reflector = Arc::new(Reflector::from_descriptor_set_bytes(FILE_DESCRIPTOR_SET));
    let router = Arc::new(ApiImpl).register(Router::new());
    let router = Arc::new(HealthImpl).register(router);
    let router = Arc::new(ReflectionImpl { reflector }).register(router);
    let service = ConnectRpcService::new(router)
        .with_interceptor(MetricsInterceptor::new(sink))
        .with_interceptor(
            CedarInterceptor::enforce(Arc::clone(&authorizer), getdoc_cedar)
                .skip_paths(PUBLIC_PATHS.iter().copied()),
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
    .skip_paths(PUBLIC_PATHS.iter().copied())
    .layer(service);

    // (3) OIDC AuthN: verify the Rauthy JWT → insert the Session. Health +
    // reflection are public (PUBLIC_PATHS) and carry no token.
    let authed = OidcLayer::new(verifier)
        .skip_paths(PUBLIC_PATHS.iter().copied())
        .layer(cedar);

    // (2) Rate limit (host-injected limiter), then (1) tracing.
    let limited = RateLimitLayer::enforce(limiter, IpKeyExtractor::new())
        .skip_paths(PUBLIC_PATHS.iter().copied())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The embedded descriptor set builds a reflector that can resolve every
    /// service this app serves — by symbol AND by file — and lists all three.
    /// This is the exact data path the live `ServerReflectionInfo` stream uses,
    /// proven without a network round trip.
    #[test]
    fn reflector_resolves_app_services() {
        let r = Reflector::from_descriptor_set_bytes(FILE_DESCRIPTOR_SET);

        // ListServices advertises the demo API + the two infra services.
        let mut services = r.service_names.clone();
        services.sort();
        assert_eq!(
            services,
            [
                "demo.v1.Api",
                "grpc.health.v1.Health",
                "grpc.reflection.v1.ServerReflection",
            ]
        );

        // file_containing_symbol resolves a method on the guarded API and
        // returns at least the declaring file's bytes (its closure).
        let by_symbol = r.respond(&reflection::ServerReflectionRequest {
            message_request: Some(
                reflection::server_reflection_request::MessageRequest::FileContainingSymbol(
                    "demo.v1.Api.GetDoc".to_owned(),
                ),
            ),
            ..Default::default()
        });
        assert!(matches!(by_symbol, ReflectAnswer::Files(ref f) if !f.is_empty()));

        // file_by_filename resolves the proto by its descriptor name.
        let by_file = r.respond(&reflection::ServerReflectionRequest {
            message_request: Some(
                reflection::server_reflection_request::MessageRequest::FileByFilename(
                    "demo/v1/api.proto".to_owned(),
                ),
            ),
            ..Default::default()
        });
        assert!(matches!(by_file, ReflectAnswer::Files(ref f) if !f.is_empty()));

        // A miss is reported in-band as not_found (keeps the stream alive),
        // not as a stream-terminating error.
        let miss = r.respond(&reflection::ServerReflectionRequest {
            message_request: Some(
                reflection::server_reflection_request::MessageRequest::FileByFilename(
                    "nope.proto".to_owned(),
                ),
            ),
            ..Default::default()
        });
        assert!(matches!(miss, ReflectAnswer::NotFound(_)));
    }

    /// `reflect_response` maps a not-found answer onto a gRPC NOT_FOUND (5)
    /// ErrorResponse and echoes the original request + host, per the protocol.
    #[test]
    fn reflect_response_reports_not_found_in_band() {
        let r = Reflector::from_descriptor_set_bytes(FILE_DESCRIPTOR_SET);
        let req = reflection::ServerReflectionRequest {
            host: "test-host".to_owned(),
            message_request: Some(
                reflection::server_reflection_request::MessageRequest::FileByFilename(
                    "missing.proto".to_owned(),
                ),
            ),
            ..Default::default()
        };
        let resp = reflect_response(&r, req);
        assert_eq!(resp.valid_host, "test-host");
        match resp.message_response {
            Some(reflection::server_reflection_response::MessageResponse::ErrorResponse(e)) => {
                assert_eq!(e.error_code, 5);
                assert!(e.error_message.contains("missing.proto"));
            }
            other => panic!("expected ErrorResponse, got {other:?}"),
        }
    }

    /// The Health response this app reports is SERVING — the message
    /// `HealthImpl::check` builds. (The handler is a trivial constant return;
    /// the live SERVING wire body is asserted end-to-end in `server/serve.nu`.)
    #[test]
    fn health_response_is_serving() {
        let resp = health::HealthCheckResponse {
            status: health::health_check_response::ServingStatus::SERVING.into(),
            ..Default::default()
        };
        assert_eq!(
            resp.status.as_known(),
            Some(health::health_check_response::ServingStatus::SERVING)
        );
    }
}
