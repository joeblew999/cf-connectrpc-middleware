# Upstream dependency tracking

This repo is the **hub**: other repos build on its crates, so its pins set the
line they follow. A pin is a **snapshot**, not a freeze. Upgrade when an
upstream moves and it buys us something. This file is the map. The pins
themselves live in the root `Cargo.toml [workspace.dependencies]` and the two
standalone Worker workspaces (`examples/rauthy-cedar/{worker,gateway}`).

_Last checked: 2026-10-22._ Cross-repo rollout tracker: [#4](https://github.com/joeblew999/cf-connectrpc-middleware/issues/4).

## What we depend on

| Dep | We pin | Latest published | Source | Notes |
|---|---|---|---|---|
| `connectrpc` | **0.9.1** | 0.9.1 (2026-09-21) | [anthropics/connect-rust](https://github.com/anthropics/connect-rust) | `default-features = false` drops the JSON codec (opt-in since 0.8). The middleware never encodes messages, so it doesn't need it. Serving crates (`examples/rauthy-cedar/{app,gateway}`) add `features = ["json"]`, otherwise you get **HTTP 415** (guarded by `connect_json_request_is_served`). |
| `connectrpc-build` | **0.9.0** | 0.9.0 (2026-08-25) | anthropics/connect-rust | build.rs codegen; moves with `connectrpc`. Shells out to `protoc` (mise-pinned). |
| `buffa` / `buffa-types` / `buffa-descriptor` | **0.9.2** | 0.9.2 (2026-09-03) | [anthropics/buffa](https://github.com/anthropics/buffa) | Moves in **lockstep** with `connectrpc`. Never bump it alone. |
| `worker` / `worker-macros` | **0.8.7** | 0.8.7 (2026-09-25) | [cloudflare/workers-rs](https://github.com/cloudflare/workers-rs) | Needs `rust-version` ≥ 1.91. `wrangler.toml` installs `worker-build@^0.8` (= 0.8.7), which always passes `--force-enable-abort-handler`, so the lock needs **wasm-bindgen ≥ 0.2.125** (we're on 0.2.129). |
| `connectrpc-workers` | **0.6.0** | 0.6.0 (2026-09-06) | [connyay/connectrpc-workers](https://github.com/connyay/connectrpc-workers) | Gateway example only: `FetcherTransport` over a `[[services]]` binding. |
| `cedar-policy` | `4` (lock 4.14.0) | 4.14.0 (2026-10-17) | [cedar-policy/cedar](https://github.com/cedar-policy/cedar) | Runtime authz engine. `cedar-policy-cli` 4.11.0 is mise-pinned for `cedar:*` tasks. |
| `tower` | `0.5` (lock 0.5.3) | 0.5.3 | [tower-rs/tower](https://github.com/tower-rs/tower) | Every middleware is a `tower::Layer`. |
| Rauthy | 0.35.2 | — | [sebadob/rauthy](https://github.com/sebadob/rauthy) | OIDC IdP for the examples and e2e (Docker); runs server-side via vm-uncloud. |
| `@connectrpc/connect` (+ `-web`) | `^2` | — | [connectrpc/connect-es](https://github.com/connectrpc/connect-es) | Peer dep of `packages/kumo-connectrpc-kit` (TS, published to JSR). |

`.src/` holds read-only clones at these pins (`mise run src:sync` / `src:check`).

## Who depends on us

| Consumer | How | Pin |
|---|---|---|
| [stripe-connectrpc](https://github.com/joeblew999/stripe-connectrpc) | `connectrpc-guard` + `connectrpc-oidc` by **git rev** | `58a3781` |
| [example-multitenant-worker](https://github.com/joeblew999/example-multitenant-worker) (`cedar` branch) | **path** deps on `crates/*`; builds only from `.src/example-multitenant-worker` | tracks `main` |
| stripe-connectrpc `service/web` | `@joeblew999/kumo-connectrpc-kit` from JSR | `^0.0.8` |

[cf-do-locator](https://github.com/joeblew999/cf-do-locator), [orangevault-admin](https://github.com/joeblew999/orangevault-admin) and [google_maps](https://github.com/joeblew999/google_maps) also use connectrpc, but they don't link these crates. They track the same connectrpc line only so everyone shares one set of gotchas.

## Upgrade order for a connectrpc bump

1. **This repo first**: bump the workspace `connectrpc` / `buffa` + both Worker workspaces, `cargo update` all three lockfiles. Then run `cargo:check`, `cargo:lint`, `cargo:test`, `example:test`, and `worker-build --release` for worker + gateway. Optionally run `example:{server,worker,gateway}` live (Docker + Rauthy).
2. **Then the consumers, in parallel:** re-pin stripe-connectrpc to the new rev (its lock must resolve exactly ONE `connectrpc`), and rebuild example-multitenant-worker `cedar` from `.src/`.
3. **Independent services any time**: cf-do-locator, orangevault-admin, google_maps.

Breaking changes hit in the 0.7 → 0.9 bump: interceptor types moved to `connectrpc::interceptor::{UnaryRequest, UnaryResponse}`, JSON became a feature, and the `guard` now needs `S::Future: Send`.
