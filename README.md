# connectrpc-cedar

## Live demo

A Worker deployed to Cloudflare with the multitenant scaffold + editorial
Kumo theme is running here:

| Surface | URL | Use for |
| --- | --- | --- |
| **Login**       | https://workers-multitenant.gedw99.workers.dev/login   | Tester landing page |
| **Preview / dev-debug** | https://workers-multitenant.gedw99.workers.dev/preview | One-click sign-in cards + theme switcher + full Kumo component showcase |
| **Dashboard** (post-login) | https://workers-multitenant.gedw99.workers.dev/        | Authed app |

**Recommended tester URL: `/preview`** — it has color-coded "Sign in"
buttons for each of the 4 demo accounts (no copy-paste creds needed).
After sign-out, you're bounced straight back to `/preview` so the next
test login is one tap away.

Pre-seeded test accounts (all password `demo-password-123`).
All emails use the `.example` TLD per RFC 6761 — guaranteed never
to collide with real users, so the seed is safe in production:

| Email                  | What they have                                            |
| ---------------------- | --------------------------------------------------------- |
| alice@acme.example     | Owns Acme + 5 other orgs (scope-switcher density demo)    |
| bob@acme.example       | Member of Acme + Engineering, owner of Marketing          |
| carol@partner.example  | 5 pending invites across different orgs (volume demo)     |
| dave@late.example      | Pending billing invite from alice                         |

The theme switcher on `/preview` flips between the **editorial** theme
(dark, red accent — what we're building) and the default **kumo** theme
(Cloudflare orange) for A/B comparison.

### Running locally

| Surface | URL | When |
| --- | --- | --- |
| Vite (HMR, HTTPS, self-signed) | https://localhost:5173 | Real browser dev (`mise run dev:up`) |
| Vite (HTTP, no cert) | http://localhost:5175 | Headless screenshot tools (`mise run kumo:web-dev-http`) |
| Wrangler dev | https://localhost:8787 | Production-like wasm path (`mise run worker:dev`) |

To redeploy or tear down, see `mise tasks | grep -E "cf:|worker:|seed:"`.

To hide demo accounts on a real-project deploy: set
`VITE_SHOW_TEST_ACCOUNTS=false` in `.env.production` — the DevAccounts
card hides AND sign-out reverts to `/login` (instead of `/preview`).

## Intent

Rust based.

what we need is a connect rpc middleware to allow us to easily use cedar for authourisation.

this crate must work well with the examples below.

We need an example that uses kumo.

## Connect rpc on cloudflare

The following have already been invented to make this as easy as possible.

https://github.com/connyay/connectrpc-workers is a cloudflare workers implementation of connect rpc, allowing code gen for rpc, with clents in react.

https://github.com/connyay/example-multitenant-worker is an example thats multi tenant and so lends it self to Cedar and importantly has a React GUI using connect rpc also.

https://github.com/connyay/example-connectrpc-worker is a set of simple examples.

## cedar 

https://github.com/cedar-policy/cedar has a browser wasm and so can also run on cloudflare too as wasm.

**Why Cedar here:**

- **Multi-tenant authz is a ReBAC problem.** Rules like "an owner of the billing account can also act on its orgs" tangle quickly when hand-rolled. Cedar expresses them as ~5-line policies.
- **Policies live separately from code.** `.cedar` files version alongside the Rust source, type-checked against a schema by `cedar validate` — typos and dangling actions fail at lint time, not in production.
- **Wasm-native, microsecond decisions.** Cedar's evaluator runs inside the Worker. No external service, no extra round trip.
- **Composes cleanly with the macaroon session.** The example's `AuthLayer` already pins (billing, org, role) on the verified session at request-issue time. Our `CedarLayer` reads that pinned scope and passes it through Cedar's `context` — no DB lookup at authorization time. Macaroon attenuates *what scope a token covers*; Cedar evaluates *whether the action is allowed at that scope*. Two layers, each doing what it does best. See [examples/multitenant-policies/](examples/multitenant-policies/README.md).




## tooling

we use mise in the root to mange all dependencies and nushell for scripting.

cedar cli might be useful too. 

https://github.com/cedar-policy/cedar-for-agents might be useful too. 

## GUI

React on top of [Kumo](https://kumo-ui.com/) primitives. The visual target
is the editorial dark-mode look already shipped in
`example-multitenant-worker/web/` (red `#d71921` accent, Space Grotesk /
Doto type). Kumo's Cloudflare-Orange default is overridden via
[web-kumo/src/kumo-theme.css](.src/example-multitenant-worker/web-kumo/src/kumo-theme.css)
so components inherit the same tokens as the `web/` baseline.

- Install: https://kumo-ui.com/installation/
- CLI (for adding blocks): https://kumo-ui.com/cli
- Registry: https://kumo-ui.com/registry/












