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
       └─ ApiImpl                                          + CedarInterceptor  (BODY-aware authz)
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
| rate `limiter` (`RateLimiter`) | `AllowAll` | `CfRateLimiter` over `worker::RateLimiter` (`RL` binding) |

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
└── worker/         CF WORKER    — event  + worker::Fetch JWKS + AE/RL bindings → calls app::make()
```

Each host does only what differs by platform: fetch the JWKS, run the serve
loop, and inject the metrics sink + rate limiter. The `app::make(verifier, sink,
limiter)` call that builds the entire middleware stack is **identical** in both
(`grep -n make` in `server/src/main.rs` and `worker/src/lib.rs`).

### Native (hyper) — `serve.nu` boots Rauthy, mints a token, asserts:

```sh
nu examples/rauthy-cedar/server/serve.nu      # needs a local Docker daemon
```
```
  ✓ healthz (no token)            [200]   skip path
  ✓ Read no-token → AuthN deny    [401]   OidcLayer rejects (no token)
  ✓ Read token → allow            [200]   verified + Cedar path allow
  ✓ Admin admin-role → allow      [200]   token carries `admin`
  ✓ Super no-superuser → deny     [403]   Cedar permission_denied
  ✓ GetDoc(public) → body allow   [200]   CedarInterceptor reads doc_id
  ✓ GetDoc(secret) → body deny    [403]   CedarInterceptor reads doc_id
==> SERVER E2E OK
```

### Cloudflare Worker (`wrangler dev`) — the SAME `app`, on the edge:

```sh
# worker/wrangler.toml [vars] point at a running Rauthy (e.g. :8088); the
# [[analytics_engine_datasets]] + [[ratelimits]] bindings are declared there.
cd examples/rauthy-cedar/worker && wrangler dev          # → http://127.0.0.1:8787

curl -s -H "Authorization: Bearer $TOKEN" -X POST 127.0.0.1:8787/demo.v1.Api/Read                                  # 200
curl -s -H "Authorization: Bearer $TOKEN" -X POST 127.0.0.1:8787/demo.v1.Api/Super                                 # 403
curl -s -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d '{"docId":"public"}' 127.0.0.1:8787/demo.v1.Api/GetDoc  # 200
curl -s -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d '{"docId":"secret"}' 127.0.0.1:8787/demo.v1.Api/GetDoc  # 403
```

Identical behaviour native and on `wrangler dev`/miniflare (no-token 401 ·
Read/Admin 200 · Super 403 · GetDoc public/secret 200/403).

### CLIENT transport — Worker → Connect service (`/client-demo`)

Everything above is the Worker as a Connect **server**. The Worker also proves
the **client** half of "Connect on Workers" via
[connyay/connectrpc-workers](https://github.com/connyay/connectrpc-workers):
its `FetchTransport` wraps the global `worker::Fetch` and implements
`connectrpc::client::ClientTransport`, so the generated `ApiClient<T>` (from the
same shared proto) makes a real outbound Connect call from inside the isolate.

The unauthenticated `/client-demo` route (handled before the guarded `make()`
stack, so it is not auth-gated) builds an `ApiClient` over a `FetchTransport`
pointed at `CLIENT_DEMO_TARGET` (`worker/wrangler.toml [vars]`) and calls
`demo.v1.Api/Read`, returning a JSON summary of the round trip:

```sh
# CLIENT_DEMO_TARGET unset → 200 explaining how to set it.
curl -s 127.0.0.1:8787/client-demo
# Point it at the native server or the Worker's own origin, then:
curl -s 127.0.0.1:8787/client-demo   # → {"client_demo":"ok"|"error", ...}
```

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
