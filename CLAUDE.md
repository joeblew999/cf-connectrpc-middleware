# CLAUDE

README has the intent.

## What this repo is

**Today**: a single library crate (`cdylib + rlib`) exposing a
`tower::Layer` that adds Cedar policy authorization to ConnectRPC
handlers. Drops into existing `connectrpc-workers`-based Cloudflare
Workers with ~2 lines of glue.

**Where it's going**: a Cargo workspace containing a **family of Cedar
crates** (Layer + Interceptor + macros) sharing a `connectrpc-cedar-core`,
on top of a generic `connectrpc-tower-kit`. Plus a set of CF-ops
middleware crates (tracing, metrics, rate-limit, idempotency, trace-context,
access). The plan is in [`MIDDLEWARES.md`](./MIDDLEWARES.md) — read that
before designing any new abstraction. The §6 "what the catalog tells us"
list of six recurring patterns is binding.

**Not a Worker itself** — this stays a library.

Reference workers it must compose with (cloned at
`/Users/apple/workspace/go/src/github.com/connyay/` + in `.src/`):

- `connectrpc-workers` — server-side ConnectRPC for Workers
- `example-multitenant-worker` — closest target shape; has React/Kumo frontend
- `example-connectrpc-worker` — minimal scaffold
- `EdgeReplica` — connyay's other CF Worker; uses the same middleware module convention

## Version pins (must match `example-multitenant-worker/Cargo.toml`)

| Dep                | Version  |
| ------------------ | -------- |
| `connectrpc`       | `0.6`    |
| `connectrpc-build` | `0.6`    |
| `tower`            | `0.5`    |
| `http`             | `1`      |
| `http-body`        | `1`      |
| `http-body-util`   | `0.1`    |
| `worker`           | `0.8` (examples only) |
| `cedar-policy`     | `4`      |
| edition            | `2024`   |
| rust-version       | `1.88`   |

Build target: **`wasm32-unknown-unknown`** (not `wasm32-wasip1`).

## API shape — today (single crate, will split)

```
CedarLayer::shadow(authorizer, extractor)    // log-only
CedarLayer::enforce(authorizer, extractor)   // reject on Deny
CedarLayer::skip_paths([...])                // public endpoints (health checks, OAuth callbacks)
CedarService<S, E>                           // Service<http::Request<B>>
```

On `call()`:
1. Bail if path matches `skip_paths`.
2. Extractor reads `SessionContext` from `req.extensions()` → `CedarRequest`.
3. `CedarAuthorizer::is_authorized(...)`.
4. `Mode::Shadow` → log + pass through. `Mode::Enforce` + `Decision::Deny` → short-circuit with a Connect-protocol error response (`ConnectError::permission_denied(...).to_json()` body, `Error = Infallible`).

## Family plan (what supersedes the single-crate API above)

The plan in [`MIDDLEWARES.md`](./MIDDLEWARES.md):

```
connectrpc-tower-kit          shared primitives — no middleware
       │
       ├─ connectrpc-cedar-core            CedarAuthorizer + Request type
       │       │
       │       ├─ connectrpc-cedar-layer         (this src/ today)
       │       ├─ connectrpc-cedar-interceptor   header-only + typed-body, two traits
       │       └─ connectrpc-cedar-macros        #[require_authorized(action=..., resource=...)]
```

Handler-side helpers stay alongside the middleware family — defense in
depth + body-field-specific checks the Layer can't see. We are
**not** deleting handler-side `require_*` after shadow mode; that
earlier note in `example-multitenant-worker/src/middleware/cedar.rs`
was wrong.

## Module layout (transitional)

Today (pre-workspace):

```
src/lib.rs          public re-exports
src/layer.rs        CedarLayer + CedarService (short-circuiting)
src/authorizer.rs   wraps cedar_policy::Authorizer + PolicySet + Entities
src/action.rs       path → Action mapping
src/extract.rs      CedarRequest + CedarRequestExtractor trait
examples/           working integration with a stub connectrpc service
tests/editorial.rs  end-to-end against examples/multitenant-policies/
```

After workspace conversion (planned next):

```
crates/connectrpc-tower-kit/             new — Rollout trait, Chain<I>, denial-response builder
crates/connectrpc-cedar-core/            new — CedarAuthorizer + types (no Layer)
crates/connectrpc-cedar-layer/           current src/ split out
crates/connectrpc-cedar-interceptor/     BUILT — body-aware authz on connectrpc::Interceptor (0.6)
crates/connectrpc-cedar-macros/          future — proc-macro
```

