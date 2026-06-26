// Invitation policies.
//
// Get / Accept / Decline: only the invited email may act (no scope needed).
// ListPending: principal-on-self.
// Resend / Revoke: org owner only (uses session scope).
@id("invitation-act-by-invitee")
permit (
  principal is User,
  action in
    [Action::"workers.invitation.v1.InvitationService.GetInvitation",
     Action::"workers.invitation.v1.InvitationService.AcceptInvitation",
     Action::"workers.invitation.v1.InvitationService.DeclineInvitation"],
  resource is Invitation
)
when { principal.email == resource.inviteeEmail };

@id("invitation-list-self")
permit (
  principal is User,
  action ==
    Action::"workers.invitation.v1.InvitationService.ListPendingInvitations",
  resource is User
)
when { principal == resource };

@id("invitation-owner-actions")
permit (
  principal is User,
  action in
    [Action::"workers.invitation.v1.InvitationService.ResendInvitation",
     Action::"workers.invitation.v1.InvitationService.RevokeInvitation"],
  resource is Invitation
)
when { context.billing == resource.org.billing && context.role == "owner" };
