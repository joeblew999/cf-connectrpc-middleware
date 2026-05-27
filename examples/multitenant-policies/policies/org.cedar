// Org-level policies.
//
// Mirrors the require_* helpers in
// .src/example-multitenant-worker/src/services/authz.rs.
//
// Owner-of-billing also authorizes org-owner actions
// (matches require_org_or_billing_owner).

@id("org-read-by-member")
permit (
  principal is User,
  action in Action::"OrgReadActions",
  resource is Organization
)
when {
  principal.memberOrgs.contains(resource) ||
  principal.memberBillings.contains(resource.billing)
};

@id("org-write-by-owner")
permit (
  principal is User,
  action in Action::"OrgOwnerActions",
  resource is Organization
)
when {
  principal.ownerOrgs.contains(resource) ||
  principal.ownerBillings.contains(resource.billing)
};

@id("org-create-by-billing-owner")
permit (
  principal is User,
  action == Action::"workers.org.v1.OrgService.CreateOrganization",
  resource is BillingAccount
)
when {
  principal.ownerBillings.contains(resource)
};
