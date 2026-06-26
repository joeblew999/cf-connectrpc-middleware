# rauthy-cedar

The reference example that composes the **whole `crates/*` middleware stack** as
one real ConnectRPC service, proven on **native** (hyper) and **Cloudflare
Workers** from a single shared `app/make()` — the one composition point. A
[Rauthy](https://github.com/sebadob/rauthy) JWT drives the AuthN → AuthZ seam,
and every other crate (tracing, rate-limit, metrics, body-aware Cedar) runs in
the same request path.

## The composed stack (outermost → innermost)

`app/src/lib.rs::make()` builds exactly this, identically on both hosts:

```
request
  │
  ▼ 1. TracingLayer             connectrpc-cf-tracing      transparent per-RPC span (CfFields)
  ▼ 2. RateLimitLayer::enforce  connectrpc-cf-rate-limit   host-injected limiter, skip /healthz
  ▼ 3. OidcLayer                connectrpc-oidc            verify Rauthy JWT → insert Session
  ▼ 4. cedar_enforce            connectrpc-guard           PATH authz: /demo.v1.Api/X → Action X
  ▼ 5. ConnectRpcService        connectrpc                 + MetricsInterceptor (host sink)
       ├─ ApiImpl                                          + CedarInterceptor  (BODY-aware authz)
       ├─ HealthImpl             grpc.health.v1.Health         public (skipped by 2–5)
       └─ ReflectionImpl         grpc.reflection.v1.ServerReflection  public (skipped by 2–5)
```

Layers 1–4 are `tower::Layer`s; the two interceptors (5) run on the Connect RPC
surface *after* envelope decode — which is why the `CedarInterceptor` can
authorize against the **decoded request body** (`GetDoc`'s `doc_id`), a decision
the path layer (4) can never make. Reaching a handler means tracing wrapped it,
the rate limit allowed it, the JWT verified, and **both** Cedar surfaces allowed
it.

### What each host injects into `make()`

`make()` is generic; the two platform-specific deps are passed in by the host —
never chosen inside the shared app:

| dep | native (`server/`) | Cloudflare (`worker/`) |
| --- | --- | --- |
| metrics `sink` (`MetricSink`) | `NoopSink` | `AeMetricSink` over a `worker::AnalyticsEngineDataset` (`AE` binding) |
| rate `limiter` (`RateLimiter`) | `AllowAll` | `CfRateLimiter` over `worker::RateLimiter` (`RL` binding); degrades to allow-all if `RL` is absent — see the [multi-worker note](#multi-worker-gateway--inter-worker-connectrpc-over-a-service-binding) |

`AllowAll` is the native counterpart to the CF Rate Limiting binding (added to
`connectrpc-cf-rate-limit`, mirroring `NoopSink`). The CF bindings are declared
in [`worker/wrangler.toml`](worker/wrangler.toml) (`[[analytics_engine_datasets]]`
+ `[[ratelimits]]`).

## The service: `demo.v1.Api`

One proto ([`app/proto/demo/v1/api.proto`](app/proto/demo/v1/api.proto)), four
methods that share one body (echo the verified `Session`). The point is the
**authorization** in front of each:

| RPC | Cedar action | resource | Decision | Enforced by |
| --- | --- | --- | --- | --- |
| `Read`   | `demo.v1.Api.Read`   | `Api` | any authenticated user → **200** | path (layer 4) |
| `Admin`  | `demo.v1.Api.Admin`  | `Api` | needs role `admin` in `context.roles` → 200 / **403** | path (layer 4) |
| `Super`  | `demo.v1.Api.Super`  | `Api` | needs role `superuser` (demo admin lacks it) → **403** | path (layer 4) |
| `GetDoc` | `demo.v1.Api.GetDoc` | `Doc` | `doc_id == "public"` → 200, anything else → **403** | body (interceptor, layer 5) |

The first three are **path-based**: `action_from_path` maps `/demo.v1.Api/Read`
→ `Action::"demo.v1.Api.Read"` on `Api::"main"`, so `cedar_enforce` (layer 4)
authorizes from the route alone. `GetDoc` is **body-aware**: the route can't tell
*which* doc, so the `CedarInterceptor` (layer 5) reads `doc_id` off the decoded
`GetDocRequest`, builds `Doc::"<doc_id>"`, and only `Doc::"public"` has a
satisfiable policy. Roles ride in `context.roles`, mapped from the Rauthy token
by `connectrpc-oidc`. The `permit` rules live in
[`app/policies/demo.cedar`](app/policies/demo.cedar); the `Doc` entity + `GetDoc`
action are in [`app/policies/demo.cedarschema`](app/policies/demo.cedarschema).

## gRPC Health + Reflection (native **and** Cloudflare)

The example serves the two standard gRPC infra services on the **same Router**
as `demo.v1.Api`, so they run identically on native and on the Worker:

- **`grpc.health.v1.Health`** — `Check` returns `SERVING`; `Watch` returns
  `unimplemented` (the spec-sanctioned signal, and keeps the wasm32 build free
  of a long-lived subscription future). Works with kubelet `grpc:` probes,
  `grpc_health_probe`, service meshes. The plain-HTTP `GET /healthz` 200 route
  is kept too for simple `httpGet:` liveness.
- **`grpc.reflection.v1.ServerReflection`** — full bidi `ServerReflectionInfo`,
  backed by a `FileDescriptorSet` `build.rs` emits (`emit_descriptor_set`) and
  `lib.rs` embeds. Answers `list_services`, `file_by_filename`, and
  `file_containing_symbol`, so `grpcurl`/`buf curl` can discover and call the
  API with no local `.proto` files.

Both are **public**: their RPC paths are in `make()`'s `PUBLIC_PATHS`, skipped
by the OIDC/Cedar/rate-limit layers (a probe or discovery call carries no
token). Crucially, the example does **not** depend on the `connectrpc-health` /
`connectrpc-reflection` crates — their manifests over-declare `connectrpc =
{ features = ["server"] }`, which pulls `mio` (`compile_error!` on wasm32). We
compile the standard protos with our own `connectrpc-build` pipeline and
reimplement the thin reflection query bridge over `buffa-descriptor` (the same
descriptor engine, pure-Rust + wasm-clean) instead.

## Policy proof

```sh
nu tests/run.nu                  # this example, directly
mise run cedar:validate          # validates this example's policies too
```

`tests/run.nu` runs the `cedar` CLI against the **same** files the app loads
(`app/policies/demo.cedar` + `app/policies/demo.cedarschema`) and asserts: Read
(no roles) → allow, Admin with `admin` → allow, Admin without → deny, Super with
`admin` → deny, GetDoc(`public`) → allow, GetDoc(`secret`) → deny. Expected:
6/6 pass. `mise run cedar:validate` (from the repo root) now also type-checks
this example's schema + policies, not just the standalone `*-policies/` trees.

## One app, two hosts (native + Cloudflare)

The whole point: **one codebase, two runtimes.** The entire middleware stack —
the `make()` composition, the policies, the `Api` service — lives in
**[`app/`](app)** and is shared verbatim. The two hosts are thin:

```
app/                shared: proto + policies + the Api impl + make() stack   (rlib, builds native AND wasm)
├── server/         NATIVE host  — hyper  + ureq JWKS  + NoopSink/AllowAll      → calls app::make()
├── worker/         CF WORKER (api) — event + worker::Fetch JWKS + AE/RL bindings → calls app::make()  (name = rauthy-cedar-api)
└── gateway/        CF WORKER (gateway) — serves gateway.v1.GatewayService; ProxyRead → backend over a [[services]] binding
```

This yields **three deployment shapes** of the same guarded service:

1. **native** — `server/`, the hyper host (`example:serve`).
2. **single worker** — `worker/` alone on `wrangler dev`, the backend serving
   `demo.v1.Api` on the edge (`example:worker:e2e`).
3. **multi-worker gateway** — `gateway/` fronts `worker/`, calling it over a
   Cloudflare `[[services]]` binding via connyay's `FetcherTransport`
   (`example:gateway:e2e`). See [§ Multi-worker gateway](#multi-worker-gateway--inter-worker-connectrpc-over-a-service-binding).

Each host does only what differs by platform: fetch the JWKS, run the serve
loop, and inject the metrics sink + rate limiter. The `app::make(verifier, sink,
limiter)` call that builds the entire middleware stack is **identical** in both
(`grep -n make` in `server/src/main.rs` and `worker/src/lib.rs`).

### Native (hyper) — `serve.nu` boots Rauthy, mints a token, asserts:

```sh
nu examples/rauthy-cedar/server/serve.nu      # needs a local Docker daemon
```
```
  ✓ healthz (no token)                  [200]            plain-HTTP liveness, skip path
  ✓ Health/Check (no token) → public    [200]            gRPC health service, public
  ✓ Read no-token → AuthN deny          [401]            OidcLayer rejects (no token)
  ✓ Read token → allow                  [200]            verified + Cedar path allow
  ✓ Admin admin-role → allow            [200]            token carries `admin`
  ✓ Super no-superuser → deny           [403]            Cedar permission_denied
  ✓ GetDoc(public) → body allow         [200]            CedarInterceptor reads doc_id
  ✓ GetDoc(secret) → body deny          [403]            CedarInterceptor reads doc_id
  ✓ Health/Check body is SERVING        [{"status":"SERVING"}]
==> SERVER E2E OK
```

### Cloudflare Worker (`wrangler dev`) — the SAME `app`, on the edge:

`worker/serve.nu` is the edge analog of the native `server/serve.nu`: it boots
the SAME Rauthy on :8088 in Docker, mints a real user token, runs the backend
(api) Worker via `wrangler dev` (miniflare) on :8787, asserts the SAME oidc→cedar
cases over HTTP plus the worker-specific gRPC reflection, then tears Rauthy +
wrangler down.

```sh
mise run example:worker:e2e      # needs a Docker daemon + wrangler (worker-build wasm compile is slow on the first run)
```
```
  ✓ healthz (no token)                  [200]
  ✓ Health/Check (no token) → public    [200]
  ✓ Read no-token → AuthN deny          [401]
  ✓ Read token → allow                  [200]
  ✓ Admin admin-role → allow            [200]
  ✓ Super no-superuser → deny           [403]
  ✓ GetDoc(public) → body allow         [200]
  ✓ GetDoc(secret) → body deny          [403]
  ✓ Health/Check body is SERVING        [{"status":"SERVING"}]
  ✓ ServerReflection lists demo.v1.Api  [200]   gRPC reflection (Connect stream)
==> WORKER E2E OK
```

Identical behaviour native and on `wrangler dev`/miniflare. To drive the backend
by hand instead:

```sh
wrangler dev -c examples/rauthy-cedar/worker/wrangler.toml          # → http://127.0.0.1:8787
curl -s -H "Authorization: Bearer $TOKEN" -X POST 127.0.0.1:8787/demo.v1.Api/Read                                  # 200
curl -s -H "Authorization: Bearer $TOKEN" -X POST 127.0.0.1:8787/demo.v1.Api/Super                                 # 403
curl -s -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d '{"docId":"public"}' 127.0.0.1:8787/demo.v1.Api/GetDoc  # 200
curl -s -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d '{"docId":"secret"}' 127.0.0.1:8787/demo.v1.Api/GetDoc  # 403
```

### Multi-worker gateway — inter-Worker ConnectRPC over a service binding

Everything above runs the guarded service as ONE Worker. The **gateway** shape
adds a second Worker (`gateway/`) in front of it, demonstrating **inter-Worker
ConnectRPC**: no DNS, no TLS, no public-internet hop — the gateway reaches the
backend through a Cloudflare `[[services]]` binding via connyay's
[`FetcherTransport`](https://github.com/connyay/connectrpc-workers) (the same
pattern as connyay's own `examples/multi/gateway-worker`).

```
caller ──Authorization: Bearer──▶ gateway (rauthy-cedar-gateway, :8787)
                                    │  serves gateway.v1.GatewayService/ProxyRead
                                    │  ApiClient<FetcherTransport>(env.service("API"))
                                    ▼  [[services]] binding  (no DNS/TLS/public hop)
                                  backend (rauthy-cedar-api, auxiliary worker)
                                    │  demo.v1.Api/Read — the FULL OIDC→Cedar make() stack
                                    ▼  Reply{subject, roles}  ──▶ ProxyReadResponse
```

The gateway enforces nothing itself: it **forwards the caller's `Authorization`
header** onto the backend call (via `CallOptions::with_header`), so the backend's
`OidcLayer → Cedar` stack is what authorizes. A valid token → backend 200 → the
gateway returns the echoed Session; **no token → the backend's 401 propagates
back through** the gateway as a Connect `unauthenticated` error. The gateway
reuses the backend's generated client type verbatim
(`rauthy_cedar_app::proto::demo::v1::ApiClient`) — it depends on `app/` for the
client, never re-generating `demo.v1`. The binding is declared in
[`gateway/wrangler.toml`](gateway/wrangler.toml) (`[[services]] binding = "API"`
→ `service = "rauthy-cedar-api"`).

`gateway/serve.nu` boots the SAME Rauthy, then runs **both** Workers in **one**
`wrangler dev` command — the [CF-supported way](https://developers.cloudflare.com/workers/local-development/multi-workers/)
to wire `[[services]]` bindings locally:

```sh
mise run example:gateway:e2e     # needs a Docker daemon + wrangler
# under the hood: wrangler dev -c gateway/wrangler.toml -c worker/wrangler.toml
#   first config (gateway) = primary on :8787; backend = auxiliary via the binding
```
```
  ✓ ProxyRead no-token → backend 401 propagated through gateway  [401]
  ✓ ProxyRead token → 200 via service binding; backend Session echoed  [200] {"subject":"...","roles":[...],"upstream":"rauthy-cedar-api (service binding)"}
==> GATEWAY E2E OK
```

Two local-dev wrinkles worth knowing (both handled, both documented in-code):

1. **`[build] cwd`** — each Worker's `wrangler.toml` sets `[build] cwd` to its
   crate directory (relative to the repo-root launch dir) so `worker-build` runs
   in the right crate under the multi-config `wrangler dev` — otherwise it would
   inherit the repo root and parse the workspace `Cargo.toml`. The gateway is a
   standalone wasm workspace (its own `[workspace]` + `[profile]`, excluded from
   the root workspace) exactly like the backend `worker/`.
2. **Auxiliary-worker rate-limit binding** — when the backend runs as an
   *auxiliary* Worker (under `-c gateway -c worker`), miniflare does **not**
   provision its `[[ratelimits]]` `RL` binding, so `env.rate_limiter("RL")`
   errors. The backend degrades to `AllowAll` when `RL` is absent (the sanctioned
   `AllowAll` use for "hosts that don't provision a CF rate-limit binding") so the
   request reaches the OIDC→Cedar stack instead of 500-ing. In the single-worker
   dev and in production the `RL` binding is present and the real limiter runs.

### Use the Rauthy GUI to drive the decision

```sh
mise run recipe:local rauthy             # in the vm-uncloud repo
```

| What | URL / value |
| --- | --- |
| **Rauthy admin GUI** | http://localhost:8080/auth/v1/admin |
| Login | `admin@localhost` / `LocalDevAdminPassword123456` |
| OIDC discovery | http://localhost:8080/auth/v1/.well-known/openid-configuration |
| JWKS | http://localhost:8080/auth/v1/oidc/certs |

Add a user, give/remove the `admin` role — those roles flow into the token →
`Session` → Cedar `context.roles`, so `Admin` flips between 200 and 403.

### Hard-won Rauthy details (baked into the harnesses)
- The distroless Rauthy image **panics without `/app/config.toml`** (seed it).
- A bootstrap client secret must be **exactly 64 chars** — validation uses
  `constant_time_eq_64`; bootstrap only checks `>=64`, so a longer one stores but
  can never match (`"Invalid 'client_secret'"`). Rauthy [PR #1599](https://github.com/sebadob/rauthy/pull/1599)
  adds **generated** bootstrap secrets + `rauthy bootstrap get` — the cleaner path
  once it lands in a release.
- `client_credentials` tokens have `sub: null` → use the **password grant** for a
  user token with `sub` + roles.

## The two planes

The **edge/native plane** is this `app` (run as `server/` or `worker/`). The
**server plane** — Rauthy itself — runs from `vm-uncloud/recipes/rauthy/`. The
contract between them is three values: `RAUTHY_ISSUER`, `RAUTHY_JWKS_URL`, and
the OIDC `client_id`/secret.

## Mapping reference (connectrpc-oidc → this model)

```
JWT.sub     → principal  User::"<sub>"
JWT.roles   → context.roles    (Set<String>)
JWT.scope   → context.scopes   (Set<String>, space-split)
```

See [`connectrpc-oidc/src/claims.rs`](../../crates/connectrpc-oidc/src/claims.rs)
for the `Claims` → `Session` conversion.
