# rauthy-cedar

The **AuthN → AuthZ seam**, end to end: a [Rauthy](https://github.com/sebadob/rauthy)
JWT validated by [`connectrpc-oidc`](../../crates/connectrpc-oidc) → a
`Session` in request extensions → [`connectrpc-cedar`](../../crates/connectrpc-cedar)
maps it to a Cedar principal and authorizes.

```
request ─► OidcLayer ──────────► CedarLayer ─────────► handler
           verify JWT vs Rauthy   read Session,         business
           JWKS, insert Session    eval policies         logic
```

**Purpose:** prove the claim shape lines up. Rauthy emits `sub`, `roles`,
`groups`, `scope`; this example shows each one driving a policy:

| Claim | Drives | Policy |
| --- | --- | --- |
| `roles` ∋ `admin` | RBAC override | `admin-full-access` |
| `groups` ∋ doc's group | resource scoping | `group-members-can-read` |
| `scope` ∋ `write` | OAuth-scope gate | `group-members-with-write-scope-can-write` |

This is the small, schema-clean counterpart to `remy-sport-policies/`
(the realistic 186-row ReBAC matrix). Same `nu` test harness.

## Run the policy proof

```sh
nu tests/run.nu        # or: mise run cedar:test (once wired)
```

Expected: 7/7 pass — admin override, group read, write-scope gating, and the
three denials (no write scope, non-admin delete, wrong group).

## Run it & check the output

### A. One-command full chain (real Rauthy token → ALLOW/DENY)

`e2e.nu` is the whole thing against the actual IdP, not a self-signed stand-in:
it boots a throwaway Rauthy, bootstraps an OIDC client, mints a real **user**
token (password grant), then runs `demo/` which does AuthN (`connectrpc-oidc`)
→ AuthZ (`connectrpc-cedar`):

```sh
nu examples/rauthy-cedar/e2e.nu          # needs a local Docker daemon
```
```
AuthN ✓  sub=…  roles=["rauthy_admin", "admin"]  scopes=["openid","profile","groups"]
AuthZ  action=read   → ALLOW   (any authenticated user)
AuthZ  action=admin  → ALLOW   (token carries the admin role)
DEMO OK — real Rauthy token verified AND authorized by Cedar.
```
The two demo policies live in [`demo/policies/`](demo/policies): `read` is
allowed for any authenticated user; `admin` only when the token's `roles`
contains `admin`. Strip the admin role off a user in the GUI (below) and
`admin` flips to DENY — that's the output to check.

### B. Use the Rauthy GUI

To poke the IdP yourself, run the standing instance (vm-uncloud recipe):

```sh
mise run recipe:local rauthy             # in the vm-uncloud repo
```

| What | URL / value |
| --- | --- |
| **Rauthy admin GUI** | http://localhost:8080/auth/v1/admin |
| Login | `admin@localhost` / `LocalDevAdminPassword123456` |
| OIDC discovery | http://localhost:8080/auth/v1/.well-known/openid-configuration |
| JWKS | http://localhost:8080/auth/v1/oidc/certs |

In the GUI you can add users, assign/remove **roles** and **groups**, and
register OIDC clients. Those roles/groups are exactly what flow into the token →
`Session` → Cedar `context`, so changing them in the GUI changes the AuthZ
decision the demo prints.

### What the harness bakes in (hard-won)
- The distroless Rauthy image **panics without `/app/config.toml`** (seeded via a
  shared volume / bind-mount).
- A bootstrap client secret must be **exactly 64 chars** — validation uses
  `constant_time_eq_64`; bootstrap only checks `>=64`, so a longer one stores but
  can never match (`"Invalid 'client_secret'"`).
- `client_credentials` tokens have `sub: null` → use the **password grant** for a
  user token with `sub` + roles.

> Note on the CF Worker packaging: `demo/` runs the **same two crates**
> (`connectrpc-oidc` + `connectrpc-cedar`) a Worker wires, as a native binary so
> the decision is easy to watch. Packaging it as a deployable `wrangler dev`
> Worker (in-Worker async JWKS fetch via the `worker-jwks` feature) is the
> remaining step.

## The two planes

This example is the **edge plane**. The **server plane** — Rauthy itself —
runs from `vm-uncloud/recipes/rauthy/`. The contract between them is three
values: `RAUTHY_ISSUER`, `RAUTHY_JWKS_URL`, and the OIDC `client_id`. The
example Worker (TODO, step 3) reads those, fetches JWKS at boot, and chains
`OidcLayer` → `CedarLayer` over a `connectrpc-workers` handler.

## Mapping reference (connectrpc-oidc → this schema)

```
JWT.sub     → User::"<sub>"
JWT.roles   → principal.roles   (Set<String>)
JWT.groups  → principal.groups  (Set<String>)
JWT.scope   → context.scopes    (Set<String>, space-split)
```

See [`connectrpc-oidc/src/claims.rs`](../../crates/connectrpc-oidc/src/claims.rs)
for the `Claims` → `Session` conversion.
