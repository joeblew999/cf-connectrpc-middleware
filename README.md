# connectrpc-cedar

## Live demo

A Worker deployed to Cloudflare with the multitenant scaffold + editorial
Kumo theme is running at:

**https://workers-multitenant.gedw99.workers.dev**

Pre-seeded test accounts (all password `demo-password-123`):

| Email              | What they have                                        |
| ------------------ | ----------------------------------------------------- |
| alice@acme.io      | Owns the "Acme" org; sent the pending invites below   |
| bob@acme.io        | Member of Acme (accepted invite)                      |
| carol@partner.dev  | Pending org invite to Acme (not yet accepted)         |
| dave@late.io       | Pending billing invite from alice                     |

Sign in at `/login` as any of them. The toggle in the top-right switches
between the **editorial** theme (dark, red accent — what we're building)
and the default **kumo** theme (Cloudflare orange) for A/B comparison.

To redeploy or tear down, see `mise tasks | grep -E "cf:|worker:|seed:"`.

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












