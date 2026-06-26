// Platform-level policies — global actions not tied to a specific
// Event / Team / Player instance.
//
// The PUBLIC + ANY_SIGNED_IN distinction matters: a real anonymous
// visitor (no session) doesn't go through the CedarLayer at all (no
// principal). For ANY_SIGNED_IN we check `principal is User` — that
// alone proves a session exists.
//
// Role-based gates (PLATFORM_ADMIN, ANY_ORGANIZER, etc.) come from
// `principal.role` (the User entity attribute populated from
// users.role_code at session-mint time).
@id("public-platform-reads")
permit (
  principal, // anonymous OK for some, but our middleware only invokes Cedar when principal exists; PUBLIC actions effectively become ANY_SIGNED_IN in practice
  action in Action::"PublicPlatformActions",
  resource is Platform
);

@id("signed-in-platform-actions")
permit (
  principal is User,
  action in Action::"SignedInPlatformActions",
  resource is Platform
);

@id("admin-platform-actions")
permit (
  principal is User,
  action in Action::"AdminPlatformActions",
  resource is Platform
)
when { principal.role == "ADMIN" };

// CREATE_EVENT and AI_CREATE_EVENT: organizers OR admins.
@id("organizer-or-admin-can-create-events")
permit (
  principal is User,
  action in Action::"OrganizerCreateActions",
  resource is Platform
)
when { principal.role == "ORGANIZER" || principal.role == "ADMIN" };

// CREATE_TEAM: coaches OR admins. (CREATE_PLAYER also allowed for players.)
@id("coach-or-admin-can-create-teams")
permit (
  principal is User,
  action == Action::"CREATE_TEAM",
  resource is Platform
)
when { principal.role == "COACH" || principal.role == "ADMIN" };

@id("coach-player-or-admin-can-create-player-profiles")
permit (
  principal is User,
  action == Action::"CREATE_PLAYER",
  resource is Platform
)
when
{
  principal.role == "COACH" ||
  principal.role == "PLAYER" ||
  principal.role == "ADMIN"
};
