// Billing-level policies.
//
// Read: any billing member.
// Mutate: billing owner only (matches require_billing_owner in authz.rs).

@id("billing-read-by-member")
permit (
  principal is User,
  action in Action::"BillingReadActions",
  resource is BillingAccount
)
when {
  principal.memberBillings.contains(resource)
};

@id("billing-write-by-owner")
permit (
  principal is User,
  action in Action::"BillingOwnerActions",
  resource is BillingAccount
)
when {
  principal.ownerBillings.contains(resource)
};
