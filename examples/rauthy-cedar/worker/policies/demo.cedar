// Any authenticated user (a valid Rauthy token got them past OidcLayer) may Read.
@id("authed-can-read")
permit (
  principal is User,
  action == Action::"demo.v1.Api.Read",
  resource is Api
);

// Admin requires the "admin" role from the Rauthy token (mapped into context).
@id("admin-role-only")
permit (
  principal is User,
  action == Action::"demo.v1.Api.Admin",
  resource is Api
)
when { context.roles.contains("admin") };

// Super requires a "superuser" role. The bootstrap admin does NOT have it, so
// this is the DENY case (no policy permits it → Cedar default-deny).
@id("super-role-only")
permit (
  principal is User,
  action == Action::"demo.v1.Api.Super",
  resource is Api
)
when { context.roles.contains("superuser") };
