// Billing-level policies.
//
// Read: any billing member (session pins billing to the right one).
// Mutate: owner role.
@id("billing-read-by-member")
permit (
  principal is User,
  action in Action::"BillingReadActions",
  resource is BillingAccount
)
when { context.billing == resource };

@id("billing-write-by-owner")
permit (
  principal is User,
  action in Action::"BillingOwnerActions",
  resource is BillingAccount
)
when { context.billing == resource && context.role == "owner" };
