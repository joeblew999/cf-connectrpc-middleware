// Policies for the Rauthy → Cedar example.
//
// Each rule reads a different slice of the Rauthy token to show the mapping:
//   - `principal.roles`  → RBAC (admin override)
//   - `principal.groups` → resource scoping (group membership)
//   - `context.scopes`   → OAuth-scope gating (write needs the `write` scope)

// Admins (JWT carried "admin" in `roles`) can do anything to any Doc.
@id("admin-full-access")
permit (
  principal is User,
  action,
  resource is Doc
)
when {
  principal.roles.contains("admin")
};

// Any signed-in user can read a Doc owned by a group they belong to
// (JWT `groups` ∋ the Doc's owning group).
@id("group-members-can-read")
permit (
  principal is User,
  action == Action::"read",
  resource is Doc
)
when {
  principal.groups.contains(resource.group)
};

// Writing additionally requires the OIDC `write` scope on the token —
// being in the group isn't enough. This shows context-level (scope) gating
// layered on top of group membership.
@id("group-members-with-write-scope-can-write")
permit (
  principal is User,
  action == Action::"write",
  resource is Doc
)
when {
  principal.groups.contains(resource.group) && context.scopes.contains("write")
};
