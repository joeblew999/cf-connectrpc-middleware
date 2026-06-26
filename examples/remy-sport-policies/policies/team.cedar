// Team-level policies — per-team-instance actions.
//
// Relations the middleware packs into context.team_relations (from
// team_coaches + player_teams + subscriptions):
//   HEAD_COACH, ASSISTANT_COACH, TEAM_MANAGER, TEAM_PLAYER, FOLLOWER_TEAM
@id("team-public-views")
permit (
  principal,
  action in Action::"TeamViewActions",
  resource is Team
);

// EDIT_TEAM_PROFILE: HEAD_COACH + TEAM_MANAGER + PLATFORM_ADMIN.
@id("team-edit-by-head-coach-manager-or-admin")
permit (
  principal is User,
  action == Action::"EDIT_TEAM_PROFILE",
  resource is Team
)
when
{
  principal.role == "ADMIN" ||
  (context has team_relations &&
   context.team_relations.contains("HEAD_COACH")) ||
  (context has team_relations &&
   context.team_relations.contains("TEAM_MANAGER"))
};

// MANAGE_ROSTER: HEAD_COACH + ASSISTANT_COACH + TEAM_MANAGER + PLATFORM_ADMIN.
@id("roster-mgmt-by-any-coach-or-admin")
permit (
  principal is User,
  action == Action::"MANAGE_ROSTER",
  resource is Team
)
when
{
  principal.role == "ADMIN" ||
  (context has team_relations &&
   (context.team_relations.contains("HEAD_COACH") ||
    context.team_relations.contains("ASSISTANT_COACH") ||
    context.team_relations.contains("TEAM_MANAGER")))
};

// REGISTER_TEAM_FOR_EVENT: HEAD_COACH + TEAM_MANAGER + PLATFORM_ADMIN,
// only for event types L/Sh/T (matrix doesn't allow Camp team-registration).
@id("team-registration-for-event")
permit (
  principal is User,
  action == Action::"REGISTER_TEAM_FOR_EVENT",
  resource is Team
)
when
{
  context has event_type &&
  ["L", "Sh", "T"].contains(context.event_type) &&
  (principal.role == "ADMIN" ||
   (context has team_relations &&
    (context.team_relations.contains("HEAD_COACH") ||
     context.team_relations.contains("TEAM_MANAGER"))))
};
