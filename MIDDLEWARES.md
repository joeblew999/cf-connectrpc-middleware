# Connect-RPC Middleware Catalog — Rust + Cloudflare Workers

The catalog of **every** Connect-RPC middleware on GitHub that we
could find, with per-entry verdict on whether it compiles for
`wasm32-unknown-unknown` (= runs on Cloudflare Workers). Covers
`tower::Layer` wrappers, `connectrpc::Interceptor` impls,
`ConnectRpcService` config knobs, handler-side helpers, and
proc-macro decorators — six surfaces total ([§1](#1-six-middleware-surfaces-in-connectrpc)).

**Audience**: anyone building on `connectrpc` in Rust, especially on
Cloudflare Workers. If you searched for "connect-rpc rust middleware",
"connectrpc tower layer", "cloudflare workers connect rpc", "cedar +
tower", or "connectrpc interceptor" — this is the right page.

**Source**: this repo, [`cf-connectrpc-middleware`](https://github.com/joeblew999/cf-connectrpc-middleware),
ships the first three library crates filling the empty intersection
(`connectrpc-tower-kit`, `connectrpc-cedar`, `connectrpc-cf-tracing`)
plus more planned — see the [crate table in the README](./README.md#crates-in-this-workspace).
The catalog isn't a side note here; it's the design map the whole
repo is built against.

Last full sweep: **2026-05-28** (against `connectrpc 0.4`, authenticated
`gh search code` queries). Cadence: monthly re-runs ([§5](#5-how-to-find-more)).

---

## 1. Six middleware surfaces in connectrpc

Picking the right surface is the most important design decision —
search queries to find prior art differ, and the wrong choice forces
you to fight the framework. There are **six** surfaces, not five (the
sixth was discovered while reading `protovalidate-buffa`):

| # | Surface | Where it sits | Fingerprint | When to pick |
| - | --- | --- | --- | --- |
| 1 | `tower::Layer<S>` *transparent* | wraps `ConnectRpcService`; pass-through on `S::Response`/`S::Error` | `impl<S> Layer<S>` + `type Response = S::Response;` | Enrich request (insert into `extensions`, attach RequestId), never short-circuit |
| 2 | `tower::Layer<S>` *short-circuit* | wraps `ConnectRpcService`; pins `Response = Response<ConnectRpcBody>`, `Error = Infallible` | `impl<S> Layer<S>` + `ConnectRpcBody` + `Error = Infallible` | Reject requests **before** envelope decode (path-based authz, CORS, rate limit) |
| 3 | `connectrpc::Interceptor` *(main-branch only)* | registered via `ConnectRpcService::with_interceptor` | `impl Interceptor` + `intercept_unary` / `intercept_streaming` | Body-aware authz, per-RPC logging with `Spec` metadata, anything needing decoded `UnaryRequest`. **⚠ Not in published `connectrpc 0.4.2`** — only on the `anthropics/connect-rust` `main` branch. Use surfaces #1/#2 until it ships. |
| 4 | `ConnectRpcService` *config* | built into the service itself | `Limits`, `DeadlinePolicy`, `with_max_body_size`, etc. | Resource limits, request-asserted timeouts — already shipped, don't write |
| 5 | Handler-side helper | called inside the handler body | `fn(&ctx, …) -> Result<_, ConnectError>` | Fine-grained authz on body fields; legacy pattern most public repos use |
| 6 | **Proc-macro handler decorator** | wraps each `impl Service` method at compile time | `#[connect_impl]` attribute injecting code before user body | Per-handler unconditional checks (validation, audit). Zero runtime cost — no `Arc`, no `dyn`. |

Surface 6 is the `protovalidate-buffa` innovation: `#[connect_impl]`
recognizes `OwnedView<T>` in handler signatures and injects `decode +
validate()` before user code runs. Type-driven, compile-time-checked,
no ordering concerns vs other middleware.

> `axum::middleware::from_fn` is **NOT** a Connect-RPC middleware surface
> per se — it's an axum convenience for writing tower middlewares with
> less ceremony. Only available when you're hosting via axum, and **not
> on `wasm32-unknown-unknown` (Cloudflare Workers)** where axum's full
> stack doesn't compile. The official `anthropics/connect-rust
> examples/middleware` uses it; CF Workers consumers must hand-roll
> tower::Layer + tower::Service pairs instead.

### Transparent vs short-circuit Layer — concrete signatures

**Transparent** (RequestIdLayer, AuthLayer style — most middleware):

```rust
impl<S, B> Service<http::Request<B>> for MyService<S>
where
    S: Service<http::Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;
    // call() inserts/reads extensions or headers, then self.inner.call(req)
}
```

**Short-circuit** (our CedarLayer in Enforce mode):

```rust
impl<S, E, B> Service<http::Request<B>> for MyService<S, E>
where
    S: Service<http::Request<B>, Response = Response<ConnectRpcBody>, Error = Infallible>,
{
    type Response = Response<ConnectRpcBody>;
    type Error = Infallible;
    // call() may construct ConnectError::*.to_json() response and never invoke S
}
```

The pin on `Error = Infallible` is because `ConnectRpcService::Error =
Infallible` — Connect encodes failures into the response body, not the
error channel. Short-circuit layers follow suit.

---

## 2. CF Workers constraint (`wasm32-unknown-unknown`)

**This stack is the target.** We're all-in on Connect-RPC + Rust + CF
Workers; the multitenant-worker example proves it works end-to-end. So
every entry in this catalog is judged on whether it compiles and runs
in that environment.

### Surface-level compatibility

| Surface | Works on Workers | Notes |
| --- | --- | --- |
| Transparent `tower::Layer` | yes (verified) | `RequestIdLayer` / `AuthLayer` ship in production |
| Short-circuit `tower::Layer` | yes (verified) | our `CedarLayer` ships in production |
| `connectrpc::Interceptor` | yes (in principle) | no public impls exist to verify against; we'd be first |
| `ConnectRpcService` config (`Limits`, `DeadlinePolicy`) | yes (verified) | built into the crate, no I/O involved |
| Handler-side helpers | yes (verified) | `ConnectError::permission_denied`, `require_session` |
| Proc-macro handler decorator (`#[connect_impl]`) | yes in principle | `protovalidate-buffa` macro emits pure Rust — should compile to wasm32; needs verification |
| `axum::middleware::from_fn` | **no** | axum full stack doesn't compile on wasm32 |
| `tower_http::*` | **probably no** | not verified; assume no — most layers use tokio I/O or tower-async features that don't reach wasm32 |
| `tonic::*` interceptors | **no** | tonic is gRPC, not Connect; doesn't compile on wasm32 either |

**Rule of thumb for CF Workers**: hand-rolled `tower::Layer` + `Service`
pair, `connectrpc::Interceptor`, or `connectrpc::ConnectRpcService`
built-in config. Nothing else.

### Dependencies verified on wasm32 (`example-multitenant-worker/Cargo.toml`)

These build for `wasm32-unknown-unknown` against the multitenant worker
— evidence drawn from the actual shipping Cargo.toml. Any middleware
that depends only on this set is automatically CF-compatible.

```
worker = "0.8"                           # CF runtime bindings
connectrpc = "0.4" (default-features = false)
tower = "0.5" (with util)                # traits only — no I/O backend
http = "1", http-body = "1", http-body-util = "0.1"
bytes = "1.5"
futures = "0.3"
cedar-policy = "4"                       # proven by this repo
tracing = "0.1" (default-features = false)
tracing-subscriber = "0.3" (fmt + env-filter only)
serde = "1", serde_json = "1"
buffa = "0.5" (json), buffa-types = "0.5"
uuid = "1" (with js feature)
argon2 = "0.5", password-hash = "0.5"
libmacaroon = "0.2" (wasm feature)
rand_core = "0.6" (with getrandom)
getrandom = "0.2" (with js feature)
```

Notable **absences** (the things that mean "no" for wasm32):
`tokio`, `tokio-util`, `tower-http`, `axum`, `hyper`, `tonic`, `tonic-build`,
`reqwest`, `rustls`, `mio`, `socket2`, anything filesystem-y.

---

## 3. By function — what's missing on Connect-RPC + CF Workers

| Function | Surface fit | Status | Notes |
| --- | --- | --- | --- |
| Request ID | transparent Layer | **shipped** in `connyay/example-connectrpc-worker` (`middleware.rs`) | Canonical pattern, copy-paste-ready |
| Session auth (decode bearer → SessionContext) | transparent Layer | **shipped** in `connyay/example-multitenant-worker` (`middleware/auth.rs`) | Macaroon-based; pattern transfers to JWT/opaque tokens |
| Path-based authz (Cedar) | short-circuit Layer | **shipped** (this repo, `connectrpc-cedar`) | First of its kind on Connect-RPC |
| Body-aware authz | Interceptor | **missing** | Greenfield; no public Interceptor impls anywhere |
| CF-context tracing (`request.cf` + `cf-ray` → span fields) | transparent Layer | **shipped** (this repo, `connectrpc-cf-tracing`) | First published Connect-RPC tracing crate for CF Workers |
| Generic tracing / structured logging (non-CF) | Interceptor or Layer | **missing** for Interceptor surface | `connectrpc-cf-tracing` covers the CF case; a non-CF variant is still open |
| Per-RPC metrics (counter + latency histogram) | transparent Layer (Interceptor when Spec lands) | **shipped** (this repo, `connectrpc-cf-metrics`) | Uses URL path for procedure label; will gain `Spec::procedure` when Interceptor lands in a connectrpc release. CF binding: Analytics Engine. |
| Rate limiting | short-circuit Layer | **shipped** (this repo, `connectrpc-cf-rate-limit`) | Wraps CF's Rate Limiting binding; `Mode::Observe`/`Enforce` via the kit's `Rollout` trait. |
| CORS | short-circuit Layer | **missing**; CF Workers usually handle CORS in `worker::Cors` directly | Look at `worker::Cors` first |
| Deadline / timeout | `ConnectRpcService` config | **shipped** in `connectrpc::DeadlinePolicy` | Don't write your own |
| Body size limit | `ConnectRpcService` config | **shipped** in `connectrpc::Limits` | Don't write your own |
| Request validation (protovalidate) | Interceptor | unclear if there's a CF-Workers-friendly one — see `mathematic-inc/protovalidate-buffa` |
| i18n error keys | handler-side | **shipped pattern** in `Y4shin/platform` | Use Lingui-style keys as `ConnectError` messages |
| Idempotency replay | Layer + KV cache | **missing** | Could be built on `Spec::idempotency` |

"Missing" = no public Rust implementation found on GitHub as of
2026-05-28 with authenticated search. "Shipped pattern" = code exists
in a non-library repo that we can copy-paste, not a published crate.

---

## 4. The catalog

Read each entry as "what to learn from it" + "what shape it actually
is". Entries are grouped by relevance to a fresh CF-Workers consumer.

### Tier 1 — copy-paste-ready (Connect-RPC + wasm32 verified)

#### `connyay/example-connectrpc-worker` — `middleware.rs`
- **Shape**: transparent `tower::Layer` + generic `tower::Service<http::Request<B>>`
- **What**: `RequestIdLayer` — assigns `x-request-id` from header or monotonic counter, inserts `RequestId(HeaderValue)` into `req.extensions()`.
- **Why care**: canonical signature for any pass-through Layer on CF Workers. Tests included.
- **Generic over `B`**: yes — works with any body type.
- **CF Workers**: **yes (verified in production)**.

#### `connyay/example-multitenant-worker` — `src/middleware/auth.rs`
- **Shape**: transparent `tower::Layer` + generic `tower::Service<http::Request<B>>`
- **What**: `AuthLayer` — verifies `Authorization: Bearer <macaroon>`, inserts `SessionContext` into extensions on success. Failures are **silent** — handlers run `require_session(ctx)?` themselves.
- **Why care**: "soft middleware + handler-side enforcement" pattern. Read it alongside `RequestIdLayer` — same shape, different purpose.
- **Companion helper**: `require_session(&RequestContext) -> Result<SessionContext, ConnectError>`.
- **CF Workers**: **yes (verified in production)**.

#### `connyay/EdgeReplica` — `worker/src/middleware/{request_id,session_auth}.rs`
- **Shape**: two transparent `tower::Layer`s + shared `extract_bearer` helper in `mod.rs`.
- **What**: same pattern as `example-multitenant-worker`, applied to a second CF Worker. RequestId + SessionAuth. Modules organized as `middleware/request_id.rs` + `middleware/session_auth.rs` + `middleware/mod.rs` (shared utilities).
- **Why care**: confirms connyay has a **convention** — every CF Worker they ship has the same middleware module layout. This is the de-facto template.
- **CF Workers**: **yes (verified in production)**.

#### `connectrpc-tower-kit` (this repo) — `crates/connectrpc-tower-kit/`
- **Shape**: foundation library (not a middleware itself).
- **What**: shared primitives every middleware in the family depends on — `Rollout` trait (generalizes `Mode::Shadow`/`Enforce` so rate-limit / validation / tracing-sampling crates can ship the same safe-rollout knob with their own enum), `deny_response(ConnectError) -> Response<ConnectRpcBody>` (Connect-protocol error builder), `ShortCircuitFuture<F>` (`pin_project_lite` Future enum for any short-circuiting Layer), canonical extension-type name docs.
- **Why care**: the kit any new Connect-RPC middleware should be built on. Reading its 4 source files is the fastest way to understand what conventions this catalog's §6 patterns translate to in code.
- **CF Workers**: **yes (generic — no CF data, no CF binding; works on any `connectrpc` host)**.

#### `connectrpc-cedar` (this repo) — `crates/connectrpc-cedar/src/layer.rs`
- **Shape**: short-circuit `tower::Layer` + `tower::Service<http::Request<B>, Response = Response<ConnectRpcBody>, Error = Infallible>`. Depends on the kit's `Rollout` + denial-response builder + `ShortCircuitFuture`.
- **What**: Cedar policy authz with `Mode::Shadow` (log-only, never reject) and `Mode::Enforce` (reject on `Decision::Deny`). `skip_paths` for health checks.
- **Why care**: first short-circuiting Layer in the ecosystem. Shadow mode is unique — no other middleware in this catalog has it. Pairs with the `connectrpc-cf-tracing` entry below: every Cedar decision shows up inside the tracing span's `cf.*` fields automatically when both are wired.
- **Future**: complement with `connectrpc-cedar-interceptor` (body-aware, two-trait split per pattern §6.4) and `connectrpc-cedar-macros` (`#[require_authorized(...)]` proc-macro). See [README crate table](./README.md#crates-in-this-workspace).
- **CF Workers**: **yes (verified — shipped to `workers-multitenant.gedw99.workers.dev`)**.

#### `connectrpc-cf-tracing` (this repo) — `crates/connectrpc-cf-tracing/`
- **Shape**: transparent `tower::Layer` + generic `tower::Service<http::Request<B>>`. Pure pass-through; opens a `tracing::Span` around `inner.call(req)`, never short-circuits.
- **What**: per-RPC span carrying CF-Workers runtime metadata — `cf.procedure` (resolved RPC path), `cf.colo`, `cf.country`, `cf.asn`, `cf.tls_cipher`, `cf.http_protocol`, `cf.ray` — pulled from `worker::Cf` (which workers-rs auto-inserts at [`http/request.rs:41`](https://github.com/cloudflare/workers-rs)) and the `cf-ray` header. Every event logged downstream picks up the active span's fields.
- **Why care**: **first published Connect-RPC tracing middleware crate for CF Workers**. Closes a wishlist gap from §3. Deliberately does NOT adopt the kit's `Rollout` trait (tracing has no off-mode worth toggling) — proves the kit's traits are opt-in.
- **CF Workers**: **yes (cf-context — reads `request.cf` + `cf-ray`; declares no CF binding; falls back gracefully on non-CF hosts)**.

#### `connectrpc-cf-rate-limit` (this repo) — `crates/connectrpc-cf-rate-limit/`
- **Shape**: short-circuit `tower::Layer` + `tower::Service<http::Request<B>, Response = Response<ConnectRpcBody>, Error = Infallible>`. Async — boxes `BoxFuture` per call since the rate-limit check round-trips to the CF binding.
- **What**: calls the CF Rate Limiting binding (`env.RL.limit(key)`) before each request. On `success = false` and `Mode::Enforce`, short-circuits with `ConnectError::resource_exhausted`. `Mode::Observe` logs would-have-throttled events but always passes through. `RateLimiter` trait + `RateLimitKeyExtractor` (default: per-IP from `cf-connecting-ip`) keep the crate `worker`-dep-free; consumer wires the binding in ~6 LOC.
- **Why care**: first short-circuit + async tower::Layer in the family. Adopts kit `Rollout` trait — second consumer after Cedar, validates the trait's abstraction. Fail-open on `RateLimitOutcome::Error` (binding errors don't take down the worker).
- **CF Workers**: **yes (cf-binding: Rate Limiting — consumer provisions `[[unsafe.bindings]]` with `type = "ratelimit"`)**.

#### `connectrpc-cf-metrics` (this repo) — `crates/connectrpc-cf-metrics/`
- **Shape**: transparent `tower::Layer` + generic `tower::Service<http::Request<B>>`. Future wrapper times the inner call and emits on completion. (Will become an `Interceptor` once that surface ships in a connectrpc release — see §1 note on surface #3.)
- **What**: emits two metrics per RPC via consumer-implemented `MetricSink` — `rpc_requests_total` (counter) and `rpc_latency_ms` (histogram), labeled by `procedure` (URL path) and `status_class` (`2xx` / `4xx` / `5xx`). `NoopSink` shipped for tests; CF Analytics Engine wiring in ~10 LOC. Uses `web_time::Instant` for cross-platform monotonic timing (`std::time::Instant` doesn't compile to wasm32).
- **Why care**: closes the observability story with `connectrpc-cf-tracing` — tracing for qualitative per-request log, metrics for quantitative dashboard. The `MetricSink` trait is async by design so non-CF hosts can wire Prometheus pushgateway, OTLP, statsd etc.
- **CF Workers**: **yes (cf-binding: Analytics Engine — consumer provisions `[[analytics_engine_datasets]]`)**.

### Tier 2 — built into `connectrpc` itself

#### `connectrpc::DeadlinePolicy` — `connectrpc/src/deadline.rs`
- **Shape**: `ConnectRpcService` config (`.with_deadline_policy(...)`).
- **What**: clamps client-asserted `Connect-Timeout-Ms` / `grpc-timeout` to a sane range, supplies a default, optionally extends to streaming response bodies.
- **Why care**: server-side timeout enforcement is already solved — don't write a TimeoutLayer.
- **CF Workers**: yes (no I/O involved; pure time math).

#### `connectrpc::Limits` — `connectrpc/src/service.rs`
- **Shape**: `ConnectRpcService` config.
- **What**: max request size, message size, etc.
- **Why care**: resource bounds, already solved.
- **CF Workers**: yes.

#### `connectrpc::Interceptor` trait — `connectrpc/src/interceptor.rs`
- **Shape**: the RPC-level middleware contract (#3 above).
- **What**: trait with `intercept_unary` / `intercept_streaming`; registered via `ConnectRpcService::with_interceptor(...)` or closure form `unary_interceptor(|req, next| async move { ... })`.
- **Why care**: `gh search code 'impl connectrpc::Interceptor' --language=Rust` returns zero PUBLISHED-LIBRARY hits, but the upstream crate's own `tests/streaming/src/lib.rs` ships 3 production-quality reference impls (see next entry) — those are the canonical examples.
- **Opportunity**: first published `Interceptor` library implementation is still open.
- **CF Workers**: yes in principle (test impls use tokio so don't verify wasm32 directly; the trait itself is platform-agnostic).

#### `connectrpc::Interceptor` — **reference impls** in `connect-rust/tests/streaming/src/lib.rs`
- **Shape**: three real `impl Interceptor` blocks (not docstrings).
- **`SpecAndBodyInterceptor`** — unary; reads decoded payload (`req.payload.message::<EchoRequest>()?`), runs `next.run(req).await`, mutates the response message (`msg.sequence += 1000`), adds response headers. Exact shape we'd use for body-aware Cedar.
- **`StreamRecorder`** — streaming; records request paths, adds `x-stream-intercepted` response header.
- **`DenyAll`** — streaming; rejects with `ConnectError::permission_denied("not authorized")`. Pattern for short-circuit at the Interceptor level.
- **Why care**: these *are* the documentation. Read them before writing any `Interceptor`. `Spec::procedure == ctx.path()` invariant is asserted in the unary example — useful guarantee.
- **CF Workers**: the trait works on wasm32; these specific tests use tokio so don't compile there directly. Borrow the patterns, not the harness.

### Tier 3 — Rust + Connect-RPC, but not directly reusable

#### `anthropics/connect-rust` — `examples/middleware/`
- **Shape**: axum + `tower_http::TraceLayer` + axum `from_fn_with_state` auth + `tower_http::timeout::TimeoutLayer`
- **What**: the official reference middleware stack.
- **Why care**: ground truth for "how the crate's authors intend middleware to be wired". Stack composition pattern transfers; the axum + tower_http parts don't (CF Workers can't use them).
- **CF Workers**: **no** — axum + tower_http won't compile to `wasm32-unknown-unknown`.

#### `washanhanzi/connectrpc-axum` — full middleware library (server + client)
- **Shape**: **the most complete Connect-RPC middleware library on GitHub.** Three sub-crates:
  - `connectrpc-axum/src/layer/connect.rs` — `ConnectLayer`: protocol detection (Connect/gRPC/gRPC-Web), `ConnectContext` building, message limits, optional `Connect-Protocol-Version` enforcement. The kind of "essential server scaffolding Layer" we don't have at all on CF Workers.
  - `connectrpc-axum/src/layer/bridge.rs` — `BridgeLayer`: clever compression bridge. Connect streaming uses per-envelope compression (`Connect-Content-Encoding` headers) while unary uses standard HTTP `Content-Encoding`. BridgeLayer sets `Accept-Encoding: identity` for streaming requests so Tower's compression doesn't double-compress. Algorithm-agnostic.
  - `connectrpc-axum-client/` — **client-side interceptor system with TWO traits**: `Interceptor` (header-only, simple) and `MessageInterceptor` (typed message access via `prost::Message + serde::Serialize`). Both compose via zero-cost compile-time `Chain<I>` (not `Vec<Arc<dyn>>`). Builder API: `.with_interceptor(...).with_message_interceptor(...).build()`. 3 example bins demonstrate header / typed / streaming variants.
- **Why care**: parallel ecosystem doing what we want, but for axum. The split between header-only and typed interceptors is excellent design — directly applicable to a future `connectrpc-cedar-interceptor` (Cedar usually only needs headers + extensions, never the body). Their `Chain<I>` zero-cost composition is what to copy.
- **CF Workers**: **no** — axum + tower_http dependent. Read for design, port nothing as-is.

#### `Y4shin/platform` — `plugins/events/src/lib.rs` (+ `crates/junius-sdk/src/telemetry.rs`)
- **Shape**: handler-side `ConnectError::permission_denied` with **Lingui i18n error keys** as the message string. Plus a separate `Telemetry` SDK with vendor-neutral `MetricSink` trait (counter, histogram) — uses `tracing_subscriber::Layer` (NOT `tower::Layer`; same name, different trait — earlier ripgrep was a false positive).
- **What**: error *message* is `events.error.group_membership_required` etc. — frontend looks it up via Lingui catalog. Frontend gets localized errors without a translation layer in the RPC plumbing. The Telemetry SDK lets plugins emit metrics through a `dyn MetricSink` injected by the host.
- **Why care**: two orthogonal patterns. (1) i18n error keys pair well with Cedar denials — we could surface Cedar policy ids as i18n keys. (2) The `MetricSink` indirection pattern is good prior art for any future `connectrpc-metrics-interceptor` that needs to stay vendor-neutral (OTel-free at the SDK boundary).
- **CF Workers**: yes — patterns are platform-agnostic. The `tracing` deps are wasm-friendly.

#### `connyay/EdgeReplica`
- **Shape**: handler-side `ConnectError::permission_denied`.
- **What**: same author as `example-multitenant-worker`; same handler-side pattern across their other workers.
- **Why care**: confirms that *even inside connyay's own ecosystem*, middleware-shaped authz didn't exist before this repo.
- **CF Workers**: **yes (verified in production)** — it's a CF Worker.

#### `NakaSato/gridtokenx-chain-bridge`
- **Shape**: handler-side `ConnectError::permission_denied`.
- **Why care**: third datapoint confirming handler-side is the community default.
- **CF Workers**: unknown (not a CF target; non-wasm).

#### `defenseunicorns/peat-node`
- **Shape**: handler-side `ConnectError`; uses `hyper::service::service_fn` to bridge tower → hyper.
- **Why care**: non-wasm Connect-RPC server; hyper-direct bridge pattern.
- **CF Workers**: **no** — hyper-direct.

#### `nu11ptr/connect2axum`
- **Shape**: protocol bridge (connect ↔ axum), not middleware per se.
- **Why care**: bookmark for Interceptor work; shows how decoded payloads flow.
- **CF Workers**: **no** — axum-dependent.

#### `mathematic-inc/protovalidate-buffa` — `crates/protovalidate-buffa/src/connect.rs` + `protovalidate-buffa-macros`
- **Shape**: **surface #6** — proc-macro handler decorator (`#[connect_impl]`). NOT tower, NOT Interceptor.
- **What**: `#[connect_impl]` on a service `impl` block scans handler signatures for `OwnedView<T>` arguments. For each match, the macro injects `decode + validate()` BEFORE the user's handler body. Validation failure short-circuits with `ConnectError::invalid_argument(violation.to_string())`. The `connect.rs` glue is tiny — `ValidationError::into_connect_error()` — because the macro does the work.
- **Why care**: a genuinely new middleware surface. Zero runtime cost (no `Arc`, no `dyn`, no `Vec<Interceptor>`), type-driven (only injects if signature matches), and validation runs unconditionally per handler. We didn't have this surface in our mental model. Read `crates/protovalidate-buffa/tests/connect_impl.rs` for the macro behavior reference.
- **CF Workers**: likely yes — macro emits pure Rust against `connectrpc::ConnectError`. Needs explicit verification but no obvious blockers.
- **Lesson for our crate**: certain checks (audit log, validation, request-id stamping) might be cleaner as proc-macros than as Layers/Interceptors. Worth considering for body-aware Cedar IF the policy id can be statically derived from the handler signature.

### Tier 3.5 — sibling stack (TypeScript on CF Workers)

#### `depot/connectrpc-workers` — `@depot/connectrpc-workers` npm package
- **Shape**: TS Connect-RPC adapter for CF Workers (`connectWorkersAdapter`).
- **What**: turns a `ConnectRouter` into an `ExportedHandler.fetch`. 15★, MIT, actively maintained.
- **Why care**: parallel-stack reference. The TS Connect-RPC world uses "interceptors" (their term for middleware) on `ConnectRouter`. Worth reading their interceptor docs before designing our Rust `Interceptor` impls — TS conventions translate well.
- **CF Workers**: yes (it's *for* CF Workers, just TS not Rust).

### Tier 4 — adjacent (different protocol or different language)

#### `Govcraft/acton-service` — **6 middlewares in one framework**
- **Shape**: mixed — three `axum::middleware::from_fn`-style and three `tower::Layer`-style.
  - `src/middleware/cedar.rs` — Cedar authz (axum `from_fn`). Builder with `with_path_normalizer` + optional cache.
  - `src/audit/middleware.rs` — audit logging (axum `from_fn`). Per-route annotation via `AuditRoute` extension.
  - `src/lockout/middleware.rs` — login lockout (axum `from_fn`). Extracts identity from JSON body, returns HTTP 423 + `Retry-After` on locked accounts, records 401-success based on response status. Stateful — wraps `LoginLockout` over Redis.
  - `src/session/csrf.rs` — CSRF protection (`tower::Layer`).
  - `src/grpc/middleware.rs` — `LoggingLayer` for tonic gRPC (`tower::Layer`). Pins `Response = Response<tonic::body::Body>, Error = Status`.
  - `src/grpc/interceptors.rs` — tonic interceptors with `RequestIdExtension`.
- **Why care**: the only production-grade Cedar+Rust+HTTP-middleware combo in the wild. Demonstrates that **non-trivial systems use 5-7 middlewares**, not one. Builder + path-normalizer + Redis cache are good API ideas for `connectrpc-cedar` v0.2.
- **CF Workers**: **no** — axum, tonic, and Redis all break wasm32. Read for design, port nothing as-is.

#### `cedar-policy/cedar-examples/tinytodo` — Rust + axum + Cedar
- **Shape**: handler-side Cedar checks; not middleware.
- **Why care**: shows Cedar wired into a real (if toy) Rust app.
- **CF Workers**: **no** — axum-dependent.

#### `cedar-policy/authorization-for-expressjs` — JS
- **Shape**: Express middleware; not Rust.
- **Why care**: influenced our `skip_paths` API surface.
- **CF Workers**: **no** — different language; CF Workers Node-compat is best-effort.

#### `permitio/cedar-agent` / `JanssenProject/jans-cedarling`
- **Shape**: sidecar service / embeddable library.
- **Why care**: alternative architectures (RPC-out vs in-process eval). We picked in-process.
- **CF Workers**: agent **no** (sidecar pattern); cedarling **unknown** (embedded, might wasm).

### Tier 5 — known Connect-RPC consumers (no published middleware yet)

These all `use connectrpc::` but didn't have anything that surfaced as
a Layer or Interceptor when grepped. Worth re-checking periodically as
their codebases evolve.

`AprilNEA/BYOKEY`, `EmilLindfors/a2a-rs` (Agent2Agent in Rust, 85★),
`R3dRum92/wrenn-releases`, `dangoe/loci`, `exowarexyz/monorepo`,
`katara-ai-inc/katara-cli`, `kuku-mom/kuku`,
`mathematic-inc/protovalidate-buffa` (protovalidate integration!),
`microsoft/openvmm`, `ohd-foundation/ohd`, `open-lakehouse/open-lineage-connect`,
`revvy02/rodeo`, `routers-org/routers`, `sunbeamdotpt/sunbeam`,
`tjdragon/NeoUI`, `uppin/tddy-coder`, `wimpheling/backbone-template-v1`,
`wordbricks/onequery`, `NakaSato/gridtokenx-iam-service`,
`NakaSato/gridtokenx-noti-service`.

---

## 5. How to find more

All restricted to `--language=Rust`. Run authenticated (`GH_TOKEN`) —
the anonymous 30/min budget gets exhausted in two batches.

### Surface-level fingerprint queries

```bash
# Short-circuit tower::Layer for Connect-RPC (our shape)
gh search code 'ConnectRpcBody tower::Layer' --language=Rust --limit=50

# Any Connect-RPC + tower combination
gh search code 'use connectrpc:: tower::Layer' --language=Rust --limit=50
gh search code 'ConnectRpcService tower' --language=Rust --limit=50

# Interceptor surface (the empty one — re-run to catch first published impl)
gh search code 'impl connectrpc::Interceptor' --language=Rust --limit=50
gh search code 'intercept_unary' --language=Rust --limit=50
gh search code 'with_interceptor connectrpc' --language=Rust --limit=50

# Handler-side authz (the legacy pattern — counts ecosystem maturity)
gh search code 'ConnectError::permission_denied' --language=Rust --limit=50
```

### Discovery queries

```bash
# Who uses the crate at all
gh search code 'use connectrpc::' --language=Rust --limit=100

# Cargo.toml entries — sometimes catches private middleware crates
gh search code 'connectrpc' --filename Cargo.toml --language=TOML --limit=50

# Cedar + Rust + tower (any combo)
gh search code 'cedar_policy::Authorizer tower' --language=Rust --limit=30
gh search code 'connectrpc cedar_policy' --language=Rust --limit=30
```

### Cadence

Re-run the surface queries **monthly**. The ecosystem grows by a couple
of repos a month; catching first-of-kind implementations early (e.g.
first published `Interceptor`) is high-value because it sets the
community convention.

---

## 6. What the catalog tells us about good shape

Cross-referencing the entries above, **six** patterns recur in every
middleware that ages well:

1. **Generic over body `B`**. `RequestIdLayer`, `AuthLayer` (both
   connyay variants — multitenant *and* EdgeReplica), and our
   `CedarLayer` all parameterize over `B` instead of pinning to
   `worker::Body`. Means the same crate works in tests (any body type)
   and in production (worker body), and outside CF Workers entirely.

2. **Insert into `req.extensions()`, never `req.headers()`**. The
   connectrpc dispatcher copies `http::Request::extensions` into
   `Context::extensions` during dispatch. Layers that stash data via
   headers force the handler to re-parse; layers that stash via
   extensions hand the handler a typed value. Acton-service's gRPC
   `RequestIdExtension` follows the same rule (extensions, not
   headers).

3. **Soft middleware + handler backstop**, at least during rollout.
   AuthLayer doesn't reject unauthorized requests — it just doesn't
   insert a session, and handlers call `require_session(ctx)?`. Our
   `Mode::Shadow` is the same idea applied to authz: log what would
   have been rejected, let the handler keep enforcing. Flip to hard
   enforcement once shadow logs are clean. The only safe rollout
   pattern for any middleware that can reject.

4. **Two-trait split for client-side interception** (washanhanzi
   discovery). Their `connectrpc-axum-client` has `Interceptor`
   (header-only, simple) and `MessageInterceptor` (typed message
   access via `prost::Message + serde::Serialize`). 90% of interceptors
   only need headers — forcing everyone through the typed surface
   inflates compile time and Send bounds. Same split should apply to
   our server-side: a future Cedar interceptor that reads headers is
   different from one that reads the body.

5. **Zero-cost compile-time composition** (washanhanzi). `Chain<I>`
   where `I` is a generic interceptor chain type, not `Vec<Arc<dyn>>`.
   `with_interceptor(...)` returns a `ClientBuilder<NewChain>` —
   each composition step changes the type. No dynamic dispatch, no
   `Arc`, no per-call allocation. The upstream `connectrpc` crate uses
   `Arc<[Arc<dyn Interceptor>]>` instead (dynamic, simpler). For
   CF Workers where every byte counts, the static approach wins.

6. **Module convention `middleware/{request_id,session_auth}.rs +
   mod.rs`** (connyay, shared between `example-multitenant-worker` and
   `EdgeReplica` with a `mod.rs` that exposes `extract_bearer`). Same
   layout, shared helpers in `mod.rs`. This is the *de facto* template
   for "a CF Worker middleware module" — if we ship more middleware
   crates, the consumer should be able to drop them into the same
   layout.

The wishlist from §3 (tracing-layer, metrics-interceptor, idempotency
replay, body-aware-authz, validation) all want the same shape:
generic over `B`, extensions-typed, opt-in enforcement, with the
header/body split where the surface allows it. Anyone building them
should read the six patterns above before starting.

### Headline numbers from this catalog (2026-05-28)

- **6** middleware surfaces in connectrpc (`tower::Layer` transparent,
  `tower::Layer` short-circuit, `Interceptor`, `ConnectRpcService`
  config, handler-helper, **proc-macro decorator**).
- **5** Rust + Connect-RPC + CF Workers consumer repos in the world
  (connyay × 3 workers, this repo's example worker, depot's TS sibling).
  That's the entire field.
- **3** real `connectrpc::Interceptor` impls exist publicly — all of
  them in the upstream crate's `tests/streaming/`, none in a library.
- **5** library crates this repo ships filling the empty intersection
  (`connectrpc-tower-kit`, `connectrpc-cedar`, `connectrpc-cf-tracing`,
  `connectrpc-cf-rate-limit`, `connectrpc-cf-metrics`). Pre-crates.io
  publish; consumed by path-dep from the example worker.
- **0** published crates on crates.io are Connect-RPC middleware
  (literal — no one's `cargo publish`-ed yet; we're next).
- **6** middlewares in the most complete framework (Govcraft) — but on
  the wrong stack (axum + tonic). Direction-of-travel reference.
- **1** novel surface discovered while researching: `protovalidate-buffa`'s
  `#[connect_impl]` proc-macro decorator.

---

## 7. Contributing

Found something not listed? Open an issue or PR. Bare minimum entry:

```markdown
#### `<owner>/<repo>` — `<file>`
- **Shape**: <surface from §1>
- **What**: <one sentence>
- **Why care**: <one sentence>
- **CF Workers**: yes | no | unknown (+ one-line evidence)
```

If you're adding a Tier 5 entry (consumer with no surfaced middleware),
no body needed — just the repo name in the list.

---

## 8. Machine-readable summary (source for the future JSONL)

This table is the canonical extractable view. Each row corresponds to
an entry above. When we later build the nushell + JSONL pipeline
(matching the `tauri-plugins-catalog` pattern), this is the source of
truth — keep table and prose in sync.

Columns: `id` | `url` | `lang` | `shape` | `function` | `cf_works` | `evidence`

`cf_works` is one of `yes-verified` / `yes-likely` / `unknown` /
`no-wasm` / `no-deps`. `shape` is one of `layer-transparent` /
`layer-short-circuit` / `interceptor` / `interceptor-client` /
`service-config` / `handler-helper` / `proc-macro-decorator` /
`axum-fn` / `axum-layer` / `adapter-ts` / `adapter-axum` /
`bridge` / `sidecar` / `library`.

| id | url | lang | shape | function | cf_works | evidence |
| --- | --- | --- | --- | --- | --- | --- |
| connyay-example-connectrpc-worker.RequestIdLayer | https://github.com/connyay/example-connectrpc-worker/blob/main/src/middleware.rs | rust | layer-transparent | request-id | yes-verified | ships in production |
| connyay-example-multitenant-worker.AuthLayer | https://github.com/connyay/example-multitenant-worker/blob/main/src/middleware/auth.rs | rust | layer-transparent | session-auth | yes-verified | ships in production |
| connyay-EdgeReplica.RequestIdLayer | https://github.com/connyay/EdgeReplica/blob/main/worker/src/middleware/request_id.rs | rust | layer-transparent | request-id | yes-verified | ships in production |
| connyay-EdgeReplica.SessionAuthLayer | https://github.com/connyay/EdgeReplica/blob/main/worker/src/middleware/session_auth.rs | rust | layer-transparent | session-auth | yes-verified | ships in production |
| cf-connectrpc-middleware.connectrpc-tower-kit | https://github.com/joeblew999/cf-connectrpc-middleware/tree/main/crates/connectrpc-tower-kit | rust | library | shared-primitives (Rollout, denial-response, ShortCircuitFuture) | yes-verified | workspace member; depended on by connectrpc-cedar |
| cf-connectrpc-middleware.connectrpc-cedar | https://github.com/joeblew999/cf-connectrpc-middleware/blob/main/crates/connectrpc-cedar/src/layer.rs | rust | layer-short-circuit | authz-cedar | yes-verified | deployed to workers-multitenant.gedw99.workers.dev |
| cf-connectrpc-middleware.connectrpc-cf-tracing | https://github.com/joeblew999/cf-connectrpc-middleware/blob/main/crates/connectrpc-cf-tracing/src/layer.rs | rust | layer-transparent | tracing-cf-context | yes-verified | wired as outermost layer in example-multitenant-worker |
| cf-connectrpc-middleware.connectrpc-cf-rate-limit | https://github.com/joeblew999/cf-connectrpc-middleware/blob/main/crates/connectrpc-cf-rate-limit/src/layer.rs | rust | layer-short-circuit | rate-limit (CF binding) | yes-verified | wraps env.RL.limit; Mode::Observe/Enforce via Rollout |
| cf-connectrpc-middleware.connectrpc-cf-metrics | https://github.com/joeblew999/cf-connectrpc-middleware/blob/main/crates/connectrpc-cf-metrics/src/layer.rs | rust | layer-transparent | metrics-cf-ae | yes-verified | counter + histogram per RPC to Analytics Engine via MetricSink trait |
| connectrpc.DeadlinePolicy | https://github.com/anthropics/connect-rust/blob/main/connectrpc/src/deadline.rs | rust | service-config | deadline | yes-likely | pure time math, no I/O |
| connectrpc.Limits | https://github.com/anthropics/connect-rust/blob/main/connectrpc/src/service.rs | rust | service-config | body-size-limit | yes-likely | pure counters |
| connectrpc.Interceptor | https://github.com/anthropics/connect-rust/blob/main/connectrpc/src/interceptor.rs | rust | interceptor | (any RPC-level) | yes-likely | trait only; zero public library impls |
| connectrpc-streaming-tests.SpecAndBodyInterceptor | https://github.com/anthropics/connect-rust/blob/main/tests/streaming/src/lib.rs | rust | interceptor | reference-impl-unary | unknown | tokio-test harness, but trait itself is platform-agnostic |
| connectrpc-streaming-tests.StreamRecorder | https://github.com/anthropics/connect-rust/blob/main/tests/streaming/src/lib.rs | rust | interceptor | reference-impl-streaming | unknown | same as above |
| connectrpc-streaming-tests.DenyAll | https://github.com/anthropics/connect-rust/blob/main/tests/streaming/src/lib.rs | rust | interceptor | short-circuit-pattern | unknown | same as above |
| anthropics-connect-rust.examples-middleware | https://github.com/anthropics/connect-rust/tree/main/examples/middleware | rust | axum-fn + axum-layer | auth + trace + timeout | no-deps | axum + tower_http not wasm32 |
| washanhanzi-connectrpc-axum.ConnectLayer | https://github.com/washanhanzi/connectrpc-axum/blob/main/connectrpc-axum/src/layer/connect.rs | rust | adapter-axum | protocol-detection + context | no-deps | axum-dependent |
| washanhanzi-connectrpc-axum.BridgeLayer | https://github.com/washanhanzi/connectrpc-axum/blob/main/connectrpc-axum/src/layer/bridge.rs | rust | adapter-axum | compression-bridge | no-deps | axum-dependent |
| washanhanzi-connectrpc-axum-client.Interceptor | https://github.com/washanhanzi/connectrpc-axum/blob/main/connectrpc-axum-client/src/config/interceptor.rs | rust | interceptor-client | client-header-injection | no-deps | hyper transport, axum-dependent |
| washanhanzi-connectrpc-axum-client.MessageInterceptor | https://github.com/washanhanzi/connectrpc-axum/blob/main/connectrpc-axum-client/src/config/interceptor.rs | rust | interceptor-client | client-typed-message-access | no-deps | hyper transport, axum-dependent |
| mathematic-inc-protovalidate-buffa.connect_impl | https://github.com/mathematic-inc/protovalidate-buffa/blob/main/crates/protovalidate-buffa/src/connect.rs | rust | proc-macro-decorator | request-validation | yes-likely | macro emits pure Rust; needs explicit wasm32 verify |
| Y4shin-platform.events-i18n-keys | https://github.com/Y4shin/platform/blob/main/plugins/events/src/lib.rs | rust | handler-helper | i18n-error-keys | yes-likely | pattern only, no platform deps |
| Y4shin-platform.junius-Telemetry | https://github.com/Y4shin/platform/blob/main/crates/junius-sdk/src/telemetry.rs | rust | library | metrics-sink-abstraction | yes-likely | uses tracing only |
| connyay-EdgeReplica.handler-authz | https://github.com/connyay/EdgeReplica | rust | handler-helper | session-authz | yes-verified | is a CF Worker |
| NakaSato-gridtokenx-chain-bridge.handler-authz | https://github.com/NakaSato/gridtokenx-chain-bridge | rust | handler-helper | session-authz | unknown | not a CF target |
| defenseunicorns-peat-node | https://github.com/defenseunicorns/peat-node | rust | handler-helper | session-authz | no-deps | hyper-direct; no Layer/Interceptor found |
| nu11ptr-connect2axum | https://github.com/nu11ptr/connect2axum | rust | bridge | protocol-adaptation | no-deps | axum-dependent |
| depot-connectrpc-workers | https://github.com/depot/connectrpc-workers | typescript | adapter-ts | runtime-adapter | yes-verified | npm package targeting CF Workers |
| Govcraft-acton-service.cedar | https://github.com/Govcraft/acton-service/blob/main/acton-service/src/middleware/cedar.rs | rust | axum-fn | authz-cedar | no-deps | axum + file-based policies |
| Govcraft-acton-service.audit | https://github.com/Govcraft/acton-service/blob/main/acton-service/src/audit/middleware.rs | rust | axum-fn | audit-logging | no-deps | axum-dependent |
| Govcraft-acton-service.lockout | https://github.com/Govcraft/acton-service/blob/main/acton-service/src/lockout/middleware.rs | rust | axum-fn | login-lockout | no-deps | axum + Redis-dependent |
| Govcraft-acton-service.csrf | https://github.com/Govcraft/acton-service/blob/main/acton-service/src/session/csrf.rs | rust | layer-transparent | csrf-protection | no-deps | axum-dependent |
| Govcraft-acton-service.grpc-LoggingLayer | https://github.com/Govcraft/acton-service/blob/main/acton-service/src/grpc/middleware.rs | rust | layer-transparent | grpc-logging | no-deps | tonic-dependent |
| Govcraft-acton-service.grpc-interceptors | https://github.com/Govcraft/acton-service/blob/main/acton-service/src/grpc/interceptors.rs | rust | interceptor | grpc-request-id | no-deps | tonic-dependent |
| cedar-examples-tinytodo | https://github.com/cedar-policy/cedar-examples/tree/main/tinytodo | rust | handler-helper | authz-cedar | no-deps | axum-dependent |
| cedar-authorization-for-expressjs | https://github.com/cedar-policy/authorization-for-expressjs | typescript | axum-layer | authz-cedar | no-deps | Express, not Connect-RPC |
| permitio-cedar-agent | https://github.com/permitio/cedar-agent | rust | sidecar | authz-cedar | no-deps | sidecar service |
| JanssenProject-jans-cedarling | https://github.com/JanssenProject/jans-cedarling | rust | library | authz-cedar | unknown | embedded evaluator; wasm32 not verified |

### Tier 5 — known consumers, no surfaced middleware

`id` | `url`. These were found via `gh search code 'use connectrpc::'`
but didn't surface anything middleware-shaped. Re-check periodically.

| id | url |
| --- | --- |
| AprilNEA-BYOKEY | https://github.com/AprilNEA/BYOKEY |
| EmilLindfors-a2a-rs | https://github.com/EmilLindfors/a2a-rs |
| R3dRum92-wrenn-releases | https://github.com/R3dRum92/wrenn-releases |
| dangoe-loci | https://github.com/dangoe/loci |
| exowarexyz-monorepo | https://github.com/exowarexyz/monorepo |
| katara-ai-inc-katara-cli | https://github.com/katara-ai-inc/katara-cli |
| kuku-mom-kuku | https://github.com/kuku-mom/kuku |
| mathematic-inc-protovalidate-buffa | https://github.com/mathematic-inc/protovalidate-buffa |
| microsoft-openvmm | https://github.com/microsoft/openvmm |
| ohd-foundation-ohd | https://github.com/ohd-foundation/ohd |
| open-lakehouse-open-lineage-connect | https://github.com/open-lakehouse/open-lineage-connect |
| revvy02-rodeo | https://github.com/revvy02/rodeo |
| routers-org-routers | https://github.com/routers-org/routers |
| sunbeamdotpt-sunbeam | https://github.com/sunbeamdotpt/sunbeam |
| tjdragon-NeoUI | https://github.com/tjdragon/NeoUI |
| uppin-tddy-coder | https://github.com/uppin/tddy-coder |
| wimpheling-backbone-template-v1 | https://github.com/wimpheling/backbone-template-v1 |
| wordbricks-onequery | https://github.com/wordbricks/onequery |
| NakaSato-gridtokenx-iam-service | https://github.com/NakaSato/gridtokenx-iam-service |
| NakaSato-gridtokenx-noti-service | https://github.com/NakaSato/gridtokenx-noti-service |

---

## 9. Backlog (this catalog)

- [x] **Tracing layer for CF Workers** — shipped as `connectrpc-cf-tracing`
      v0.1 (2026-05-30). Covers `request.cf` + `cf-ray` → span fields.
- [x] **Rate-limit layer wrapping CF binding** — shipped as
      `connectrpc-cf-rate-limit` v0.1 (2026-05-30). Mode::Observe/Enforce
      via the kit's Rollout trait. First short-circuit + async layer in
      the family.
- [x] **Metrics layer to Analytics Engine** — shipped as
      `connectrpc-cf-metrics` v0.1 (2026-05-30). Counter +
      latency histogram per RPC, async `MetricSink` trait so non-CF
      hosts can wire Prometheus/OTLP. Will become an `Interceptor`
      sibling once that surface ships in a connectrpc release.
- [ ] **Nushell + JSONL extraction**: write `scripts/middlewares-extract.nu`
      that parses §8 tables into `middlewares.jsonl`. Pattern: same as
      `tauri-plugins-catalog`. Once it lands, this MD becomes the
      authoring surface; consumers query the JSONL.
- [ ] **Build-verification CI**: for every Tier 1/2 entry tagged
      `yes-verified`, add a smoke `cargo check --target wasm32-unknown-unknown`
      to CI so we catch regressions if upstream changes.
- [ ] **Re-sweep monthly**: re-run the §5 queries; promote any newly-
      surfaced repos from Tier 5 to Tier 1–4 with proper analysis.
- [ ] **Publish to crates.io**: the family is path-dep-only today.
      Pushing v0.1 of all 3 crates lets others `cargo add` instead of
      forking.
- [ ] **Fill the remaining wishlist gaps** (§3):
      - `connectrpc-cedar-interceptor` (body-aware authz, two-trait split) — blocked on Interceptor surface shipping in a connectrpc release
      - `connectrpc-cedar-macros` (`#[require_authorized(...)]`)
      - `connectrpc-cf-idempotency` (KV binding)
      - `connectrpc-cf-trace-context` (W3C `traceparent` + `cf-ray` propagation)
      - `connectrpc-cf-access` (CF Access JWT verification)
      Each is a real package this catalog will gladly host. See the
      [README crate table](./README.md#crates-in-this-workspace) for status.
