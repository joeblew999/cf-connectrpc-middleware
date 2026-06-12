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
