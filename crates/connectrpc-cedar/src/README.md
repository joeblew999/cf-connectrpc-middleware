# connectrpc-cedar

Cedar policy authorization for Connect-RPC services on Cloudflare
Workers (and anywhere `connectrpc` + `tower` runs).

This is the **per-crate** README. The crate is the `tower::Layer`
surface; its siblings in the workspace are `connectrpc-cedar-interceptor`
(body-aware authz on the `connectrpc::Interceptor` surface) and
`connectrpc-guard` (a convenience bundle), all over the shared
`connectrpc-tower-kit` foundation.

## What ships in this crate

- `CedarAuthorizer` — wraps `cedar_policy::{Schema, PolicySet, Entities, Authorizer}`. Evaluates `is_authorized(principal, action, resource, context)`.
- `CedarLayer` — short-circuiting `tower::Layer`. `Mode::Shadow` (log-only) and `Mode::Enforce` (reject on `Decision::Deny`). `skip_paths(...)` builder for public endpoints.
- `CedarRequest` — `{ principal, action, resource, context }`.
- `CedarRequestExtractor` — trait + blanket `Fn(&Request<B>) -> Option<CedarRequest>` impl.
- `action::action_from_path` — `/pkg.Service/Method` → `Action::"pkg.Service.Method"`.

## The Cedar crates

- `connectrpc-cedar` (this crate) — the `tower::Layer`. Runs before
  envelope decode; sees the raw `http::Request` (headers + extensions).
- `connectrpc-cedar-interceptor` — the `connectrpc::Interceptor`
  surface. Runs after envelope decode, so it can authorize against the
  decoded request body (body-aware, two-trait split: header / typed).
- `connectrpc-guard` — convenience that bundles OIDC → Cedar.

The split keeps wasm size down on CF Workers: a Worker that only needs
path-based authz takes the Layer and avoids pulling in the typed-body
Interceptor's bounds.

## Two middleware shapes inside this crate

`tower::Layer<S>` for Connect-RPC takes one of two sub-shapes:

- **Transparent** — generic `impl<S, B> Service<http::Request<B>> for X<S>` where `type Response = S::Response`. Inserts into `req.extensions()`, never short-circuits. Used by a `RequestIdLayer` / `AuthLayer`.
- **Short-circuit** — pinned to `S::Response = Response<ConnectRpcBody>, S::Error = Infallible`. May construct a denial `Response<ConnectRpcBody>` from `ConnectError::permission_denied(...).to_json()` without invoking `S`. Used by **this crate** in `Mode::Enforce`.

The single short-circuit Layer is the entire surface this crate ships.

## Composing with CF Workers

The crate is `wasm32-unknown-unknown`-compatible. A Worker wires it
like this:

```rust
let cedar_authorizer = build_authorizer();                          // load schema + policies
let cedar_layer = shadow_layer::<worker::Body>(Arc::clone(&cedar_authorizer));
let svc = auth_layer.layer(cedar_layer.layer(ConnectRpcService::new(router)));
```

Order matters: `AuthLayer` inserts a session into extensions, then
`CedarLayer` reads it via the extractor.

## Mode::Shadow

`Mode::Shadow` evaluates Cedar on every request and logs the decision
via `tracing`, but **always** passes through to the inner service. Run
this in production for N days alongside the handler-side `require_*`
helpers, diff Cedar's would-have-done log against actual responses,
then flip to `Mode::Enforce` once they match.

The kit lifts this into a generic `Rollout` trait so other rejecting
middlewares (rate-limit, validation) adopt the same safe-rollout
pattern with their own enum (`Observe`/`Throttle`, `Warn`/`Reject`,
`Sample`/`All`).
