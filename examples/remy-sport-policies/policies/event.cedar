// Event-level policies — per-event-instance actions.
//
// Relations the middleware packs into context.event_relations (from
// events.organizer_user_id + event_co_organizers + subscriptions):
//   OWNER, CO_ORGANIZER, FOLLOWER_EVENT
//
// Event subtype passes through as context.event_type ("T" | "L" | "K" | "Sh")
// — required for scoring actions which only apply to L/Sh/T per matrix.md.

// VIEW_EVENT and all the view-actions are PUBLIC per the matrix.
// Cedar receives the request only if a principal exists, but we permit
// any principal as long as the action is in the view group.
@id("event-public-views")
permit (
  principal,
  action in Action::"EventViewActions",
  resource is Event
);

// EDIT_EVENT, DELETE_EVENT, INVITE_CO_ORGANIZER, MANAGE_DIVISIONS,
// GENERATE_BRACKETS, GENERATE_FIXTURES, ASSIGN_COURTS:
//   matrix says OWNER + CO_ORGANIZER + PLATFORM_ADMIN.
//   DELETE_EVENT is OWNER + PLATFORM_ADMIN only — narrower than the rest.
//   We split into two policies for the narrower one.
@id("event-owner-actions-by-owner-coorg-or-admin")
permit (
  principal is User,
  action in Action::"EventOwnerActions",
  resource is Event
)
when {
  principal.role == "ADMIN" ||
  (context has event_relations && context.event_relations.contains("OWNER")) ||
  (context has event_relations && context.event_relations.contains("CO_ORGANIZER"))
}
unless {
  // DELETE_EVENT is owner+admin only, not co-organizer.
  action == Action::"DELETE_EVENT" &&
  principal.role != "ADMIN" &&
  !(context has event_relations && context.event_relations.contains("OWNER"))
};

// ENTER_SCORES + CONFIRM_MATCH_STATUS:
//   matrix says OWNER + CO_ORGANIZER + PLATFORM_ADMIN + ANY_REFEREE,
//   but only for event_type in {L, Sh, T} — NOT for K (Camp).
@id("scoring-by-owner-coorg-referee-or-admin")
permit (
  principal is User,
  action in Action::"EventScoringActions",
  resource is Event
)
when {
  context has event_type &&
  ["L", "Sh", "T"].contains(context.event_type) &&
  (
    principal.role == "ADMIN" ||
    principal.role == "REFEREE" ||
    (context has event_relations && context.event_relations.contains("OWNER")) ||
    (context has event_relations && context.event_relations.contains("CO_ORGANIZER"))
  )
};