Frontend / Kumo UI lives in the *consumer* example workers, not in this
repo.

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

Active integration work lives on the `cedar` branch of
`example-multitenant-worker`. The two other forks have `cedar` branches
deleted — they were placeholders never touched. PRs upstream to
`connyay/*` happen when the cedar branch is stable.

## Kumo frontend (`web-kumo/` sibling)

> Read [KUMO.md](./KUMO.md) before any Kumo work, and [SEED.md](./SEED.md)
> before touching demo scenarios — this section is repo-specific only.

The multitenant fork has **two** frontends now:

- `web/` — original React app, **the visual source of truth**. Editorial
  dark-mode design, red `#d71921` accent, Space Grotesk + Doto type.
  Only contains Login / Signup / Dashboard / AcceptInvite.
- `web-kumo/` — same look-and-feel, rebuilt on `@cloudflare/kumo`
  primitives so we get Breadcrumbs / Banner / PageHeader / Sidebar etc.
  for free. Kumo's default Cloudflare-Orange tokens are overridden by a
  proper *generated* theme — `src/styles/theme-editorial.css` emitted by
  `scripts/theme-generator/generate.mjs` from `config.editorial.mjs`.
  Hand-authored selectors that the token system can't express (sidebar
  button reset, h1 Doto font) live in `src/styles/theme-editorial-extras.css`.
  Layout fixes that apply across all themes (shell width when AppShell
  is mounted, mobile topbar wrap, mobile sidebar overlay) live in
  `src/styles/layout-fixes.css`.
  Members / Invitations / Billing are **net-new** here — no `web/`
  precedent. Design them in the editorial style; if Kumo doesn't have a
  block, install one with `mise run kumo:add-block -- <Name>`.

### Theme architecture (repo-specific)

Editorial = `[data-theme="editorial"]`, declared in `index.html` for
first-paint correctness. The `ThemeToggle` is mounted **only** on
`/preview` as a dev A/B affordance; it resets to editorial on unmount.
No localStorage, no URL param, no persistence — see [KUMO.md §9](./KUMO.md)
for the reasoning (race conditions with agents).

