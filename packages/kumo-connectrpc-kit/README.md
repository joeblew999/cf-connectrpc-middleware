# @joeblew999/kumo-connectrpc-kit

Shared **Kumo + ConnectRPC chrome** — the reusable bits every ConnectRPC+Kumo
app needs, extracted once so projects (stripe-connectrpc, the multitenant
example, …) consume instead of copy.

Published to **JSR** (not npm). Consume with pnpm 10.9+ (native JSR support):

```sh
pnpm add @joeblew999/kumo-connectrpc-kit
```

## What's in it

- **`makeTransport(baseUrl)` / `setAuthToken(token)` / `errorMessage`** — a Connect
  transport that stamps `Authorization: Bearer <token>` (the Rauthy JWT), matching
  the server-side OIDC layer. Each app does its own `createClient(MyService, transport)`.
- **`AuthProvider` / `useAuth`** — the one shared session: a Rauthy JWT + `whoami`,
  persisted in localStorage, generic over the whoami type. The app passes its
  codegen'd `whoami()`; the kit owns storage/refresh/logout + token-stamping.
- **`AppShell`** — Kumo sidebar shell; nav + brand as props, sign-out via the shared
  `useAuth().logout()`.
- **`PageLoading`, `AuthHero`** — standalone Kumo chrome components.
- **`ThemeToggle`** — dev theme A/B tool; pass your own `themes` list.
- **`setHtmlTheme` / `readHtmlTheme` / `Theme`** — `<html data-theme>` helpers.

`react`, `react-dom`, `@cloudflare/kumo`, `@connectrpc/connect[-web]` are **peer
dependencies** (so your app's single React/Kumo is used — no duplicate-React
hook crash). Kumo's own UI blocks stay `kumo add` (Cloudflare registry).

## Publishing (maintainers)

```sh
mise run jsr:check     # dry-run: slow-types + packaging, no publish
mise run jsr:publish   # interactive: GitHub auth + the @joeblew999 JSR scope
```

## Server side

This kit is the *client* half of the one common way. The server half — verifying
the Rauthy JWT and enforcing Cedar policy on every ConnectRPC call — is the
`connectrpc-oidc` + `connectrpc-cedar` crates in this same repo (adopted via
Cargo). Rauthy issues the token; Cedar authorizes; this kit holds the session
and stamps the token on requests.
