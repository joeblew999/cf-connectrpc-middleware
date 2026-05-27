// Invitation policies.
//
// Get / Accept / Decline: only the invited email may act.
// Resend / Revoke: org owner only (handled by OrgOwnerActions group).
// ListPending: principal-on-self.

@id("invitation-act-by-invitee")
permit (
  principal is User,
  action in [
    Action::"workers.invitation.v1.InvitationService.GetInvitation",
    Action::"workers.invitation.v1.InvitationService.AcceptInvitation",
    Action::"workers.invitation.v1.InvitationService.DeclineInvitation"
  ],
  resource is Invitation
)
when {
  principal.email == resource.inviteeEmail
};

@id("invitation-list-self")
permit (
  principal is User,
  action == Action::"workers.invitation.v1.InvitationService.ListPendingInvitations",
  resource is User
)
when {
  principal == resource
};
