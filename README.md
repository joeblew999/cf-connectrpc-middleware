# cf-connectrpc-middleware

The goal of this project it to make it as easy as possible to use ConnectRPC for Native and Cloudflare projects. As such it lives on the shoulders of the great work from Anthropic and Connyay. Then we have middleware for obvious integrations also, all working on Native and Cloudflare. Then we have KUMO GUI components to help use all the goodies in the project so that developers using ConnectRPC also have Web Kumo Componnets that work with them to make building large projects as DRY as possible, and we have example projects using it all to keep us honest.

Composable Rust `tower` middleware for [ConnectRPC](https://connectrpc.com/)
services that run **on Cloudflare Workers and natively from one codebase**.
The flagship middleware is **Cedar authorization + Rauthy OIDC** — the shared
auth layer for our projects.

## CLAUDE

Keep everthing coherent !!! DONT just make slop on layers of slop !!!

## How it works

The [`connectrpc`](https://github.com/anthropics/connect-rust) runtime gives a
`tower::Service` that compiles to `wasm32`. We wrap it in tower layers:

```
request → AuthN (verify token → Session) → AuthZ (Cedar allow/deny) → CF-ops → your service
```

The same stack runs two ways, unchanged:

- **Native** — served by `hyper`.
- **Cloudflare Worker** — the service is driven by the Worker `fetch` event.

Proven side-by-side in [`examples/rauthy-cedar/`](./examples/rauthy-cedar):
one shared app, hosted by `server/` (native) and `worker/` (edge).

## Built on

A thin layer — the heavy lifting is upstream, all on the `0.7` line:

| Repo | Role | Version |
| --- | --- | --- |
| [anthropics/connect-rust](https://github.com/anthropics/connect-rust) | ConnectRPC runtime; its `Router` is the `tower::Service` we wrap; compiles to wasm | `connectrpc 0.7` |
| [anthropics/buffa](https://github.com/anthropics/buffa) | protobuf codec `connectrpc` uses (pure-Rust, wasm-clean) | `0.7.1` (transitive) |
| [connyay/connectrpc-workers](https://github.com/connyay/connectrpc-workers) | Workers **client** transport — the only way a Worker can *call* a Connect service | `0.7` (main; no release tag yet) |
| [cloudflare/workers-rs](https://github.com/cloudflare/workers-rs) | the `worker` Workers binding: `fetch` transport + AE/Rate-Limit/KV bindings | `0.8` |
| [cedar-policy/cedar](https://github.com/cedar-policy/cedar) | authz engine; runs inside the Worker | `4` |
| [sebadob/rauthy](https://github.com/sebadob/rauthy) | OIDC identity provider (runs server-side via [vm-uncloud](https://github.com/joeblew999/vm-uncloud)) | `0.35.2` |

The root `Cargo.toml [workspace.dependencies]` is the source of truth for pins.
**A pin is a snapshot, not a freeze — we upgrade when an upstream we need moves.**

## The crates

All are `tower` layers (or interceptors) over a `connectrpc` service. An AuthN
layer puts a `Session` into request extensions; the AuthZ layer reads it and
allows or denies.

| Crate | Does |
| --- | --- |
| `connectrpc-tower-kit` | shared primitives + the `Session` (AuthN→AuthZ contract) |
| `connectrpc-oidc` | AuthN: verify a Rauthy/OIDC JWT → `Session` |
| `connectrpc-session` | AuthN: non-OIDC (opaque token / API key / macaroon) → `Session` |
| `connectrpc-cedar` | AuthZ: Cedar policy check (allow / deny) |
| `connectrpc-cedar-interceptor` | AuthZ: body-aware variant on the `Interceptor` surface |
| `connectrpc-guard` | convenience: bundles OIDC → Cedar |
| `connectrpc-cf-tracing` / `-metrics` / `-rate-limit` | CF-ops: tracing, Analytics-Engine metrics, rate limiting |

Why Cedar: multi-tenant authz is a ReBAC problem; Cedar expresses it as ~5-line
policies, type-checked against a schema, evaluated inside the Worker (no extra
service). Policies version alongside the code under `examples/*-policies/`.

## Run it

```sh
mise run cargo:test                              # native tests
nu examples/rauthy-cedar/server/serve.nu         # rauthy-cedar example, native (hyper); needs Docker
cd examples/rauthy-cedar/worker && wrangler dev  # rauthy-cedar example, on a CF Worker (wrangler)
mise run stack:local                             # whole auth stack on Docker: Rauthy + a real token → oidc→cedar (no spend)
mise tasks                                        # everything else
```

See [`examples/rauthy-cedar/README.md`](./examples/rauthy-cedar/README.md) for
both run paths and the expected output.

## Dependency source (`.src/`)

`.src/` (gitignored) holds the upstream source we read, pinned so it can't go
stale or drift from what we compile:

- `mise run src:sync` — checks out each pinned upstream: `connect-rust` and
  `buffa` at the git **tag** matching `Cargo.lock`; `connectrpc-workers` at a
  fixed **commit** (it has no 0.7 release tag yet).
- `mise run src:check` — verifies each clone is at its pin; flags any that drifted.

**Never trust a clone for version facts** — check upstream `main`/crates.io.
`cargo update` refreshes the lock in-semver (cheap freshness).

> **Known blocker:** [mathematic-inc/protovalidate-buffa](https://github.com/mathematic-inc/protovalidate-buffa)
> (request validation) is still on `buffa 0.6`; can't adopt until it bumps to 0.7.

## Layout

```
crates/      the tower middleware crates (above)
examples/    rauthy-cedar (native + worker proof) · *-policies (Cedar policy sets)
packages/    kumo-connectrpc-kit (TS client) + frontend
scripts/     nushell task implementations
.src/        gitignored: pinned upstream source (src:sync) + connyay reference workers
```

All tasks run through **mise** (`mise tasks`); task scripts are **nushell**.
Secrets via **fnox** + keychain (per-repo `fnox.toml`).
