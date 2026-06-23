# iam — Rauthy + Cedar auth, reusable across every project

The standard auth stack: **[Rauthy](https://github.com/sebadob/rauthy)** (OpenID
Connect IdP — *AuthN*) + **[Cedar](https://www.cedarpolicy.com/)** (policy
authorization — *AuthZ*), with transactional email via Cloudflare. Rauthy and
Cedar both run **unmodified**; everything here is the glue + the deploy.

This repo is the **front door** — the map, the cross-repo deploy orchestrator,
and the "add auth to a new project" guide. The building blocks live in their own
repos (they're reused as-is):

| Repo | Plane | What it does |
| --- | --- | --- |
| **[vm-uncloud](https://github.com/joeblew999/vm-uncloud)** | AuthN (deploy) | runs **Rauthy** (the IdP) + the **email bridge**, as a recipe (`recipes/rauthy/`). Declarative clients/roles in `bootstrap.nuon`. |
| **[cf-connectrpc-middleware](https://github.com/joeblew999/cf-connectrpc-middleware)** | AuthZ | the **`connectrpc-oidc`** (verify a Rauthy JWT) + **`connectrpc-cedar`** (authorize) crates, plus one shared app that runs **native AND on Cloudflare**. |
| **[saasmail](https://github.com/joeblew999/saasmail)** | email | CF email worker; **`/api/rauthy-inbound`** sends Rauthy's mail via Cloudflare Email. |

## How it fits together

```
 Browser ─► Rauthy ──JWT──► your Worker:  OidcLayer (verify) → CedarLayer (authorize) → handler
           (vm-uncloud)                    └────────── cf-connectrpc-middleware ──────────┘

 Rauthy ──SMTP──► bridge ──webhook──► saasmail /api/rauthy-inbound ──► Cloudflare Email
         (vm-uncloud, smtp2http)
```

- **AuthN** = Rauthy issues JWTs. **AuthZ** = Cedar evaluates policies against the
  token's claims. **Email** = Rauthy's SMTP is bridged to CF Email.
- The seam is one `Session` struct (sub, roles, groups, scopes) the OIDC layer
  inserts and the Cedar layer reads — the same crates run native and on the edge.

## Try it — the whole stack, locally, no spend

One command brings up **everything** (Rauthy + email bridge on Docker, the
oidc→cedar service), drives a **real token** through every hop, prints green, and
leaves Rauthy's GUI running so you can poke it:

```sh
mise trust                  # first run only (new repo)
mise run stack:local        # needs Docker + ../vm-uncloud + ../cf-connectrpc-middleware
```
```
1/4 · Rauthy + email bridge (Docker) ...        ✓ up + worker-client bootstrapped
2/4 · minting a real user token ...             ✓
3/4 · oidc→cedar service (the shared app) ...
4/4 · driving requests through oidc → cedar ...
      ✓ healthz (no token)            [200]
      ✓ Read no-token → AuthN deny    [401]
      ✓ Read token → allow            [200]
      ✓ Admin admin-role → allow      [200]
      ✓ Super no-superuser → deny     [403]
══ stack is UP and GREEN — poke it ══
  Rauthy GUI : http://localhost:8080/auth/v1/admin   (admin@localhost / LocalDevAdminPassword123456)
mise run stack:local --down  # tear it all down
```

Change a user's roles in the Rauthy GUI, re-run, and watch the AuthZ decision
flip. The point: **see it work on your laptop before spending a cent on deploy.**
The same shared app runs on the edge too — `cd ../cf-connectrpc-middleware/examples/rauthy-cedar/worker && wrangler dev`.

## Deploy

```sh
mise run deploy:stack            # DRY — prints the whole ordered plan, no spend
mise run deploy:stack -- --execute
```

The map is **[`deploy-stack.nuon`](deploy-stack.nuon)** — every target, its repo,
its command, the shared values. To add auth to a new project, see
**[docs/NEW-PROJECT.md](docs/NEW-PROJECT.md)**.

## Why this repo exists

The auth stack is an **assembly** across repos that are each about something else
(deploy, middleware, email). So cross-cutting "how it all fits" had no home — and
this is it. The org map at [github.com/joeblew999](https://github.com/joeblew999)
points here; the detail lives in each component's own README.
