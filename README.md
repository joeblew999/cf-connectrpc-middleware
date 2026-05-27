# connectrpc-cedar

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

React using Kumo. Love that Orange look !!

Easy to include https://kumo-ui.com/installation/

worth installing the cli : https://kumo-ui.com/cli ?

Colour referecnes: https://kumo-ui.com/colors/ so we can get the Orange look.

Maybe the Registry is useful ? https://kumo-ui.com/registry/












