//! Live oidc→cedar demo — the full AuthN→AuthZ chain on a REAL Rauthy token.
//!
//! This is the same two crates a Worker wires (`connectrpc-oidc` +
//! `connectrpc-cedar`), run as a plain binary so you can watch the decision:
//!
//!   1. verify the Rauthy JWT  (connectrpc-oidc)  → Session{sub, roles, ...}
//!   2. map Session → Cedar request (roles in context, dynamic per request)
//!   3. authorize two actions (connectrpc-cedar)  → ALLOW / DENY
//!
//! Driven by examples/rauthy-cedar/e2e.nu, which boots Rauthy, mints a token,
//! and passes the paths in env:
//!   RAUTHY_TOKEN_FILE  RAUTHY_JWKS_FILE  RAUTHY_ISSUER
//!
//! Expected against the bootstrap admin (roles [rauthy_admin, admin]):
//!   read  → ALLOW (any authenticated user)
//!   admin → ALLOW (carries the admin role)
//! A user WITHOUT the admin role would get read=ALLOW, admin=DENY.

use std::time::{SystemTime, UNIX_EPOCH};

use cedar_policy::{Context, EntityUid, RestrictedExpression};
use connectrpc_cedar::CedarAuthorizer;
use connectrpc_oidc::{JwksVerifier, Session};

fn env_file(key: &str) -> String {
    let path = std::env::var(key).unwrap_or_else(|_| panic!("set {key}"));
    std::fs::read_to_string(path).unwrap()
}

fn main() {
    // ── 1. AuthN: verify the real Rauthy token ───────────────────────────────
    let token = env_file("RAUTHY_TOKEN_FILE");
    let jwks = env_file("RAUTHY_JWKS_FILE");
    let issuer = std::env::var("RAUTHY_ISSUER").expect("set RAUTHY_ISSUER");
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

    let verifier = JwksVerifier::from_jwks_json(&issuer, Some("worker-client".into()), &jwks)
        .expect("build verifier");
    let claims = verifier
        .verify(token.trim(), now)
        .expect("real Rauthy token must verify");
    let session = Session::from(claims);
    println!(
        "AuthN ✓  sub={}  roles={:?}  scopes={:?}",
        session.subject, session.roles, session.scopes
    );

    // ── 2. map Session → Cedar request (roles ride in context) ───────────────
    let principal: EntityUid = format!(r#"User::"{}""#, session.subject).parse().unwrap();
    let resource: EntityUid = r#"Api::"main""#.parse().unwrap();
    let context = Context::from_pairs([
        (
            "roles".to_string(),
            RestrictedExpression::new_set(
                session
                    .roles
                    .iter()
                    .map(|r| RestrictedExpression::new_string(r.clone())),
            ),
        ),
        (
            "scopes".to_string(),
            RestrictedExpression::new_set(
                session
                    .scopes
                    .iter()
                    .map(|s| RestrictedExpression::new_string(s.clone())),
            ),
        ),
    ])
    .unwrap();

    // ── 3. AuthZ: load policies and authorize two actions ────────────────────
    let authz = CedarAuthorizer::from_str(
        include_str!("../policies/demo.cedarschema"),
        include_str!("../policies/demo.cedar"),
    )
    .expect("load policies");

    let mut any_fail = false;
    for action_name in ["read", "admin"] {
        let action: EntityUid = format!(r#"Action::"{action_name}""#).parse().unwrap();
        let (decision, reasons) =
            authz.is_authorized(&principal, &action, &resource, context.clone());
        let d = format!("{decision:?}").to_uppercase();
        println!("AuthZ  action={action_name:<6} → {d}  {reasons:?}");
        // Sanity: admin user should be allowed both.
        if d != "ALLOW" {
            any_fail = true;
        }
    }

    if any_fail {
        eprintln!("DEMO FAILED: admin user was denied an action it should have");
        std::process::exit(1);
    }
    println!("DEMO OK — real Rauthy token verified AND authorized by Cedar.");
}
