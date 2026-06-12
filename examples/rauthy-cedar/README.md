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

## One app, two hosts (native + Cloudflare)

The whole point: **one codebase, two runtimes.** All the logic — the
`OidcLayer → CedarLayer` stack, the policies, the `Session→Cedar` extractor —
lives in **[`app/`](app)** and is shared verbatim. The two hosts are thin:

```
app/                shared: policies + extractor + the make() stack   (rlib, builds native AND wasm)
├── server/         NATIVE host  — hyper  + ureq JWKS         → calls app::make()
└── worker/         CF WORKER    — event  + worker::Fetch JWKS → calls app::make()
```

Both hosts are ~40 lines and do only the two things that differ by platform:
fetch the JWKS, and run the serve loop. The `app::make(verifier)` call that
builds the entire middleware stack is **identical** in both. (`grep -n make`
in `server/src/main.rs` and `worker/src/lib.rs` to see it.)

### Native (hyper) — `serve.nu` boots Rauthy, mints a token, asserts:

```sh
nu examples/rauthy-cedar/server/serve.nu
```
```
  ✓ healthz (no token)            [200]   skip path
  ✓ Read no-token → AuthN deny    [401]   OidcLayer rejects (no token)
  ✓ Read token → allow            [200]   verified + Cedar allow
  ✓ Admin admin-role → allow      [200]   token carries `admin`
  ✓ Super no-superuser → deny     [403]   Cedar permission_denied
==> SERVER E2E OK
```

### Cloudflare Worker (`wrangler dev`) — the SAME `app`, on the edge:

```sh
# worker/wrangler.toml [vars] point at a running Rauthy (e.g. :8088)
cd examples/rauthy-cedar/worker && wrangler dev          # → http://127.0.0.1:8787

curl -s -H "Authorization: Bearer $TOKEN" -X POST 127.0.0.1:8787/demo.v1.Api/Read
# {"status":"ok","authorized":"sub=KpT3… roles=[\"rauthy_admin\",\"admin\"]"}   200
curl -s -H "Authorization: Bearer $TOKEN" -X POST 127.0.0.1:8787/demo.v1.Api/Super
# {"code":"permission_denied","message":"cedar denied"}                          403
```

**Verified on both** against a real Rauthy token (no-token 401 · Read/Admin 200 ·
Super 403 — identical native and on `wrangler dev`/miniflare). The RPC path maps
to a Cedar action automatically (`/demo.v1.Api/Read` → `Action::"demo.v1.Api.Read"`,
via `action_from_path`), so the policies in [`app/policies/`](app/policies)
address routes by their proto path.

### Use the Rauthy GUI to drive the decision

```sh
mise run recipe:local rauthy             # in the vm-uncloud repo
```

| What | URL / value |
| --- | --- |
| **Rauthy admin GUI** | http://localhost:8080/auth/v1/admin |
| Login | `admin@localhost` / `LocalDevAdminPassword123456` |
| OIDC discovery | http://localhost:8080/auth/v1/.well-known/openid-configuration |
| JWKS | http://localhost:8080/auth/v1/oidc/certs |

Add a user in the GUI, give/remove the `admin` role — those roles flow into the
token → `Session` → Cedar `context`, so `Admin` flips between 200 and 403. That's
the output to check.

### Hard-won Rauthy details (baked into the harnesses)
- The distroless Rauthy image **panics without `/app/config.toml`** (seed it).
- A bootstrap client secret must be **exactly 64 chars** — validation uses
  `constant_time_eq_64`; bootstrap only checks `>=64`, so a longer one stores but
  can never match (`"Invalid 'client_secret'"`). Rauthy [PR #1599](https://github.com/sebadob/rauthy/pull/1599)
  adds **generated** bootstrap secrets + `rauthy bootstrap get` — the cleaner,
  reusable path (let Rauthy generate the secret, extract it) once it lands in a release.
- `client_credentials` tokens have `sub: null` → use the **password grant** for a
  user token with `sub` + roles.

## The two planes

The **edge/native plane** is this `app` (run as `server/` or `worker/`). The
**server plane** — Rauthy itself — runs from `vm-uncloud/recipes/rauthy/`. The
contract between them is three values: `RAUTHY_ISSUER`, `RAUTHY_JWKS_URL`, and
the OIDC `client_id`/secret (bootstrapped declaratively via a `bootstrap/` dir,
the same way Cedar policies declare authz).

## Mapping reference (connectrpc-oidc → this schema)

```
JWT.sub     → User::"<sub>"
JWT.roles   → principal.roles   (Set<String>)
JWT.groups  → principal.groups  (Set<String>)
JWT.scope   → context.scopes    (Set<String>, space-split)
```

See [`connectrpc-oidc/src/claims.rs`](../../crates/connectrpc-oidc/src/claims.rs)
for the `Claims` → `Session` conversion.
