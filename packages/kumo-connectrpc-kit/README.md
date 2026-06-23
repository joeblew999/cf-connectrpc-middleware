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
  transport that stamps `Authorization: Bearer <token>`, matching a server-side
  bearer/token auth layer. Each app does its own `createClient(MyService, transport)`.
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

## Roadmap

`AppShell` + the auth context need decoupling (nav/auth injected as props, not
imported) before they move in — tracked as the next extraction.
