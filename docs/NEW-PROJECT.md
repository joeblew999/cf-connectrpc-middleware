# Add Rauthy + Cedar auth to a new project

The auth stack is shared: **one Rauthy** is the SSO IdP for all projects; each
project gets its **own OIDC client** + its **own Cedar policies**. You don't
redeploy Rauthy — you register a client and add the middleware.

## 1. Register the project's OIDC client (declarative)

In **vm-uncloud** → `recipes/rauthy/bootstrap.nuon`, add a client:

```nuon
clients: [
  { id: "my-project", name: "My Project",
    flows: ["authorization_code", "client_credentials", "password"],
    scopes: ["openid", "profile", "groups", "email"],
    redirect_uris: ["https://my-project.example/oauth/callback"] }
]
```

`prepare.nu` generates a stable 64-char secret (persisted in the fnox keychain
as `VMU_RAUTHY_CLIENT_my-project`, printed once) and seeds it on the next
`mise run recipe rauthy` / `recipe:local rauthy`. **You never hand-write a
secret** (and never hit Rauthy's exactly-64-char validation by hand).

## 2. Add the middleware to the project's Worker/service

Depend on the two crates from **cf-connectrpc-middleware**:

```toml
connectrpc-oidc  = { git = "https://github.com/joeblew999/cf-connectrpc-middleware" }
connectrpc-cedar = { git = "https://github.com/joeblew999/cf-connectrpc-middleware" }
```

Wire them (same on native and CF — see `examples/rauthy-cedar/app/`):

```rust
let svc = OidcLayer::new(verifier)          // verify the Rauthy JWT
    .skip_paths(["/healthz"])
    .layer(CedarLayer::enforce(authz, extract) // authorize with your policies
    .layer(your_service));
```

- **Verifier**: fetch JWKS from `https://id.<domain>/auth/v1/oidc/certs` at boot.
  Issuer is `https://id.<domain>/auth/v1/` (note the path), aud = your `client_id`.
- **JWKS fetch**: `ureq` (native) or `connectrpc_oidc::fetch::fetch_jwks` +
  the `worker-jwks` feature (CF).

## 3. Write the project's Cedar policies

Roles/groups from the Rauthy token arrive in `context` (dynamic principal).
Starter (copy `examples/rauthy-cedar/app/policies/` and edit):

```cedar
// any authenticated user
permit (principal is User, action == Action::"read", resource is Api);
// gated on a role from the token
permit (principal is User, action == Action::"admin", resource is Api)
when { context.roles.contains("admin") };
```

The RPC path maps to a Cedar action automatically via `action_from_path`
(`/pkg.v1.Svc/Method` → `Action::"pkg.v1.Svc.Method"`).

## 4. (If the project sends auth email) — nothing to do

Rauthy's transactional email already routes through the shared bridge →
saasmail → Cloudflare Email. Set the shared `RAUTHY_WEBHOOK_SECRET` once
(see `deploy-stack.nuon`).

## 5. Deploy

Add the project's Worker to `deploy-stack.nuon` (its repo path + deploy command)
and run `mise run deploy:stack`.
