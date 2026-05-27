# Roadmap — making Cedar earn its place in the multitenant example

A 1:1 port of `services/authz.rs` `require_*` helpers to Cedar policies
trades one form of code for another. The features below are the ones the
hand-rolled approach **cannot** replicate (or can only replicate with
significant code churn). Each one is small in isolation; together they
make the case for pulling Cedar in.

Status legend: `[ ]` not started · `[~]` in progress · `[x]` done.

---

## 1. `forbid` guardrails

**Status:** `[ ]`

**What:** Cedar's `forbid` rules override `permit` rules. Use them as
cross-cutting guardrails that apply across many actions in one file.

**Two concrete additions:**

```cedar
// policies/guardrails.cedar

@id("mfa-required-for-destructive-actions")
forbid (
  principal is User,
  action in [
    Action::"workers.billing.v1.BillingService.DeleteBillingAccount",
    Action::"workers.billing.v1.BillingService.CancelSubscription",
    Action::"workers.org.v1.OrgService.DeleteOrganization"
  ],
  resource
)
unless { context.mfa_verified == true };

@id("read-only-during-maintenance")
forbid (
  principal,
  action in Action::"BillingOwnerActions",
  resource
)
when { context.maintenance_mode == true };
```

**Why this isn't free in Rust:** the equivalent requires touching every
mutating handler (or threading a `Maintenance` state through middleware).
Here it's one policy added in one file; the rule applies everywhere
those actions are reachable.

**Schema work:** extend `ScopeContext` with `mfa_verified: Bool` and
`maintenance_mode: Bool` (both `false` by default in the Rust glue).

**Demo:**
- Toggle maintenance via a `wrangler dev` env var; deletes return 403
  with policy id `read-only-during-maintenance` in diagnostics.
- Hit `DeleteOrg` without MFA → 403; with MFA bit set → 200.

---

## 2. Policy templates for ad-hoc grants

**Status:** `[ ]`

**What:** Cedar templates are parameterized policies instantiated per
(principal, resource) pair. The instances are **data**, not code —
stored as rows in a table, linked at evaluation time.

**The capability the example currently lacks:** sharing access without a
code change.

```cedar
// policies/templates/grant-org-read.cedar  (template)
@id("grant-org-read")
permit (
  principal == ?principal,
  action in Action::"OrgReadActions",
  resource == ?resource
);

// Instances live in D1:
//   linked_policies(id, template_id, principal, resource, granted_by, expires_at)
// e.g. (id=42, template=grant-org-read, principal=User::"bob", resource=Organization::"acme")
```

**Why this isn't free in Rust:** every share/unshare today needs a code
change or a custom table + per-action lookup. Templates make sharing a
single `INSERT` (and `cedar link` at request time).

**Rust changes:**
- New D1 table `grants`.
- `CedarLayer` loads grants matching `(principal=session.user, resource_kind)`
  at request time and calls `PolicySet::link` to materialize instances
  before `is_authorized`.

**Demo:** `ShareOrgAccess(target_email, expires_at)` RPC inserts a grant;
target user can `GetOrganization` without being a member.

---

## 3. `cedar symcc` formal verification in CI

**Status:** `[ ]`

**What:** Cedar's symbolic compiler can **prove** properties of a policy
set as SAT/UNSAT statements. Not test, prove.

**Example property:**

```text
# .cedar/symcc-claims/no-non-owner-billing-mutation.smt2
# Claim: no path where a non-owner role can take a BillingOwnerActions.
```

Wrap as `mise run cedar:verify`. CI fails on regression.

**Why this isn't free in Rust:** you can write tests, but tests sample;
symcc proves over the entire input space. For audit/security-sensitive
properties this is qualitatively different.

**Rust changes:** none — pure tooling/CI add.

**Demo:** add a deliberately broken policy in a draft branch, watch
`cedar:verify` fail with a counterexample principal+context.

---

## 4. `whoami:permissions` introspection endpoint

**Status:** `[ ]`

**What:** New auth RPC that returns the set of actions allowed for the
current principal in the current scope.

```proto
rpc Permissions(PermissionsRequest) returns (PermissionsResponse);
message PermissionsResponse {
  repeated string allowed_actions = 1;  // fully-qualified action IDs
}
```

Implementation: iterate the schema's known actions, call
`Authorizer::is_authorized` for each with the current session's
context. Cache by `(user, billing, org, role)` tuple — invalidate on
session change.

**Why this isn't free in Rust:** today the frontend either round-trips
speculatively (call, get 403, hide button) or hard-codes role checks
(duplicating server logic). With Cedar as the source of truth, this
endpoint is mechanical and always consistent with the server.

**Rust changes:**
- New `auth.v1.AuthService.Permissions` RPC handler.
- Iterate Cedar's `Schema::actions()` for the action list.

**Demo:** React UI greys out the "Delete Org" button when `Permissions()`
omits that action.

---

## 5. Per-tenant policy overrides

**Status:** `[ ]`

**What:** Enterprise customers get bespoke rules **without a deploy**.
Cedar's `PolicySet` is a runtime value — load tenant-scoped policies
from D1 at request time and merge with the static set.

```text
tenant_policies (billing_id, policy_text, version, created_at)
```

