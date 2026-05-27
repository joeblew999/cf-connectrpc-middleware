# multitenant-policies

Cedar schema + policies that re-express the hand-rolled `require_*` helpers
in `.src/example-multitenant-worker/src/services/authz.rs` as a Cedar
policy set.

```
multitenant.cedarschema   entity + action surface
policies/org.cedar        org read / write rules
policies/billing.cedar    billing read / owner rules
policies/invitation.cedar invitee + list-self + org-owner rules
```

## Model: session attenuates, Cedar evaluates

The example uses **macaroon-based sessions**. By the time a request reaches
the worker, the `AuthLayer` has verified the macaroon and produced a
`SessionContext`:

```rust
pub struct SessionContext {
    pub user: UserId,
    pub email: String,
    pub billing: BillingAccountId,
    pub org: Option<OrgId>,
    pub role: Role,
    pub auth_method: AuthMethod,
    pub exp_unix: i64,
}
```

The session has already **pinned the active scope** (billing + optional org
+ role). Users switch context via the `SwitchContext` RPC, which re-issues
the macaroon with new caveats.

We pass that pinned scope through Cedar's per-request **`context`** rather
than duplicating membership data on the User entity. This means:

- No DB round trip for memberships at authorization time — the macaroon
  already did it at issue time.
- Policies are simple equality checks (`context.billing == resource.billing
  && context.role == "owner"`) instead of set-containment chains.
- The macaroon and Cedar layer cleanly: macaroon attenuates **what scope
  the token covers**; Cedar evaluates **whether the action is allowed at
  that scope**.

## Entity model

- `BillingAccount` is the tenant boundary.
- `Organization in [BillingAccount]` — every org has one billing parent.
- `Invitation in [Organization]` — invitations belong to an org.
- `User` carries only identity (`email`). Scope lives in `context`.

## Shared context type

```cedar
type ScopeContext = {
  billing: BillingAccount,
  org?: Organization,
  role: String,
};
```

Every action that needs the session scope declares `context: ScopeContext`
in its `appliesTo`. Public actions (signup/login) and pure-identity actions
(invitee-by-email) declare no context.

## Action naming

Actions use the fully-qualified proto path:
`workers.<pkg>.v1.<Service>.<Method>`. The `CedarLayer` in this crate
maps the HTTP path `/workers.org.v1.OrgService.GetOrganization` to
`Action::"workers.org.v1.OrgService.GetOrganization"` mechanically — no
lookup table.

## Validate

```
mise run cedar:validate
```

The task globs `examples/*-policies/` so any future example dirs are
picked up automatically.

## What's next

The basic port above doesn't yet justify pulling Cedar in. See
[ROADMAP.md](ROADMAP.md) for the five Cedar-specific features to add
(forbid guardrails, policy templates for grants, `cedar symcc`
verification, `whoami:permissions` introspection, per-tenant policy
overrides) and the structural changes to the example code that unlock
them.
