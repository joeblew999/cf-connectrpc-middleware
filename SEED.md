# Demo-scenario seeding for multitenant SaaS apps

Companion to [KUMO.md](./KUMO.md). Designed for reuse: a project's
`CLAUDE.md` can link to this file with a one-liner like
"Demo scenarios: see [SEED.md][seed-md]."

A **demo scenario** is one coherent demo of your app: a theme, a set
of sign-in cards, and a backend seed script that materializes the
users + orgs + invites the sign-in cards reference. Switching demos
(editorial → remysport → next-thing) should be **one env var**, and
adding a new scenario should be **one folder**.

## §0 — The one rule

> **One folder per scenario. One file inside it owns everything for
> that scenario.**

```
scenarios/
├── editorial/
│   └── scenario.mjs       # SCENARIO_NAME, THEME, ACCOUNTS, PASSWORD, seed()
└── remysport/
    └── scenario.mjs
```

The folder name = the value used by `<html data-theme>` = the value
of `SCENARIO=` for the backend seed = the value of
`VITE_SEED_SCENARIO` for the frontend build. Keep all four in lockstep
by giving them the same name. The validator in
`scripts/theme-generator/generate.mjs` fails the build if folder name
≠ `SCENARIO_NAME` inside the file.

The single env var `VITE_SEED_SCENARIO` controls **all three concerns
simultaneously**:
- HTML element gets `<html data-theme="$VITE_SEED_SCENARIO">` via Vite's
  `%VITE_SEED_SCENARIO%` substitution.
- Frontend `ACTIVE_SCENARIO` resolves to the matching scenario module
  (via `import.meta.glob`).
- Backend seed reads the same value (as `SCENARIO=`) to pick which
  scenario.mjs to run.

---

## §1 — Anatomy of a scenario.mjs

Six exports. Five data, one function. Names are ALL_CAPS to match the
`.mjs` convention and make the consumer side explicit.

```js
// scenarios/<name>/scenario.mjs

export const SCENARIO_NAME = "<name>";          // must equal folder name
export const DESCRIPTION   = "one-liner shown in the seed log";
export const PASSWORD      = "demo-password-123";

// === THEME — consumed by scripts/theme-generator/engine.mjs ===
export const THEME = {
  colorScheme: "dark" | "light",
  colorSchemeLight: "dark" | "light" | undefined,
  palette: {
    base: { /* --accent, --surface, --text-primary, etc. */ },
    light: { /* sparse override for prefers-color-scheme: light */ },
  },
  fonts: { "font-display": "...", "font-body": "...", "font-mono": "..." },
  scale: { /* type sizes, spacing, radii, motion */ },
  kumoOverrides: {
    text: { "kumo-default": "var(--text-primary)", ... },
    color: { "kumo-brand": "var(--accent)", ... },
  },
};

// === ACCOUNTS — consumed by src/components/DevAccounts.tsx ===
export const ACCOUNTS = [
  {
    email: "alice@acme.example",                  // matches a user seed() creates
    label: "Alice (owner of 6 orgs)",
    scenario: "one-line description of what's interesting",
    landAt: "/",                                  // where to navigate post-login
    badge: "orange" | "blue" | "purple" | "teal",
  },
  // ...
];

// === SEED — consumed by scripts/seed/run.mjs ===
export async function seed(h) {
  // h is the helpers object from scripts/seed/helpers.mjs:
  //   h.ensureUser(email), h.ensureOrg(user, name),
  //   h.tryInviteToOrg(inviter, orgId, email, role),
  //   h.tryInviteToBilling(...), h.inviteAndJoin(...)
  // Idempotent: re-running is safe; existing users/orgs are reused.
  return {
    users: { alice: { email, token, whoami }, ... },
    orgs:  { acme: orgId, ... },
    notes: { alice: "/billing → 6 orgs visible; ...", ... },
  };
}
```

The full type lives in `src/seed-scenarios/types.ts` — frontend code
uses it for typed imports.

---

## §2 — Auto-discovery: no SCENARIOS registry to maintain

Both Vite and Node enumerate `scenarios/` at runtime/build time:

