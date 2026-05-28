# remy-sport-policies

Cedar policy exploration for the RemySports / ChampsCircuit ReBAC
matrix (`joeblew999/remy-sport-biz/access/matrix.md` — 186 permission
rows, ~60 actions, 18 relations across 4 object types).

**Purpose:** prove the multitenant-worker's `SessionContext` plus a
modest set of relation strings packed into `context.*_relations` can
express the full ChampsCircuit ReBAC matrix in Cedar. Companion to
`examples/multitenant-policies/` (which models the simpler editorial
Acme scenario).

## What this exploration proves

`cedar:validate` says every policy file is schema-clean. `cedar:test`
runs 20 representative requests covering each relation type the
matrix uses; all pass:

```
$ mise run cedar:test
── remy-sport-policies ──
  ✓ admin can MANAGE_ALL_USERS
  ✓ coach cannot MANAGE_ALL_USERS
  ✓ organizer can CREATE_EVENT
  ✓ spectator cannot CREATE_EVENT
  ✓ coach can CREATE_TEAM
  ✓ player cannot CREATE_TEAM
  ✓ head coach can MANAGE_ROSTER of his team
  ✓ assistant coach can MANAGE_ROSTER
  ✓ coach with no relation to team cannot MANAGE_ROSTER
  ✓ assistant coach cannot EDIT_TEAM_PROFILE (head/manager only)
  ✓ admin can MANAGE_ROSTER on any team
  ✓ referee can ENTER_SCORES on a Tournament
  ✓ referee CANNOT ENTER_SCORES on a Camp (subtype K not scored)
  ✓ event owner can EDIT_EVENT
  ✓ coach with no event relation cannot EDIT_EVENT
  ✓ co-organizer CANNOT DELETE_EVENT (owner+admin only)
  ✓ player can EDIT own profile
  ✓ player cannot EDIT another's profile
  ✓ guardian can EDIT child's profile
  ✓ head coach can EDIT player profile (player is on coach's team)
  pass=20  fail=0  total=20
```

The architecture works.

## Architecture

### Where do relations come from?

The matrix defines 18 relations. Some are **role-based** (PLATFORM_ADMIN
= users with role_code=ADMIN, ANY_COACH = role_code=COACH, etc.) and
some are **object-instance** (HEAD_COACH of a specific team, OWNER of
a specific event).

- **Role-based** relations come for free from `SessionContext.role` —
  baked into the Cedar `User` entity's `role` attribute.

- **Object-instance** relations need a per-request DB lookup: given
  `(principal, resource)`, query the relevant join table (team_coaches,
  player_teams, event_co_organizers, subscriptions). The
  connectrpc-cedar middleware does this query, packs the resulting
  strings into `context.team_relations` / `context.event_relations` /
  `context.player_relations`, and Cedar evaluates against them.

The alternative — loading every relation tuple into Cedar's entity
graph per request — doesn't scale. The context-bound approach scales
linearly with the actions per request, not the tenant size.

### How event subtypes (T/L/K/Sh) work

Several matrix rows are subtype-qualified (e.g. ENTER_SCORES applies
to League/Showcase/Tournament but NOT Camp). Cedar can't enumerate
subtypes via the entity graph without duplicating Event into 4 child
types. Instead the subtype passes through as `context.event_type`
("T" | "L" | "K" | "Sh") and policies guard on it:

```cedar
when {
  ["L", "Sh", "T"].contains(context.event_type) && ...
}
```

### What's NOT covered by this exploration

- **Notification actions** (RECEIVE_*_NOTIFICATIONS) aren't included
  in the action set. The matrix has them but they're really about
  delivery routing, not request-time authz.

- **AI actions** (AI_QA, AI_CREATE_EVENT, AI_BRACKET_SUGGESTIONS) are
  shaped identically to their non-AI siblings — adding them is one
  schema line each, no new architecture.

- **Anonymous PUBLIC actions** are technically permitted by the
  policies (`permit (principal, ...)` no `is User`), but the
  connectrpc-cedar middleware only invokes Cedar when a session
  exists. For truly anonymous endpoints (browse events without
  signing in), the worker should skip the layer entirely.

## What's hard about wiring this to the multitenant-worker DB

Two relation pairs collapse onto our existing schema's 2-state role:

- `team_coaches.HEAD_COACH` AND `ASSISTANT_COACH` AND `TEAM_MANAGER`
  → all become `org_membership.role = OWNER` (or MEMBER for ASSISTANT)
- `player_teams.<rows>` → `org_membership.role = MEMBER`

Cedar policies distinguish HEAD_COACH vs ASSISTANT_COACH (HEAD can
edit team profile, ASSISTANT can't). Our DB doesn't preserve that
distinction. **Options:**

1. **Live with the collapse.** Treat assistant coaches like head
   coaches in our policies — slight authz over-grant.
2. **Stash coach role in the membership row.** Add a non-FK `subrole`
   column (free text) to org_memberships, populated by the seed.
   No schema migration for ChampsCircuit-domain entities; just one
   extra column on an existing table.
3. **Resolve subrole from a sidecar table.** New `team_coaches`
   table holds (user_id, team_id, coach_role_code). The middleware
   joins on it when packing context.team_relations.

The user's direction is "do not extend the DB or rust code" right
now — so option 1 is the default. The collapse means an Assistant
Coach gets EDIT_TEAM_PROFILE in our deployment even though the
matrix says no. Document it as a known divergence; revisit when
the real ChampsCircuit Worker is built (it'll have its own schema).

## Running

```
mise run cedar:validate         # validates every examples/*-policies/
mise run cedar:test             # runs every examples/*-policies/tests/run.nu
mise run cedar:format           # cedar fmt --write across all policies
```

Or directly:

```
cd examples/remy-sport-policies
nu tests/run.nu
```

## Files

```
remy-sport.cedarschema          # entity types + actions + RequestContext
policies/
  platform.cedar                # role-based platform actions (CREATE_*, MANAGE_*)
  event.cedar                   # per-event-instance actions
  team.cedar                    # per-team-instance actions
  player.cedar                  # per-player-instance actions
tests/
  entities.json                 # fixed entity store (users, teams, events, players)
  run.nu                        # 20 representative test cases
```
