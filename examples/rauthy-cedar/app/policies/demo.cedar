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

// GetDoc is BODY-AWARE: the CedarInterceptor authorizes against the specific
// Doc entity built from the request body's `doc_id`. Only the "public" doc is
// permitted — any other doc_id has no satisfiable policy → default-deny. This
// is a decision the path-based CedarLayer cannot make (it never sees the body).
@id("getdoc-public-only")
permit (
  principal is User,
  action == Action::"demo.v1.Api.GetDoc",
  resource == Doc::"public"
);
