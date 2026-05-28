# connectrpc-cedar

A Rust `tower::Layer` that adds Cedar policy authorization to ConnectRPC
handlers on Cloudflare Workers. Designed to drop into existing
`connectrpc-workers`-based Workers with 2 lines of glue code.

The crate itself is the goal. The deployed Worker + React frontend
under `.src/example-multitenant-worker/` exists as the reference shape
the layer must compose with cleanly.

## Live demo

Deployed at https://workers-multitenant.gedw99.workers.dev/

The deploy serves one **demo scenario** at a time. A scenario is the
theme + the demo accounts + the seed data it expects, picked at build
time via `VITE_SEED_SCENARIO`. Two scenarios exist today:

| Scenario | Brand | Demo accounts | Domain |
| --- | --- | --- | --- |
| `editorial` | Dark, red accent, Doto pixel display | alice / bob / carol / dave | Acme Corp multitenancy |
| `remysport` | Paper bg, orange accent, Inter | coach / captain / scout / manager | Bangkok Suns basketball club |

| Surface | URL | What's there |
| --- | --- | --- |
| **Login** | `/login` | Credential form + one-click sign-in cards for the active scenario's demo accounts |
| **Preview** | `/preview` | Kumo component showcase + theme A/B toggle (editorial / remysport / kumo / fedramp) |
| **Dashboard** (post-login) | `/` | Authed app — scope switcher, member tables, billing, invitations |

**Tester URL: `/login`.** Color-coded sign-in cards, no copy-paste creds
needed. Shared password is `demo-password-123`. After sign-out you're
bounced back to `/preview` (because dev accounts are still enabled).

All seed emails use the `.example` TLD per RFC 6761 — guaranteed never
to collide with real users, so the seed is safe in production.

To check what scenario is currently live:

```
curl -s https://workers-multitenant.gedw99.workers.dev/ | grep data-theme
```

To flip the deploy between scenarios:

```
mise run worker:deploy            && mise run seed:prod            # editorial
mise run worker:deploy:remysport  && mise run seed:prod:remysport  # remysport
```

The deploy + seed pair MUST match — see [SEED.md §7](./SEED.md).

### Running locally

| Surface | URL | When |
| --- | --- | --- |
| Vite (HMR, HTTPS, self-signed) | https://localhost:5173 | Real browser dev (`mise run dev:up`) |
| Vite (HTTP, no cert) | http://localhost:5175 | Headless screenshot tools (`mise run kumo:web-dev-http`) |
| Wrangler dev | https://localhost:8787 | Production-like wasm path (`mise run worker:dev`) |

To switch the local dev server scenario, set the env at startup:

```
VITE_SEED_SCENARIO=remysport pnpm dev          # in .src/example-multitenant-worker/web-kumo/
SCENARIO=remysport mise run seed:dev           # seeds local D1 with matching users
```

For all `mise run …` tasks, see `mise tasks`.

To hide demo accounts on a real-project deploy: set
`VITE_SHOW_TEST_ACCOUNTS=false` in `.env.production`. The DevAccounts
panel hides and sign-out reverts to `/login` (instead of `/preview`).

## Intent

A `tower::Layer` that runs Cedar policy evaluation on every Connect
RPC call. The layer reads a `SessionContext` from the request
extensions (populated by an upstream `AuthLayer`), maps the URL path
`/pkg.Service/Method` to a Cedar `Action`, calls
`cedar_policy::Authorizer::is_authorized`, and short-circuits with
`permission_denied` on `Decision::Deny`.

The repo proves it composes with these existing pieces:

- [`connyay/connectrpc-workers`](https://github.com/connyay/connectrpc-workers)
  — Cloudflare Workers implementation of Connect RPC, with codegen and
  React clients.
- [`connyay/example-multitenant-worker`](https://github.com/connyay/example-multitenant-worker)
  — multitenant scaffold (billing accounts + orgs + invitations);
  cloned under `.src/` and forked to a `cedar` branch.
- [`connyay/example-connectrpc-worker`](https://github.com/connyay/example-connectrpc-worker)
  — minimal RPC scaffold for the next-test case.

## Why Cedar

- **Multi-tenant authz is a ReBAC problem.** Rules like "an owner of
  the billing account can act on its orgs" tangle quickly when
  hand-rolled. Cedar expresses them as ~5-line policies.
- **Policies live separately from code.** `.cedar` files version
  alongside the Rust source, type-checked against a schema by
  `cedar validate` — typos and dangling actions fail at lint time,
  not in production.
- **Wasm-native, microsecond decisions.** Cedar's evaluator runs
  inside the Worker. No external service, no extra round trip.
- **Composes cleanly with the macaroon session.** The example's
  `AuthLayer` already pins (billing, org, role) on the verified session
  at request-issue time. Our `CedarLayer` reads that pinned scope and
  passes it through Cedar's `context` — no DB lookup at authorization
  time. Macaroon attenuates *what scope a token covers*; Cedar evaluates
  *whether the action is allowed at that scope*. Two layers, each doing
  what it does best. See [examples/multitenant-policies/](examples/multitenant-policies/README.md).

Reference: [Cedar policy language](https://github.com/cedar-policy/cedar)
has a browser wasm and runs on Cloudflare too.
[cedar-for-agents](https://github.com/cedar-policy/cedar-for-agents)
may be useful for agent-driven policy authoring.

## Frontend: React + Kumo

`.src/example-multitenant-worker/web-kumo/` is a React app built on
[Kumo](https://kumo-ui.com/) primitives. Every scenario declares its
own theme in `scenarios/<name>/scenario.mjs`; the CSS generator
(`mise run kumo:theme-gen`) emits per-scenario palette + Kumo-token
mapping files into `src/styles/`.

The whole demo (theme + DevAccounts + backend seed) is consolidated
into one file per scenario — auto-discovered by both Vite and the
seed scripts. Add a new scenario: `mkdir scenarios/<name>` + write
one `.mjs` file. See [SEED.md](./SEED.md) for the contract.

The original `web/` frontend lives alongside `web-kumo/` and is the
upstream baseline (red `#d71921` accent, Space Grotesk / Doto type).
The `web-kumo/` editorial scenario reproduces that look on Kumo
primitives.

## Tooling

- **mise** in the root manages all dependencies (rust, node, wrangler,
  fnox, cedar CLI) and orchestrates tasks. `mise tasks` lists them.
- **nushell** for all task scripting (`shell = "nu -c"` in `mise.toml`).
- **fnox** + macOS keychain for secrets (Cloudflare API token, macaroon
  root key, deployed Worker URL). Per-repo `fnox.toml` is the contract;
  values live once in the keychain.

## Docs

- [CLAUDE.md](./CLAUDE.md) — instructions to the AI agent driving this
  repo. Pinned versions, module layout, mise task namespaces, the
  `.src/` fork workspace.
- [KUMO.md](./KUMO.md) — cross-project rulebook for using
  `@cloudflare/kumo` without fighting it. Cascade-layer traps, theme
  generator gotchas, the `kumo ai` discipline.
- [SEED.md](./SEED.md) — cross-project rulebook for demo-scenario
  seeding. One folder per scenario, auto-discovery, the deploy↔seed
  pairing contract.
