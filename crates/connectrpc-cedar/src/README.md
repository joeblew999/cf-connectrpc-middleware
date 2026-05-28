# connectrpc-cedar

Cedar policy authorization for Connect-RPC services on Cloudflare
Workers (and anywhere `connectrpc` + `tower` runs).

This file is the **per-crate** README. For the broader Connect-RPC
middleware landscape — every middleware we found, their shapes, what
works on CF Workers, and how this crate fits in — see [`../MIDDLEWARES.md`](../MIDDLEWARES.md).

## Status

**Pre-1.0, single crate today, planned family of crates.** The single
`connectrpc-cedar` crate currently bundles everything (authorizer +
layer + extractor). It will split into a family of three Cedar crates
sharing a core, on top of a generic kit. See [§ Family plan](#family-plan).

What ships in this src/ today:

- `CedarAuthorizer` — wraps `cedar_policy::{Schema, PolicySet, Entities, Authorizer}`. Evaluates `is_authorized(principal, action, resource, context)`.
- `CedarLayer` — short-circuiting `tower::Layer`. `Mode::Shadow` (log-only) and `Mode::Enforce` (reject on `Decision::Deny`). `skip_paths(...)` builder for public endpoints.
- `CedarRequest` — `{ principal, action, resource, context }`.
- `CedarRequestExtractor` — trait + blanket `Fn(&Request<B>) -> Option<CedarRequest>` impl.
- `action::action_from_path` — `/pkg.Service/Method` → `Action::"pkg.Service.Method"`.

Currently deployed on CF Workers via [`example-multitenant-worker`](.src/example-multitenant-worker)
in `Mode::Shadow`. See [`MIDDLEWARES.md` §4](../MIDDLEWARES.md) for the
"Tier 1 verified" entry.

## Family plan

When the dust settles, this repo becomes a Cargo workspace with the
following crates (see [`MIDDLEWARES.md` §6](../MIDDLEWARES.md) for the
six surfaces, and the bottom of `MIDDLEWARES.md` for the CF-ops family):

```
connectrpc-tower-kit         shared primitives — Rollout trait, static Chain<I>,
                             denial-response builder, canonical extension-type
                             names, pin_project Future enum. No middleware.
       │
       ├─ connectrpc-cedar-core         CedarAuthorizer (Schema + PolicySet + Entities)
       │       │
       │       ├─ connectrpc-cedar-layer       what this src/ is becoming
       │       ├─ connectrpc-cedar-interceptor body-aware, two-trait split (header / typed)
       │       └─ connectrpc-cedar-macros      #[require_authorized(...)] proc-macro
```

The split exists because:

- **wasm size matters on CF Workers**: a Worker that only needs path-based
  authz shouldn't pull in proc-macro infrastructure or `prost`/`serde`
  bounds from the typed-body Interceptor.
- **surface conventions need to crystallize together**: shipping three
  Cedar surfaces in one release stress-tests the kit's abstractions
  against three real consumers immediately. If we only ship the Layer,
  the kit over-fits to one shape.
- **canonical layout for consumers**: each crate drops into the
  consumer's `src/middleware/cedar_{layer,interceptor,macros}.rs`,
  matching [the connyay convention](../MIDDLEWARES.md) `middleware/request_id.rs +
  session_auth.rs + mod.rs` shared between their two CF Workers.

## Two middleware shapes inside this crate today

`tower::Layer<S>` for Connect-RPC takes one of two sub-shapes; the kit
will eventually formalize this:

- **Transparent** — generic `impl<S, B> Service<http::Request<B>> for X<S>` where `type Response = S::Response`. Inserts into `req.extensions()`, never short-circuits. Used by `RequestIdLayer` / `AuthLayer` in the connyay workers.
- **Short-circuit** — pinned to `S::Response = Response<ConnectRpcBody>, S::Error = Infallible`. May construct a denial `Response<ConnectRpcBody>` from `ConnectError::permission_denied(...).to_json()` without invoking `S`. Used by **this crate** in `Mode::Enforce`.

The four other surfaces (`Interceptor`, `ConnectRpcService` config,
handler-helper, proc-macro decorator) are documented in
[`MIDDLEWARES.md` §1](../MIDDLEWARES.md). The single short-circuit
Layer is the entire surface we ship today.

## Composing with CF Workers

The crate is `wasm32-unknown-unknown`-compatible. The
[`example-multitenant-worker`](.src/example-multitenant-worker) wires
it like this:

```rust
let cedar_authorizer = build_authorizer();                          // load schema + policies
let cedar_layer = shadow_layer::<worker::Body>(Arc::clone(&cedar_authorizer));
let svc = auth_layer.layer(cedar_layer.layer(ConnectRpcService::new(router)));
```

Order matters: `AuthLayer` inserts `SessionContext` into extensions, then
`CedarLayer` reads it via the extractor. The kit will eventually
prescribe canonical ordering (see [`MIDDLEWARES.md` §3 stack ordering](../MIDDLEWARES.md)).

## Mode::Shadow

`Mode::Shadow` evaluates Cedar on every request and logs the decision
via `tracing`, but **always** passes through to the inner service. Run
this in production for N days alongside the handler-side `require_*`
helpers, diff Cedar's would-have-done log against actual responses,
then flip to `Mode::Enforce` once they match.

This is the differentiator vs every middleware in the catalog —
none of the 30+ middlewares we surveyed has a comparable rollout
toggle. The kit will lift this into a generic `Rollout` trait so
future rate-limit / validation / tracing-sampling middlewares can
adopt the same safe-rollout pattern with their own enum
(`Observe`/`Throttle`, `Warn`/`Reject`, `Sample`/`All`).

## What this README used to contain

Earlier versions had a `gh search code` query list, a prior-art table,
and a "verified gaps in the public ecosystem" section. Those have all
moved to [`../MIDDLEWARES.md`](../MIDDLEWARES.md), which is the
canonical catalog. Per-crate README stays focused on what *this* crate
does and how the family hangs together.
