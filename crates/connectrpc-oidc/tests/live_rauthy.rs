//! Live end-to-end: verify a REAL Rauthy-issued token through the public API.
//!
//! `#[ignore]` by default — it needs a running Rauthy. The e2e harness boots
//! Rauthy, mints a user token via the password grant, captures the token + JWKS,
//! and runs this with the paths in env:
//!
//! ```sh
//! RAUTHY_TOKEN_FILE=token.txt RAUTHY_JWKS_FILE=jwks.json \
//! RAUTHY_ISSUER=http://localhost:8088/auth/v1/ \
//!   cargo test -p connectrpc-oidc --test live_rauthy -- --ignored --nocapture
//! ```

use connectrpc_oidc::{session_from_claims, JwksVerifier};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
#[ignore = "needs a running Rauthy (driven by the e2e harness)"]
fn verifies_real_rauthy_user_token() {
    let token = std::fs::read_to_string(std::env::var("RAUTHY_TOKEN_FILE").unwrap()).unwrap();
    let jwks = std::fs::read_to_string(std::env::var("RAUTHY_JWKS_FILE").unwrap()).unwrap();
    let issuer = std::env::var("RAUTHY_ISSUER").unwrap();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

    // Verify the real token against Rauthy's real JWKS, enforcing aud.
    let verifier =
        JwksVerifier::from_jwks_json(&issuer, Some("worker-client".into()), &jwks).unwrap();
    let claims = verifier
        .verify(token.trim(), now)
        .expect("a real Rauthy token must verify");

    let session = session_from_claims(claims);
    assert!(!session.subject.is_empty(), "sub must be present");
    assert!(
        session.roles.contains(&"rauthy_admin".to_string()),
        "admin user carries the rauthy_admin role"
    );

    println!(
        "VERIFIED real Rauthy token → sub={} roles={:?} groups={:?} scopes={:?}",
        session.subject, session.roles, session.groups, session.scopes
    );
}
