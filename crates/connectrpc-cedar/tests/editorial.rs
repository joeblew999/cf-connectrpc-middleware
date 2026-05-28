//! Integration test: validate that the editorial (Acme) Cedar policies
//! from `examples/multitenant-policies/` load via `CedarAuthorizer` and
//! produce the expected decisions for the rules `services/authz.rs`
//! currently encodes by hand.
//!
//! Each case is a (principal, action, resource, context, expected)
//! tuple matching one of the hand-rolled `require_*` paths. The
//! intent is to prove the substrate works before integrating into the
//! multitenant-worker — if these fail, the whole shadow-mode plan
//! doesn't ship.

use cedar_policy::{Context, Decision, EntityUid, RestrictedExpression};
use connectrpc_cedar::{CedarAuthorizer, action::action_from_path};

const SCHEMA: &str = include_str!("../../../examples/multitenant-policies/multitenant.cedarschema");
const BILLING_POLICIES: &str = include_str!("../../../examples/multitenant-policies/policies/billing.cedar");
const INVITATION_POLICIES: &str = include_str!("../../../examples/multitenant-policies/policies/invitation.cedar");
const ORG_POLICIES: &str = include_str!("../../../examples/multitenant-policies/policies/org.cedar");

fn authorizer() -> CedarAuthorizer {
    let combined = format!("{BILLING_POLICIES}\n\n{INVITATION_POLICIES}\n\n{ORG_POLICIES}");
    CedarAuthorizer::from_str(SCHEMA, &combined)
        .expect("editorial policies must load + validate against the schema")
}

fn uid(s: &str) -> EntityUid {
    s.parse().unwrap_or_else(|e| panic!("invalid entity uid {s:?}: {e}"))
}

/// Build a `ScopeContext` matching the editorial schema:
///   type ScopeContext = { billing: BillingAccount, org?: Organization, role: String };
fn scope_ctx(billing: &str, role: &str, org: Option<&str>) -> Context {
    let mut pairs = vec![
        (
            "billing".to_string(),
            RestrictedExpression::new_entity_uid(uid(&format!(r#"BillingAccount::"{billing}""#))),
        ),
        (
            "role".to_string(),
            RestrictedExpression::new_string(role.to_string()),
        ),
    ];
    if let Some(o) = org {
        pairs.push((
            "org".to_string(),
            RestrictedExpression::new_entity_uid(uid(&format!(r#"Organization::"{o}""#))),
        ));
    }
    Context::from_pairs(pairs).expect("scope context pairs are well-typed")
}

// ────────────────────────────────────────────────────────────────────
// Path → Action mapping (the bridge between Connect-RPC URLs and Cedar)
// ────────────────────────────────────────────────────────────────────

#[test]
fn action_mapping_round_trips_proto_path() {
    let a = action_from_path("/workers.billing.v1.BillingService/DeleteBillingAccount").unwrap();
    assert_eq!(
        a.to_string(),
        r#"Action::"workers.billing.v1.BillingService.DeleteBillingAccount""#
    );
}

// ────────────────────────────────────────────────────────────────────
// Billing policies
// ────────────────────────────────────────────────────────────────────

#[test]
fn billing_member_can_read_their_billing_account() {
    let authz = authorizer();
    let principal = uid(r#"User::"alice""#);
    let action = action_from_path("/workers.billing.v1.BillingService/GetBillingAccount").unwrap();
    let resource = uid(r#"BillingAccount::"acme""#);
    let ctx = scope_ctx("acme", "member", None);
    let (decision, reasons) = authz.is_authorized(&principal, &action, &resource, ctx);
    assert_eq!(
        decision,
        Decision::Allow,
        "billing member should read; reasons={reasons:?}"
    );
}

#[test]
fn billing_member_cannot_read_someone_elses_billing_account() {
    let authz = authorizer();
    let principal = uid(r#"User::"alice""#);
    let action = action_from_path("/workers.billing.v1.BillingService/GetBillingAccount").unwrap();
    // Alice's pinned scope is acme, but the request targets initech.
    let resource = uid(r#"BillingAccount::"initech""#);
    let ctx = scope_ctx("acme", "owner", None);
    let (decision, _) = authz.is_authorized(&principal, &action, &resource, ctx);
    assert_eq!(decision, Decision::Deny, "scope mismatch should deny");
}

#[test]
fn billing_owner_can_delete_their_billing_account() {
    let authz = authorizer();
    let principal = uid(r#"User::"alice""#);
    let action = action_from_path("/workers.billing.v1.BillingService/DeleteBillingAccount").unwrap();
    let resource = uid(r#"BillingAccount::"acme""#);
    let ctx = scope_ctx("acme", "owner", None);
    let (decision, _) = authz.is_authorized(&principal, &action, &resource, ctx);
    assert_eq!(decision, Decision::Allow, "billing owner should delete");
}

#[test]
fn billing_member_cannot_delete_billing_account() {
    let authz = authorizer();
    let principal = uid(r#"User::"alice""#);
    let action = action_from_path("/workers.billing.v1.BillingService/DeleteBillingAccount").unwrap();
    let resource = uid(r#"BillingAccount::"acme""#);
    let ctx = scope_ctx("acme", "member", None);
    let (decision, _) = authz.is_authorized(&principal, &action, &resource, ctx);
    assert_eq!(decision, Decision::Deny, "non-owner should not delete billing");
}

#[test]
fn billing_member_cannot_invite_to_billing() {
    let authz = authorizer();
    let principal = uid(r#"User::"alice""#);
    let action = action_from_path("/workers.billing.v1.BillingService/InviteMember").unwrap();
    let resource = uid(r#"BillingAccount::"acme""#);
    let ctx = scope_ctx("acme", "member", None);
    let (decision, _) = authz.is_authorized(&principal, &action, &resource, ctx);
    assert_eq!(decision, Decision::Deny);
}

// ────────────────────────────────────────────────────────────────────
// Schema validation — the authorizer must reject a malformed schema
// at construction time, not silently authorize-everything.
// ────────────────────────────────────────────────────────────────────

#[test]
fn malformed_schema_fails_to_build() {
    let bad_schema = "this is not a Cedar schema!";
    let result = CedarAuthorizer::from_str(bad_schema, BILLING_POLICIES);
    assert!(result.is_err(), "malformed schema must fail at build");
}

#[test]
fn policies_referencing_unknown_actions_fail_validation() {
    let bad_policies = r#"
        @id("references-action-not-in-schema")
        permit (
            principal,
            action == Action::"workers.totally.fake.Service.DoesNotExist",
            resource
        );
    "#;
    let result = CedarAuthorizer::from_str(SCHEMA, bad_policies);
    assert!(
        result.is_err(),
        "policy referencing an undeclared action must fail validation"
    );
}
