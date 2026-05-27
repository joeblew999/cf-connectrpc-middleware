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

### Theme architecture

The editorial theme is structurally identical to Kumo's own
`theme-kumo.css` / `theme-fedramp.css`: it's a CSS file scoped to
`[data-theme="editorial"]` so flipping themes is a one-attribute change.
The `ThemeToggle` component in the topnav lets you A/B between
`editorial` and default `kumo` at runtime (choice persists to
`localStorage["wm.theme"]`, also accepts `?theme=…` URL param).

To change the editorial palette, **always edit `config.editorial.mjs`
and re-run `mise run kumo:theme-gen`** — don't hand-edit the generated
CSS. The generator validates token names against
`@cloudflare/kumo/scripts/theme-generator/config` and warns on typos.

Tailwind v4 only scans source files for utility class names, and Kumo's
classes live inside `node_modules/@cloudflare/kumo/dist`. `styles.css`
adds `@source "../node_modules/@cloudflare/kumo/dist"` so Tailwind emits
rules for `.bg-kumo-badge-orange`, `.bg-kumo-info-tint`, etc. Without
that line, badges and banners render colorless.

`mise.toml::MULTITENANT_WEB` points at `web-kumo/`. All `kumo:web-*`
tasks (install, init, dev, build) operate there.

### Dev URLs (three of them — pick the right one)

| URL                          | Server   | When to use                                  |
| ---------------------------- | -------- | -------------------------------------------- |
| `https://localhost:5173/`    | Vite     | Real Chrome iteration. HMR is instant.       |
| `http://localhost:5175/`     | Vite     | Headless screenshot tools (Claude preview, Playwright). HTTP-only — no cert dance. Started by `mise run kumo:web-dev-http` via `vite.config.http.ts`. |
| `https://localhost:8787/`    | Wrangler | Production-like wasm path. Stale until you `mise run kumo:web-reload` (= build + restart). |

Pitchfork supervises both HTTPS daemons (`worker` on :8787, `web` on
:5173); the HTTP variant on :5175 is launched on demand.

Kumo's own repo is cloned at `.src/kumo/` for reference (component
source, examples, CLI source).

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

### Using the Kumo CLI properly

The kumo CLI has more than `add` / `ls`. The most useful command for
agents:

- **`mise run kumo:ai`** — prints Kumo's official AI usage guide. The
  canonical reference for every component's variants, props, and
  compound-subcomponent API (`<Dialog.Root>`, `<Combobox.Item>`, etc.).
  **Run this before authoring a new Kumo-component-heavy page or
  showcase entry** — it saves the trial-and-error of guessing
  `Dialog.Trigger render={...}` vs `<DialogTrigger asChild>` (the
  former is correct).
- **`mise run kumo:doc -- <Name>`** — full doc for one component.
- **`mise run kumo:docs`** — docs for ALL 42 primitives.
- **`mise run kumo:list-blocks`** — installable layout blocks (just 3:
  PageHeader, ResourceListPage, DeleteResource).
- **`mise run kumo:add-block -- <Name>`** — copy a block's source into
  `src/components/kumo/`. Interactive prompt only on overwrite.
- **`mise run kumo:migrate`** — token rename map when bumping Kumo.
- **`mise run kumo:list-components`** — show what's already in
  `src/components/kumo/`.

The Kumo CSS import order is opinionated and matters. Per `kumo:ai`:

```css
@source "../node_modules/@cloudflare/kumo/dist";
@import "@cloudflare/kumo/styles";
@import "tailwindcss";
```

`@source` first (tells Tailwind to scan Kumo's compiled JS for class
names), then Kumo's tokens (so they register before Tailwind processes
utilities), then Tailwind. Putting tailwind first will silently break
some utility-class outputs.

### CSS architecture — cascade layers are the spine

Three things compete for paint precedence: legacy editorial styles,
Kumo's utility classes, and our hand-authored overrides. We use CSS
Cascade Layers in [styles.css][styles] to make the order explicit:

```
@layer legacy, theme, base, components, utilities, editorial;
```

- `legacy` — `legacy-styles.css` imported under `layer(legacy)`. Lowest
  precedence — any layered rule beats it. This is why our Kumo Button
  variants paint correctly without `!important` or specificity hacks.
- `theme` / `base` / `components` / `utilities` — Tailwind v4 + Kumo
  defaults.
- `editorial` — `theme-editorial-extras.css` + `layout-fixes.css`
  imported under `layer(editorial)`. Last layer = highest precedence,
  so selector overrides (sidebar reset, h1 Doto font, mobile fixes) win
  over utilities cleanly.

**Without the layer wrap, legacy is unlayered → trumps everything →
forces every override into a specificity war.** That's the trap I fell
into for hours before the user pointed it out. Don't repeat it.

The one place `!important` remains is the ThemeToggle hide on mobile.
That's a fight with inline `style={{ display: "inline-flex" }}` —
inline styles beat all layers, period. Documented in the file.

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

All tasks are **`noun:verb`** — no bare verbs at the top level.

- `mise:install` — bootstrap everything in `[tools]`
- `cargo:*` — every Rust workflow command (`check`, `build`, `build:release`,
  `test`, `lint`, `format`, `fix`, `watch`, `machete`, `expand`, `clean`,
  `pre-commit`)
- `cedar:*` — Cedar policy workflow (`validate`, `format`)
- `kumo:*` — frontend workflow (`web-install`, `web-init`, `web-dev`,
  `web-dev-http`, `web-build`, `web-reload`, `theme-gen`, `list-blocks`,
  `list-components`, `add-block`, `doc`)
- `seed:dev` — populate local D1 with alice/bob/carol/dave + Acme org +
  pending invitations via Connect RPC. Idempotent. Writes `.seed.json`
  with session tokens so you can switch user in DevTools:
  `localStorage.setItem('wm.session', JSON.stringify({token,whoami})); location.reload()`
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
