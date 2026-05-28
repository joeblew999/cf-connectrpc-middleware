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

Active integration work lives on the `cedar` branch of
`example-multitenant-worker`. The two other forks have `cedar` branches
deleted — they were placeholders never touched. PRs upstream to
`connyay/*` happen when the cedar branch is stable.

## Kumo frontend (`web-kumo/` sibling)

> **Before any Kumo work, read [KUMO.md](./KUMO.md).** It's the
> cross-project rulebook (run `kumo ai` first, CSS layer pitfalls,
> theme-generator gotchas, etc.). The section below covers only the
> things specific to this repo.

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

- `legacy` — `legacy-styles.css` under `layer(legacy)`. Lowest precedence.
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