Token changes go through `scripts/theme-generator/config.editorial.mjs`
+ `mise run kumo:theme-gen`. The generator validates against Kumo's
exported `THEME_CONFIG` and emits **unlayered** CSS — see
[KUMO.md §3](./KUMO.md) (cascade trap) and [KUMO.md §4](./KUMO.md)
(why we hand-roll instead of using Kumo's internal generator).

`mise.toml::MULTITENANT_WEB` points at `web-kumo/`. All `kumo:*` tasks
operate there.

### Dev URLs (concrete ports)

See [KUMO.md §8](./KUMO.md) for the generic 3-server pattern. This
repo's instantiation:

| URL                          | Server   | When to use                                  |
| ---------------------------- | -------- | -------------------------------------------- |
| `https://localhost:5173/`    | Vite     | Real Chrome iteration. HMR is instant.       |
| `http://localhost:5175/`     | Vite     | Headless screenshot tools (Claude preview, Playwright). HTTP-only. Started by `mise run kumo:web-dev-http` via `vite.config.http.ts`. |
| `https://localhost:8787/`    | Wrangler | Production-like wasm path. Stale until `mise run kumo:web-reload`. |

Pitchfork supervises both HTTPS daemons (`worker` on :8787, `web` on
:5173); the HTTP variant on :5175 is launched on demand.

Kumo's own repo is cloned at `.src/kumo/` for reference.

[rm]: examples/multitenant-policies/ROADMAP.md

**All mise task scripts use nushell** (`shell = "nu -c"`). Don't add
bash-based tasks.

### Cloning this stack to a new project

This repo is a template. To reuse the editorial-Kumo + CF-deploy +
seed pattern in another project:

**Portable bits — copy as-is, edit names:**

- `scripts/theme-generator/{config.editorial.mjs,generate.mjs}` — works
  for any Kumo project. Edit `config.editorial.mjs` token values for the
  new palette; everything else stays.
- `src/styles/{theme-editorial.css,theme-editorial-extras.css,layout-fixes.css}`
  + `@source "../node_modules/@cloudflare/kumo/dist"` in `styles.css`.
- `src/components/ThemeToggle.tsx` + the topnav wiring.
- `vite.config.http.ts` + `dev:http` script for headless screenshot tools.
- `scripts/seed.mjs` skeleton — keep the `rpc()` + idempotent-detect
  pattern; swap the specific RPC calls for the new project's protos.
- The whole `cf:*` / `worker:deploy` / `worker:teardown` / `seed:prod`
  mise task family — keep verbatim except for keychain item names and
  the D1 database name.
- `fnox.toml` shape with `service = "fnox"` + repo-prefixed item names.

**Repo-specific — find/replace per new project:**

- `MULTITENANT_WEB` / `MULTITENANT_WORKER` env vars → e.g. `WEB` / `WORKER`.
- `CONNECTRPC_CEDAR_*` keychain item names → `<NEW_PROJECT>_*` (per the
  fnox cross-repo contract — names *are* the API boundary).
- `workers-multitenant-cedar` D1 database name → `workers-<project>`.
- `UPSTREAM` / `FORK` / `WORK_BRANCH` if not using the `.src/` fork
  pattern.
- Page components + seed RPC calls.

**Conventions that make the stack legible:**

- All tasks are `noun:verb` — no bare verbs at the top level.
- Five top-level namespaces: `cargo`, `cedar`, `cf`, `dev`, `kumo`,
  `seed`, `src`, `worker`. Each maps to one concern.
- `cf:bootstrap` (one-shot secret gen) is separate from `worker:deploy`
  (idempotent re-run loop). Bootstrap rotates SESSION_KEY — don't run it
  on an existing prod with users.
- `worker:teardown` is destructive but interactive — requires typing the
  worker name to confirm.
- `seed:dev` and `seed:prod` share `scripts/seed.mjs` — `BASE` env var
  picks the target. `.seed.json` always gitignored.
- Theme token changes go through `scripts/theme-generator/` and
  `mise run kumo:theme-gen`, never hand-edit the generated CSS.
- Non-token selector fixes (sidebar reset, h1 fonts) go in
  `theme-editorial-extras.css`; cross-theme layout in `layout-fixes.css`.

When porting, the cheapest validation is: install Kumo, run
`mise run kumo:list-blocks`, install one with `mise run kumo:add-block
-- PageHeader`, confirm it picks up the new theme via the ThemeToggle.
If that works, the rest will too.

### Kumo CLI (mise wrappers)

See [KUMO.md §0–§1](./KUMO.md) for the rules (`kumo ai` first, every
session). These are the mise wrappers in this repo:

| Task | What |
| --- | --- |
| `mise run kumo:ai` | Canonical AI usage guide. Run this FIRST every session. |
| `mise run kumo:doc -- <Name>` | Docs for one component. |
| `mise run kumo:docs` | Docs for all 42 primitives. |
| `mise run kumo:list-blocks` | Installable layout blocks (PageHeader / ResourceListPage / DeleteResource). |
| `mise run kumo:add-block -- <Name>` | Copy block source into `src/components/kumo/`. |
| `mise run kumo:migrate` | Token rename map for version bumps. |
| `mise run kumo:list-components` | Show what's in `src/components/kumo/`. |

CSS import order in `styles.css` is opinionated — see [KUMO.md §2](./KUMO.md).

### CSS architecture (cascade layers)

[styles.css][styles] declares this order:

```
@layer legacy, theme, base, components, utilities, editorial;
```

- `legacy` — `editorial-chrome.css` under `layer(legacy)`. Lowest precedence. Holds base resets + the `.shell/.topbar/.brand/.page` chrome classes; the palette is generated separately (see Theme architecture).
- `theme` / `base` / `components` / `utilities` — Tailwind v4 + Kumo.
- `editorial` — `theme-editorial-extras.css` + `layout-fixes.css` under
  `layer(editorial)`. Highest precedence among layered rules.

**The trap that bit us twice**: Kumo emits *unlayered* `:root, :host`
defaults for `--color-kumo-*` tokens. Per the cascade-layers spec,
unlayered rules beat ANY layered rule regardless of specificity. So
custom theme overrides MUST also be unlayered — see [KUMO.md §3](./KUMO.md).
The theme generator handles this for token overrides; if you author
selector overrides by hand in the `editorial` layer, A/B-test under
each theme via the `/preview` ThemeToggle.

### Porting mobile fixes

`layout-fixes.css` annotates each rule with `REUSABLE WHEN: [tag]`:

- `[universal]` — pure HTML/CSS; works in any project (table h-scroll,
  long-ID truncation).
- `[Kumo]` — needs `@cloudflare/kumo` (AppShell padding, sidebar
  overlay z-index).
- `[editorial]` — needs the legacy editorial shell (`.shell`, `.topbar`,
  `.topnav` from `web/src/App.tsx` pattern).

When porting, skim the file and drop rules whose tags don't apply.
Most projects keep `[universal]` + `[Kumo]`; only repos that lifted the
editorial shell keep `[editorial]`.

[styles]: .src/example-multitenant-worker/web-kumo/src/styles.css

### mise task layout

All tasks are `noun:verb` — run `mise tasks` to see the full inventory.
Namespaces:

| ns | what |
| --- | --- |
| `mise` | bootstrap |
| `cargo` | Rust dev loop |
| `cedar` | Cedar policy workflow |
| `worker` | wrangler dev + D1 migrations + deploy + teardown |
| `kumo`   | frontend dev, build, theme-gen, block install, `kumo:ai` |
| `cf`     | CF auth check + SESSION_KEY bootstrap |
| `seed`   | populate local/remote D1 with `.example` users + orgs + invites |
| `dev`    | pitchfork supervises long-running daemons |
| `src`    | `.src/` fork workspace |

Shared env in `[env]`: `WASM_TARGET`, `UPSTREAM`, `FORK`,
`MULTITENANT_WEB`, `MULTITENANT_WORKER`, `RUST_BACKTRACE`.
Reference as `$env.WASM_TARGET` etc. — do **not** hardcode.

`cargo:pre-commit` chains `format + lint + test + machete` via `depends`.
Run before every commit.

### Cedar GUI / MCP tooling (cedar:* extensions)

Beyond `cedar:validate/format/test`, the `cedar:*` namespace wraps Cedar's
editor + agent tooling:

| Task | What |
| --- | --- |
| `cedar:tools` | One-time build of the two git-only binaries into `.tools/bin` (see trap below). Prereq for `cedar:lsp` + `cedar:schema:from-mcp`. |
| `cedar:viz` | `cedar visualize --entities` → `<policy-dir>/entities.dot` (Graphviz) per `examples/*-policies/tests/entities.json`. Writes `.svg` only if `dot` is on PATH (graphviz isn't in mise's registry, so `.dot`-only by default). |
| `cedar:lsp` | Runs `cedar-language-server` over stdio (Neovim/Helix). VS Code users want the extension — see [.vscode/extensions.json](./.vscode/extensions.json) (`cedar-policy.vscode-cedar`). |
| `cedar:mcp:install` | Clones `cedar-policy/cedar-for-agents` into `.src/` and `npm`-builds the **analysis MCP server** (verify-policy-changes / detect-policy-issues). Upstream ships it Docker-only; we build from source with the mise node toolchain. |
| `cedar:mcp:config` | Prints the `mcpServers` JSON block (node → built `dist/server.js`) to paste into a Claude/Q MCP client. Point it at `examples/remy-sport-policies/policies/*.cedar` to reason about the matrix before shipping a change. |
| `cedar:schema:from-mcp` | `cedar-policy-mcp-schema-generator generate <stub> <tools.json>` — turns an MCP tool manifest into a Cedar schema. |

Only **one** Cedar tool is a clean mise registry install: `cargo:cedar-policy-cli`
(the `cedar` binary, on crates.io) — it powers `cedar:validate/format/viz`. The
other three are NOT distributed as binaries by upstream (see Known traps).

## Proto codegen pipeline

Two independent, language-local toolchains — **no shared system `buf`**:

- **Rust (Worker)**: `connectrpc-build = "0.6"` as a `[build-dependencies]`
  entry; `build.rs` calls `Config::new().files(...).compile()`. Protos
  compile at `cargo build` time. **connectrpc-build 0.6 shells out to
  `protoc`** (0.4 was pure-Rust), so `mise.toml` pins
  `aqua:protocolbuffers/protobuf/protoc`. Still no `buf` — protoc is the
  compiler; `buf` is the wrapper we avoid.
- **TS (frontend)**: `@bufbuild/buf` is an `npm devDependency` in `web/`,
  invoked via `buf generate` (which resolves to `node_modules/.bin/buf`).
  `protoc-gen-es` is also npm-local.

Do **not** add `aqua:bufbuild/buf` to `mise.toml` — it's redundant with
the npm-local copy and introduces version drift. (`protoc` IS in
`mise.toml`, required by connectrpc-build 0.6 codegen — that's different.)

## Known traps

- `connectrpc` is at `0.6` (migrated from 0.4.2 on 2026-06-06). The 0.x
  scheme makes each minor breaking, so 0.4→0.6 spanned two; the only code
  impact was `RequestContext` field reads → accessor methods
  (`ctx.extensions` → `ctx.extensions()` / `ctx.extensions_mut()`). The
  library crates needed zero changes. **connectrpc-build 0.6 now requires
  `protoc`** — see Proto codegen pipeline.
- Build target is `wasm32-unknown-unknown`, **not** `wasm32-wasip1` —
  comment in the example Cargo.toml confirms.
- `connectrpc::Interceptor` (surface #3 in MIDDLEWARES.md §1) **shipped in
  connectrpc 0.6** (it didn't exist in the 0.4.2 we used to pin). Both
  deferred pieces now exist: `connectrpc-cedar-interceptor` (new crate,
  body-aware authz) and `connectrpc-cf-metrics`'s `interceptor` module
  (`MetricsInterceptor`, `Spec::procedure` labels). The example worker
  wires cedar + metrics as **interceptors** on the `ConnectRpcService`
  (`.with_interceptor(..)`); cf_tracing / cf_rate_limit / auth stay tower
  Layers. Note: the Interceptor trait requires `Send + Sync + 'static`,
  which worker 0.8's AE binding satisfies on wasm32 (verified building).
- **`tracing_subscriber::fmt` on wasm32 panics by default**: the
  default time format calls `std::time::SystemTime::now()`, which is
  unsupported on `wasm32-unknown-unknown`. The fmt layer's
  `on_event` panics with "unreachable" on every event, surfacing as
  a 500 + hung-request error in `wrangler dev`. **Fix**: call
  `.without_time()` on the fmt builder. CF already attaches its own
  timestamps via `wrangler tail` / Logpush. See
  `.src/example-multitenant-worker/src/observability.rs`.
- **`worker::Delay` is `!Send`**: it holds a JS closure. If your
  middleware uses `Delay` (e.g. timing out a binding call) and the
  trait it adapts requires `Send` futures (most of ours do, for
  `tower::Service` bounds), wrap with `worker::send::SendFuture::new`.
  Sound on Workers because the runtime is single-threaded.
- **CF Rate Limiting binding is "remote" in `wrangler dev`**: the JS
  promise from `worker::RateLimiter::limit(...)` can stall locally
  (binding handshakes but no Cloudflare backend responds). Always
  guard CF-binding calls with a short timeout (we use 500ms in
  `cf_rate_limit.rs`) so the layer fails-open instead of hanging
  ~15s before miniflare cancels the request.
- **Local D1 needs migrating before signup works**: `mise run
  worker:d1-migrate` applies `migrations/0001_init.sql` to
  `.wrangler/state/v3/d1` — without this, signup returns 500 from
  inside the handler (the layer stack is fine, the DB write fails).
  Easy to forget after a fresh clone or `worker:teardown`.
- **Cedar's LSP + MCP-schema-generator are NOT registry-distributed**:
  `cedar-language-server` 404s on crates.io (no release tag, no binary
  asset anywhere) and `cedar-policy-mcp-schema-generator` v0.6.0 ships
  **0 release assets**. So neither can be a `cargo:` entry in `[tools]`.
  Three compounding reasons the naive `cargo:` entry fails: (1) not on
  crates.io; (2) mise's `cargo:` backend can't select a single member
  from a git **workspace** — it mangles `version = "rev:<sha>"` into
  `<crate>@rev:<sha>` and cargo rejects it; (3) this machine's
  `cargo-binstall` shim is ambiguous (two versions, no default), which
  breaks mise's cargo backend entirely. **Fix**: the `cedar:tools` task
  runs explicit `cargo install --git <url> --rev <pinned-sha> <crate>
  [--features cli] --root .tools`, which targets the workspace member
  correctly. Binaries land in `.tools/bin` (gitignored), put on PATH by
  `[env] _.path`. Only `cargo:cedar-policy-cli` (the `cedar` binary) is
  a clean registry install. If upstream ever attaches release binaries,
  switch the two to mise's `ubi:` backend and drop the source compile.
- **The analysis MCP server is Docker-only upstream**: not on npm.
  `cedar:mcp:install` builds it from a `.src/cedar-for-agents` clone via
  `npm ci` + `tsc`; `cedar:mcp:config` points `node` at the built
  `dist/server.js` (no Docker — lighter + OS-neutral).
