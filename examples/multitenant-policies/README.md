# multitenant-policies

Cedar schema + policies that re-express the hand-rolled `require_*` helpers
in `.src/example-multitenant-worker/src/services/authz.rs` as a Cedar
policy set.

```
multitenant.cedarschema   entity + action surface
policies/org.cedar        org read/write rules
policies/billing.cedar    billing read/owner rules
policies/invitation.cedar invitee + list-self rules
```

## Model

- `BillingAccount` is the tenant boundary.
- `Organization in [BillingAccount]` — every org has one billing parent.
- `User` carries four membership sets as attributes:
  `ownerOrgs`, `memberOrgs`, `ownerBillings`, `memberBillings`.
- Owner-of-billing also authorizes org-owner actions
  (`require_org_or_billing_owner` in the original).

Actions use the fully-qualified proto path:
`workers.<pkg>.v1.<Service>.<Method>`. The `CedarLayer` in this crate
maps the HTTP path `/workers.org.v1.OrgService.GetOrganization` to
`Action::"workers.org.v1.OrgService.GetOrganization"` mechanically.

## Validate

```
mise run cedar:validate
```

The task globs `examples/*-policies/` so any future example dirs are
picked up automatically.
