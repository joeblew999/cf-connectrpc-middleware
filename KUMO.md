# Using `@cloudflare/kumo` without fighting it

A reusable rulebook for AI agents (and humans) building on Kumo. Earned
from a multi-day session where every shortcut around these rules cost
hours. Designed to be linked from any project's `CLAUDE.md`.

**How to use:** in your project's `CLAUDE.md`, add a line like
`Kumo discipline: see [KUMO.md][kumo-md].` and link to a checked-in
copy or to this file's permalink on GitHub.

---

## §0 — The one rule that beats every other rule

> **Run `kumo ai` BEFORE writing any Kumo code.** Not after, not
> while debugging. Before.

`kumo ai` (= `mise run kumo:ai` if mise-managed) prints the canonical
AI usage guide — every component's variants, compound subcomponent
API, common patterns. It's ~5KB of text that prevents 5 hours of
trial-and-error.

If you skip this, you will:
- Use wrong Banner variants (`info/warning/danger/success` — these
  don't exist; correct values are `default/alert/error`).
- Double-wrap inputs (`<Field label><Input/></Field>` — wrong; `Input`
  already auto-wraps in Field via its `label` prop).
- Miss compound APIs (`<Dialog.Trigger render={...}>` vs
  `<DialogTrigger asChild>` — the first is correct).

Cost of running it: one tool call. Cost of skipping it: every
component you write needs to be re-written after the user catches
the mistake.

---

## §1 — CLI commands you must use before reaching for grep

| Command | When |
| --- | --- |
| `kumo ai` | Before any Kumo-component-heavy work. Always. |
| `kumo doc <Name>` | Before using a component you haven't touched in this session. |
| `kumo docs` | Browse all 42 primitives when scoping a new page. |
| `kumo ls` | What's in `node_modules/@cloudflare/kumo` (component registry). |
| `kumo blocks` | Installable layout blocks (PageHeader, ResourceListPage, DeleteResource). |
| `kumo add <Block>` | Copy a block's source into `src/components/kumo/`. |
| `kumo migrate` | Token rename map when bumping Kumo versions. |

**Anti-pattern**: `grep -r SomeComponent node_modules/@cloudflare/kumo/`.
The minified chunks are unreadable. `kumo doc <Name>` gives you the
props table, variants, examples, and semantic tokens used — readable.

---

## §2 — CSS import order is opinionated, get it wrong and utilities silently break

Per `kumo ai`:

```css
@source "../node_modules/@cloudflare/kumo/dist";  /* 1st — scans Kumo's compiled JS for class names */
@import "@cloudflare/kumo/styles";                 /* 2nd — Kumo's @theme tokens register */
@import "tailwindcss";                             /* 3rd — Tailwind processes utilities */
```

Putting `tailwindcss` first silently breaks some utility-class outputs.
The `@source` line is the reason `bg-kumo-badge-orange` etc. actually
get emitted — Tailwind v4 only scans your source files by default, so
class names that live in Kumo's compiled bundle need the explicit
`@source` directive.

---

## §3 — Cascade-layer trap (this one will bite you)

```css
@layer theme, base, components, legacy, utilities, editorial;
```

Layer order is important. **But** Kumo also emits an **unlayered**
`:root, :host { --color-kumo-brand: light-dark(...) }` rule that sets
every `--color-kumo-*` token to a default value.

Per the CSS cascade-layers spec, **unlayered rules always beat any
layered rule, regardless of selector specificity**.

This means:
- A custom theme like `@layer base { [data-theme="mine"] { --color-kumo-brand: red } }`
  will **silently lose** to Kumo's unlayered blue default.
- Kumo's own `generateThemeOverrideCSS` wraps overrides in
  `@layer base { ... }`, so this bug exists upstream too. Set
  `<html data-theme="fedramp">` in any stock Kumo dev server and
  `--color-kumo-brand` still paints blue, not the fedramp value.

**Fix for custom themes:** emit `[data-theme="X"] { ... }` **unlayered**.
Then it ties Kumo's `:root, :host` on layer (both none) and wins on
specificity (`[data-theme=X]` = 0,1,0 > `:root` = 0,0,1) plus source
order.

---

## §4 — Kumo's theme generator is NOT a consumer API

Kumo's `scripts/theme-generator/generate-css.ts` has a clean
`generateThemeOverrideCSS(config, themeName)` function. Tempting.
You **cannot** call it from a consumer project:

- `node_modules/@cloudflare/kumo/dist/scripts/theme-generator/` ships
  `generate-css.d.ts` (types only) — **no `.js` runtime**.
- The package.json `exports` map only exposes
  `./scripts/theme-generator/{config,types}`. Deep imports past those
  fail under strict ESM.
- Kumo invokes the generator via `tsx scripts/theme-generator/index.ts`
  from inside their monorepo — a path that doesn't exist for consumers.

**Recommended path for custom themes:** hand-roll a small generator
that reads `THEME_CONFIG` (for validation) and emits CSS. Keep it
~100 lines, fully typed via JSDoc from
`@cloudflare/kumo/scripts/theme-generator/types`. See
[`scripts/theme-generator/generate.mjs`][example-gen] in
`example-multitenant-worker/web-kumo` for a working example.

Emit **unlayered** rules (see §3).

[example-gen]: https://github.com/joeblew999/example-multitenant-worker/blob/cedar/web-kumo/scripts/theme-generator/generate.mjs

---

## §5 — Form inputs already wrap themselves in Field

Wrong:

```tsx
<Field label="Email">
  <Input value={...} onChange={...} />
</Field>
```

Right:

```tsx
<Input
  label="Email"
  description="We'll never share your email"
  error={emailError}
  value={...}
  onChange={(e) => setEmail(e.target.value)}
/>
```

Inputs that take a `label` prop auto-construct the `<Field>` wrapper
internally. Same for `<Select>`, `<Combobox>`, `<SensitiveInput>`.

`<Field>` is only needed when you have a non-Kumo control to wrap, or
need `controlFirst={true}` (for checkbox/switch layouts).

---

## §6 — `<Text>` strips `className` when `variant` is set

Surprising but real. This errors at compile time:

```tsx
<Text variant="secondary" className="text-sm">…</Text>
// ❌ Property 'className' does not exist on type ...
```

Workaround: use a plain element with Kumo token classes for custom
styling.

```tsx
<span className="text-sm text-kumo-subtle">…</span>
```

Available token text classes: `text-kumo-default`, `text-kumo-subtle`,
`text-kumo-strong`, `text-kumo-danger`, `text-kumo-brand`. Use these
instead of raw Tailwind color classes for theme-aware text.

---

## §7 — Override the system as little as possible

Every line in your `chrome.css` / `theme-extras.css` / `layout-fixes.css`
is a fight with Kumo. The longer those files grow, the more brittle
your stack becomes (and the more cascade-layer bugs you'll hit).

Discipline:
1. Before writing CSS, check if Kumo has the component or pattern.
2. Before adding a selector override, check if the token can be set
   in the theme generator config.
3. Before adding a layer rule, draw out the cascade — which Kumo rule
   are you trying to beat? Is it layered or unlayered?

If you find yourself needing a third extras file, stop and rethink.

---

## §8 — Dev-server topology that works with agents

For headless screenshot tools (Claude preview, Playwright, etc.) HTTPS
self-signed certs are a pain. The pattern that works:

| URL                          | Server   | When to use                                  |
| ---------------------------- | -------- | -------------------------------------------- |
| `https://localhost:5173/`    | Vite     | Real Chrome iteration. HMR is instant.       |
| `http://localhost:5175/`     | Vite     | Headless tools. HTTP-only, no cert dance.    |
| `https://localhost:8787/`    | Wrangler | Production-parity wasm path (CF Workers).    |

The HTTP variant uses a separate `vite.config.http.ts` with `https:false`.
Launched on demand (not supervised) — agents start it via
`mise run kumo:web-dev-http`.

---

## §9 — Theme persistence: scope it, don't globalize it

Anti-pattern: a global ThemeToggle that writes to `localStorage["app.theme"]`
and reads it at boot. Two failure modes:

1. **Race condition with agents**: when the user and an AI agent both
   have dev tabs open, one writes a theme, the other's next page-load
   silently inherits it. Visual confusion.
2. **Bleeds across showcase intent**: the ThemeToggle is usually a dev
   affordance for verifying a Preview/Storybook-like page renders
   under each theme. It's not a user preference.

**Better pattern:**
- Declare the canonical theme statically in `index.html`:
  `<html data-theme="editorial">`.
- The ThemeToggle component owns ephemeral state via `useState`, calls
  `setHtmlTheme(t)` on change, and a `useEffect` cleanup resets back
  to the canonical theme on unmount.
- No `localStorage`, no URL params, no boot-time JS.

Result: navigating away always lands you back in the canonical theme.
The toggle is purely a per-page A/B tool.

---

## §10 — Known bugs in Kumo upstream (file these if not already filed)

1. **Cascade-layer bug** (§3): `generateThemeOverrideCSS` wraps in
   `@layer base`, but Kumo's unlayered `:root, :host` defaults win.
   Repro: stock Kumo dev server, `<html data-theme="fedramp">`,
   inspect `--color-kumo-brand` — still Kumo blue.

2. **Generator not exposed in `exports` map** (§4): `generate-css.js`
   doesn't ship; only the `.d.ts` does. Consumers can read `THEME_CONFIG`
   but can't call the generator. Worth a request to either ship the
   runtime or document hand-rolling.

3. **`<Text>` strips `className` when `variant` is set** (§6): probably
   intentional (variant owns styling) but the error message is
   inscrutable. At minimum, the type error should explain this.

4. **Kumo `<Button>` has no `data-` attribute consumers can target**: if
   you need theme-specific styling on Kumo Buttons (mono+caps in our
   editorial palette, say), the only stable selector today is
   `button.shrink-0` — which matches Kumo Buttons because Kumo applies
   `shrink-0` to every Button, but also matches any other element with
   that Tailwind class. A `data-kumo-button` would let consumers target
   buttons precisely.

---

## §11 — Bundle size: Kumo is heavy, plan for code-splitting

Out of the box, the Kumo bundle is ~830 KB minified (~250 KB gzip)
because every component you import pulls its Base UI primitive deps.
Vite's default 500 KB chunk warning fires on the production build.

Mitigations to consider once your app grows:
- Route-level dynamic imports (`React.lazy(() => import("./pages/X"))`).
- `manualChunks` in `vite.config.ts` to split Kumo into its own chunk
  (cached separately, so app updates don't invalidate it).
- For showcase / preview-only components (DateRangePicker, Chart,
  CommandPalette, etc.), keep them out of the main route bundle —
  lazy-load on the Preview page.

Not a Kumo bug, just a heads-up: if your app stays small, you don't
need to act. If you start shipping pages with heavy primitives the
user doesn't hit on every visit, code-split.

---

## Quick checklist before authoring a Kumo page

- [ ] Ran `kumo ai` for this session
- [ ] Ran `kumo doc <Component>` for each component you're about to use
- [ ] CSS import order: `@source` → `@cloudflare/kumo/styles` → `tailwindcss` → yours
- [ ] No layered `@layer base` wrapper on theme overrides for tokens
      Kumo's `:root, :host` defaults set (i.e. any `--color-kumo-*`)
- [ ] Inputs use `label="..."` instead of being wrapped in `<Field>`
- [ ] No `className` on `<Text variant="...">`
- [ ] Extras/overrides files haven't grown — if they have, ask why
