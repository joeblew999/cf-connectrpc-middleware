# CLAUDE

README has the intent.

## What this crate is

A **library crate** (`cdylib + rlib`) exposing a `tower::Layer` that adds
Cedar policy authorization to ConnectRPC handlers. Designed to drop into
existing `connectrpc-workers`-based Cloudflare Workers with 2 lines of
glue code. **Not a Worker itself.**

Reference workers it must compose with (cloned alongside this repo at
`/Users/apple/workspace/go/src/github.com/connyay/`):

- `connectrpc-workers` — server-side ConnectRPC for Workers
- `example-multitenant-worker` — closest target shape; has React/Kumo frontend
- `example-connectrpc-worker` — minimal scaffold

## Version pins (must match `example-multitenant-worker/Cargo.toml`)

| Dep                | Version  |
| ------------------ | -------- |
| `connectrpc`       | `0.4`    |
| `connectrpc-build` | `0.4`    |
| `tower`            | `0.5`    |
| `http`             | `1`      |
| `http-body`        | `1`      |
| `http-body-util`   | `0.1`    |
| `worker`           | `0.8` (examples only) |
| `cedar-policy`     | `4`      |
| edition            | `2024`   |
| rust-version       | `1.88`   |

Build target: **`wasm32-unknown-unknown`** (not `wasm32-wasip1`).

## API shape (mirrors `AuthLayer` in the multitenant repo)

```
CedarLayer::new(authorizer)        // Clone, Arc<CedarAuthorizer> inside
CedarService<S>                    // Service<http::Request<B>>
```

On `call()`:
1. Read `SessionContext` (or principal type) from `req.extensions()` —
   populated by an upstream `AuthLayer`.
2. Map URL path `/pkg.Service/Method` → Cedar `Action::"pkg.Service.Method"`.
3. Call `cedar_policy::Authorizer::is_authorized`.
4. On `Decision::Deny`, short-circuit with `ConnectError::permission_denied`.

Companion helper for fine-grained, body-aware checks inside handlers
(parallels `require_session`):

```rust
require_authorized(ctx: &connectrpc::RequestContext, action, resource)
    -> Result<(), ConnectError>
```

## Module layout

```
src/lib.rs          public re-exports
src/layer.rs        CedarLayer + CedarService
src/authorizer.rs   wraps cedar_policy::Authorizer + PolicySet + Entities
src/action.rs       path → Action mapping
examples/           working integration with a stub connectrpc service
```

## Order of work

1. [x] Fix `mise.toml`: target → `wasm32-unknown-unknown`, bump
       `cedar-policy-cli` to `4.11.0`.
2. [ ] Write `Cargo.toml` with the pins above.
3. [ ] Build `authorizer.rs` + `action.rs` (unit-testable, no Worker deps).
4. [ ] Build `layer.rs` (Tower plumbing — copy `AuthLayer` skeleton,
       swap logic).
5. [ ] Add `examples/` showing it wired into a connectrpc handler.
6. [ ] `cargo build --target wasm32-unknown-unknown` is the green gate.

Frontend / Kumo UI lives in the *consumer* example workers, not in this
crate.

## .src/ workspace (mise-managed clones of the example repos)

`.src/` (gitignored) holds mutable clones of the upstream reference
repos. `patches/` (checked in) holds the diffs we apply to demonstrate
Cedar integration. Outside-of-repo clones at
`/Users/apple/workspace/go/src/github.com/connyay/` are kept as
read-only reference and not touched by these tasks.

| mise task | what |
| --- | --- |
| `mise run src:clone` | Clone the three upstream repos into `.src/` |
| `mise run src:fork`  | Fork each upstream, rewrite remotes, push main |
| `mise run src:create-branch` | Create `cedar` branch (`$WORK_BRANCH`) in each |
| `mise run src:update` | `git pull --ff-only` each |
| `mise run src:show-status` | `git status -sb` each |
| `mise run src:reset` | Nuke `.src/` and re-clone (then re-run `src:fork`) |

`.src/kumo/` is also cloned (read-only — for component source + usage
patterns). Not part of `src:*` since it's reference, not a fork target.

After `src:fork` each `.src/<repo>` has both remotes:

- `origin` → `https://github.com/joeblew999/<repo>.git` (push target)
- `upstream` → `https://github.com/connyay/<repo>.git` (read-only source)

Phases:

1. ✅ **Done**: forks exist under `joeblew999/*`; `.src/` wired with both remotes.
2. ✅ **Done**: `cedar` branch created in each fork.
3. **Now**: patch `.src/<repo>` to integrate the Cedar middleware on
   the `cedar` branch. Push commits to `origin/cedar`.
4. **Later**: PRs upstream from `cedar` branches to `connyay/*`.

## Kumo frontend (`web-kumo/` sibling)

The multitenant fork has **two** frontends now:

- `web/` — original React app, untouched, kept as visual baseline.
- `web-kumo/` — Kumo + Cloudflare Orange theme, Tailwind v4. Phase 1
  floor is in place (build succeeds). Page-by-page conversion happens
  in Phases 2-5 (see [examples/multitenant-policies/ROADMAP.md][rm]).

`mise.toml::MULTITENANT_WEB` points at `web-kumo/`. All `kumo:web-*`
tasks (install, init, dev, build) operate there.

Kumo's own repo is cloned at `.src/kumo/` for reference (component
source, examples, CLI source).

[rm]: examples/multitenant-policies/ROADMAP.md

**All mise task scripts use nushell** (`shell = "nu -c"`). Don't add
bash-based tasks.

### mise task layout

All tasks are **`noun:verb`** — no bare verbs at the top level.

- `mise:install` — bootstrap everything in `[tools]`
- `cargo:*` — every Rust workflow command (`check`, `build`, `build:release`,
  `test`, `lint`, `format`, `fix`, `watch`, `machete`, `expand`, `clean`,
  `pre-commit`)
- `cedar:*` — Cedar policy workflow (`validate`, `format`)
- `kumo:*` — frontend setup helpers (`setup`, `list-blocks`, `list-components`)
- `src:*` — `.src/` upstream-repo workspace (`clone`, `fork`, `update`,
  `show-status`, `reset`)

Shared env in `[env]`: `WASM_TARGET`, `UPSTREAM`, `FORK`, `RUST_BACKTRACE`.
Reference as `$env.WASM_TARGET` etc. in tasks — do **not** hardcode.

`cargo:pre-commit` chains `format + lint + test + machete` via `depends`.
Run it before every commit.

## Proto codegen pipeline

Two independent, language-local toolchains — **no shared system `buf`**:

- **Rust (Worker)**: `connectrpc-build = "0.4"` as a `[build-dependencies]`
  entry; `build.rs` calls `Config::new().files(...).compile()`. Protos
  compile at `cargo build` time. No `buf` CLI needed.
- **TS (frontend)**: `@bufbuild/buf` is an `npm devDependency` in `web/`,
  invoked via `buf generate` (which resolves to `node_modules/.bin/buf`).
  `protoc-gen-es` is also npm-local.

Do **not** add `aqua:bufbuild/buf` to `mise.toml` — it's redundant with
the npm-local copy and introduces version drift.

## Known traps

- `connectrpc` is at `0.4`, **not** `0.6` — earlier survey was wrong.
- Build target is `wasm32-unknown-unknown`, **not** `wasm32-wasip1` —
  comment in the example Cargo.toml confirms.
