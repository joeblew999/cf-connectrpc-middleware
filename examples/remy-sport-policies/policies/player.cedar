// Player-level policies — per-player-instance actions.
//
// Relations the middleware packs into context.player_relations:
//   SELF (from players.user_id), GUARDIAN (from guardians.user_id),
//   FOLLOWER_PLAYER (from subscriptions)
//
// Also: HEAD_COACH / ASSISTANT_COACH of the player's TEAM grant edit
// permission — that's modelled by passing the player's team-relations
// through too (context.team_relations on a Player request describes
// the requester's relation to the player's team, not directly to the
// player). Slightly indirect; documented here.
@id("player-public-views")
permit (
  principal,
  action in Action::"PlayerViewActions",
  resource is Player
);

// EDIT_PLAYER_PROFILE: HEAD_COACH + ASSISTANT_COACH (of the player's
// team) + SELF + GUARDIAN + PLATFORM_ADMIN.
@id("player-edit-by-coach-self-guardian-or-admin")
permit (
  principal is User,
  action == Action::"EDIT_PLAYER_PROFILE",
  resource is Player
)
when
{
  principal.role == "ADMIN" ||
  (context has player_relations &&
   (context.player_relations.contains("SELF") ||
    context.player_relations.contains("GUARDIAN"))) ||
  (context has team_relations &&
   (context.team_relations.contains("HEAD_COACH") ||
    context.team_relations.contains("ASSISTANT_COACH")))
};

// REGISTER_PLAYER_FOR_EVENT: SELF + GUARDIAN + PLATFORM_ADMIN, only
// for event types K/Sh (Camp + Showcase).
@id("player-event-registration")
permit (
  principal is User,
  action == Action::"REGISTER_PLAYER_FOR_EVENT",
  resource is Player
)
when
{
  context has event_type &&
  ["K", "Sh"].contains(context.event_type) &&
  (principal.role == "ADMIN" ||
   (context has player_relations &&
    (context.player_relations.contains("SELF") ||
     context.player_relations.contains("GUARDIAN"))))
};
