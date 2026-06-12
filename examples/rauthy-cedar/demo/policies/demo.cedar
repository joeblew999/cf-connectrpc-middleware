// Any authenticated user (a valid Rauthy token got them this far) may "read".
@id("authed-can-read")
permit (
  principal is User,
  action == Action::"read",
  resource is Api
);

// Only a user whose Rauthy token carries the "admin" role may "admin".
// `roles` comes from the token, mapped into context by the OIDC layer.
@id("admin-role-only")
permit (
  principal is User,
  action == Action::"admin",
  resource is Api
)
when { context.roles.contains("admin") };