**Frontend (Vite):**
```ts
// src/seed-scenarios/index.ts
const modules = import.meta.glob<Scenario>(
  "/scenarios/*/scenario.mjs",
  { eager: true },
);
const SCENARIOS = Object.fromEntries(
  Object.values(modules).map((mod) => [mod.SCENARIO_NAME, mod]),
);
export const ACTIVE_SCENARIO =
  SCENARIOS[import.meta.env.VITE_SEED_SCENARIO ?? "editorial"]
  ?? SCENARIOS.editorial;
```

**Backend (Node):**
```js
// scripts/seed/run.mjs
const available = await readdir("scenarios");
const scenario = await import(`../../scenarios/${SCENARIO}/scenario.mjs`);
```

**Theme generator (Node):**
```js
// scripts/theme-generator/generate.mjs
for (const name of await readdir("scenarios")) {
  const mod = await import(`../../scenarios/${name}/scenario.mjs`);
  // validates mod.SCENARIO_NAME === folder name, emits CSS per scenario
}
```

Adding a third scenario: `mkdir scenarios/<new>`, write
`scenario.mjs`, run `mise run kumo:theme-gen`. Nothing else changes.

---

## §3 — Naming convention for emails

> **All seed emails use the `.example` TLD.**

Per [RFC 6761](https://datatracker.ietf.org/doc/html/rfc6761), `.example`
is permanently reserved for documentation and never resolves on the
public internet. Real customer emails cannot collide with seed accounts.

So:
- `alice@acme.example` ✓
- `coach@bangkok-suns.example` ✓
- `นักบาส@สโมสร.example` ✓ (Thai script — exercises font rendering)
- `alice@acme.com` ✗ (would shadow real `@acme.com` users)

This makes the seed safe to run against production for QA work
("can I sign in as alice@acme.example on the live deploy?") without
ever risking a real-user collision.

---

## §4 — Scenarios share one database, not isolated tenants

**Important caveat to flag, especially when you have multiple
scenarios:**

There is ONE D1 database (or whatever your backing store is). Running
`mise run seed:dev` and then `mise run seed:dev:remysport` produces a
database holding BOTH scenarios' users + orgs + invites. The frontend's
`VITE_SEED_SCENARIO` only controls which sign-in cards appear in
DevAccounts — not what data exists in the database.

In practice this is usually fine — every email is `.example`, every
org has a distinct name, so they don't collide. But it does mean:

- Running both seeds doesn't "switch" demos. It additively populates
  the same DB.
- The `Members` / `Invitations` etc. pages will show data from whatever
  scope the logged-in user is in — coach@bangkok-suns sees their
  basketball teams, alice@acme sees Acme orgs. No bleed.
- If you want a TRUE clean-slate per scenario, you need to nuke the DB
  between scenarios (`mise run worker:teardown`, then reseed).

For a "demo deploy" production pattern — separate Workers (each with
their own D1) per scenario — set `VITE_SEED_SCENARIO=remysport` at
both deploy + seed time, and use a distinct worker name + D1.

---

## §5 — Seed scripts MUST be idempotent

Re-running a scenario's `seed()` should never fail or double-create.
The helpers enforce this:

- `h.ensureUser(email)` — tries `Login` first, signs up only if not found.
- `h.ensureOrg(user, name)` — lists orgs first, creates only if not found.
- `h.tryInviteToOrg(...)` — silently no-ops if the user is already a
  member or the invite already exists.

When authoring a new scenario, use these helpers. If you call raw RPCs
directly, you'll re-create users on every seed and the run will fail
with `email already exists` style errors.

---

## §6 — Per-scenario manifests are gitignored

`scripts/seed/run.mjs` writes `.seed.<scenario>.json` containing live
session tokens for the seeded users. The frontend (e.g. DevAccounts'
"Sign in as alice" button) re-logs in via the password rather than
reading the manifest — but the manifest is useful for `curl`-based
QA scripts and for inspecting whoami payloads.

`.gitignore` has `.seed.*.json` so manifests for every scenario stay
out of git. Each scenario gets its own file:
- `.seed.editorial.json`
- `.seed.remysport.json`

---

## §7 — Env-var coherence: one var = one demo

```
VITE_SEED_SCENARIO=remysport pnpm dev
  ↓
  index.html: <html data-theme="remysport">
  Vite: ACTIVE_SCENARIO = remysport
  Frontend: DevAccounts shows coach/captain/scout/manager
  Theme: orange + paper + Inter
  ↓
SCENARIO=remysport mise run seed:dev
  ↓
  Backend: creates coach@bangkok-suns + 5 teams + scout invites
```

`VITE_SEED_SCENARIO` (build-time, for the frontend bundle) and
`SCENARIO` (run-time, for the seed) should match. By convention the
mise tasks pair them — for any scenario `<name>` you should have THREE
tasks: a local seed, a prod seed, and a prod deploy:

| mise task | What | Pair with |
| --- | --- | --- |
| `seed:dev` | Editorial scenario → local :8787 | `pnpm dev` |
| `seed:dev:remysport` | RemySports scenario → local :8787 | `VITE_SEED_SCENARIO=remysport pnpm dev` |
| `worker:deploy` | Build (editorial) + deploy | `seed:prod` |
| `worker:deploy:remysport` | Build (remysport) + deploy | `seed:prod:remysport` |
| `seed:prod` | Editorial scenario → deployed URL | `worker:deploy` |
| `seed:prod:remysport` | RemySports scenario → deployed URL | `worker:deploy:remysport` |

When you `pnpm dev` without setting the env, Vite defaults to
`editorial` via `vite.config.ts`. Same for the seed runner.

**Important coupling:** `worker:deploy*` and `seed:prod*` MUST use the
matching scenario or the demo breaks (frontend would show
coach@bangkok-suns but DB would only have alice@acme). Always run them
as a pair:

```
mise run worker:deploy:remysport && mise run seed:prod:remysport
```

---

## §8 — Don't use a `.env` file for the scenario default

`.env` is in most developers' global `~/.gitignore` to prevent
accidental secret commits. If your scenario default lives there, fresh
clones won't have it, and Vite's `%VITE_SEED_SCENARIO%` substitution
will leave the placeholder literal in `index.html` — broken.

**Instead**, set the default in `vite.config.ts`:

```ts
process.env.VITE_SEED_SCENARIO ??= "editorial";

export default defineConfig({ ... });
```

Vite's `loadEnv` picks this up before processing HTML. Shell env still
overrides (`VITE_SEED_SCENARIO=remysport pnpm dev`).

---

## §9 — Bundle includes ALL scenarios (acceptable today)

`import.meta.glob({ eager: true })` statically imports every
`scenarios/*/scenario.mjs` into the bundle. Only the active one is
displayed, but all of them ship to the browser.

Cost: a few KB per scenario (mostly theme tokens + accounts metadata).
With 2 scenarios it's invisible; with 20 you'd want to code-split.

To code-split when it matters: switch to `eager: false`, return async
loader functions, refactor `DevAccounts.tsx` to handle a loading state.
Not in scope by default.

---

## §10 — When to graduate to "scenario per database"

Today: one D1, all scenarios coexist. Fine for demos.

Graduate when:
- Cedar policies start enforcing tenant boundaries and you want to
  prove they hold across distinct production-shape databases.
- A scenario needs different schema (e.g. RemySports adds a "game"
  table that editorial doesn't have).
- Demo deploys go to real prospects and you want truly isolated data.

Path: separate Wrangler deployment per scenario (different
`workers-<name>` name, different `D1_DATABASE_ID` in the keychain),
each seeded independently. The `VITE_SEED_SCENARIO` build env still
drives the bundle.

---

## Quick checklist when adding a new scenario

- [ ] `mkdir scenarios/<name>`
- [ ] Write `scenarios/<name>/scenario.mjs` exporting SCENARIO_NAME,
      DESCRIPTION, PASSWORD, THEME, ACCOUNTS, seed
- [ ] `SCENARIO_NAME` matches folder name (the generator validates this)
- [ ] All ACCOUNT emails use `.example` TLD
- [ ] `seed()` uses idempotent helpers, returns `{ users, orgs, notes }`
- [ ] Add mise tasks `seed:dev:<name>` + `seed:prod:<name>` (copy
      existing pair, swap SCENARIO value)
- [ ] Run `mise run kumo:theme-gen` — should report your scenario
      among "N scenario(s)"
- [ ] Run `SCENARIO=<name> node scripts/seed/run.mjs` against local
      dev — should print a tour
- [ ] `VITE_SEED_SCENARIO=<name> pnpm dev` — visit /preview, click a
      DevAccount, verify sign-in works
- [ ] Theme paints correctly: `<html data-theme="<name>">` is set,
      brand colors come through

[seed-md]: ./SEED.md