`CedarLayer` builds the PolicySet as: `static_policies ∪ tenant_policies[session.billing]`.

**Concrete use cases the example currently can't serve:**
- "Acme Inc. requires SSO for ALL admin actions, not just billing"
- "Customer Foo blocks DELETE actions outside business hours"
- "Tenant Bar grants their support team read access to all orgs"

**Why this isn't free in Rust:** every override is either a code change
or a customer-specific feature flag, which doesn't scale. Cedar makes
overrides data.

**Rust changes:**
- New D1 table `tenant_policies`.
- `CedarLayer` accepts a `Box<dyn PolicySource>` instead of
  `Arc<PolicySet>`.
- A `cedar validate` step on tenant policies at write-time so bad
  customer policies can't be saved.

**Demo:** insert a tenant policy that adds an extra `forbid`; observe
that customer's traffic gets the new rule without redeploying.

---

# Structural changes to the example to unlock Cedar fully

These are bigger lifts but each one **strengthens the demo** because
the diff becomes more visceral.

## A. Delete `services/authz.rs`. Let `CedarLayer` enforce everything.

The whole point of pulling Cedar in is that `require_*` helpers become
the policy set. Keeping both is mud — readers can't tell which is
authoritative. Once `CedarLayer` is wired, `authz.rs` should be a
deleted file. Handlers become pure business logic; the patch reviewer
sees ~80% of the authz code go away.

**Side-effect requirement:** body-aware authz cases (e.g. "owner of
the org named in `request.org_id`") need a way for `CedarLayer` to see
the resource id. Two options:

- **Per-handler `require_authorized(ctx, action, resource)?`** —
  parallels existing `require_session(ctx)?`. Keeps middleware
  path-aware only; handlers explicitly invoke Cedar for resource-id
  cases. Cleaner.
- **Body-decoding middleware** — heavier; tightly couples middleware
  to proto runtime. Skip.

## B. Add a `ScopeLayer` that pre-loads resource entities

Sits between `AuthLayer` and `CedarLayer`. Reads the URL path, parses
the proto path → resource type, fetches the resource entity (with its
attributes — billing parent, inviteeEmail, etc.) once, inserts it into
`req.extensions()`. `CedarLayer` reads it from there.

Why: without this, every handler that calls `require_authorized` ends
up duplicating the "fetch resource then build Cedar entity" code.
Centralizing it means the resource is loaded once per request and
attribute access (`resource.org.billing`) just works.

## C. Audit log from Cedar diagnostics — free

`Authorizer::is_authorized` returns `Diagnostics` including
`reason()` (which policies fired) and `errors()`. Today the example has
no central audit log. With Cedar, every decision is structured:

```
{ user: "alice", action: "DeleteBillingAccount", resource: "BA::acme",
  decision: "Deny", reason: ["mfa-required-for-destructive-actions"] }
```

Pipe to `console.log` (Workers default observability), or a D1 audit
table for replay. Cost: ~10 lines in `CedarLayer`.

## D. Generate action declarations from `.proto` files at build time

The schema currently hand-lists ~30 action IDs that mirror the proto
RPC paths. A `build.rs` step using `prost-types`/`protox` could parse
the `.proto` files in `proto/` and emit the action declarations into
a generated `actions.cedarschema` fragment.

Why: adding an RPC = adding an action automatically. No risk of an
action being addressable by URL but having no policy decision path.

Cost: a build script (~50 lines). Pays back with every new proto
method that ships.

## E. Split `SessionContext` into `Identity` + `Scope`

```rust
pub struct Identity {
    pub user: UserId,
    pub email: String,
    pub auth_method: AuthMethod,
    pub exp_unix: i64,
}

pub struct Scope {
    pub billing: BillingAccountId,
    pub org: Option<OrgId>,
    pub role: Role,
}

pub struct SessionContext { pub identity: Identity, pub scope: Scope }
```

Why: for Cedar these are different inputs — identity → principal,
scope → context. The current flat struct mixes them. Splitting clarifies
which fields go where and makes the `ScopeLayer` ↔ `CedarLayer` contract
obvious. Small refactor; large readability win.

---

# Suggested order of work

1. Implement the basic `CedarLayer` (no extras — just port the
   `require_*` helpers). This is step 3 of the master plan in CLAUDE.md.
2. Apply structural change **A** (delete `authz.rs`) and **E** (split
   SessionContext) — these make item 1 land cleanly.
3. ROADMAP item **1** (forbid guardrails) — smallest add, biggest
   "oh, that's neat" reaction.
4. ROADMAP item **3** (`cedar symcc` in CI) — pure tooling, no Rust
   changes; locks in correctness for everything that follows.
5. Structural change **C** (Cedar diagnostics → audit log) — 10 lines,
   high value for ops.
6. ROADMAP item **4** (`whoami:permissions`) — proves Cedar can drive UI.
7. Structural change **B** (`ScopeLayer`) — needed before item 2.
8. ROADMAP item **2** (grant templates) — the killer ReBAC feature.
9. ROADMAP item **5** (per-tenant overrides) — the enterprise pitch.
10. Structural change **D** (proto-driven action codegen) — last because
    it's investment, not capability.
