// Org-level policies.
//
// The session has already pinned (billing, org?, role) as the active scope.
// These policies enforce role-on-scope; the data-layer is responsible for
// matching `context.billing` to the resource's parent billing account.

@id("org-read-by-billing-member")
permit (
  principal is User,
  action in Action::"OrgReadActions",
  resource is Organization
)
when {
  context.billing == resource.billing
};

@id("org-write-by-owner")
permit (
  principal is User,
  action in Action::"OrgOwnerActions",
  resource is Organization
)
when {
  context.billing == resource.billing && context.role == "owner"
};

@id("org-create-by-billing-owner")
permit (
  principal is User,
  action == Action::"workers.org.v1.OrgService.CreateOrganization",
  resource is BillingAccount
)
when {
  context.billing == resource && context.role == "owner"
};
