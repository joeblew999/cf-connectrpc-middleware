//! JWKS-backed JWT verification — pure RustCrypto, so it builds on BOTH native
//! and `wasm32-unknown-unknown` (the Worker target).
//!
//! Rauthy publishes its signing keys at `<host>/auth/v1/oidc/certs` (JWKS). We
//! fetch that key set, then verify each inbound token's signature + standard
//! claims. VERIFIED by booting rauthy 0.35.2 (2026-06-12):
//!
//! - The issuer (`iss`) is `https://<host>/auth/v1/` — note the **`/auth/v1/`
//!   path suffix**. [`JwksVerifier::issuer`] must be that full value, not the
//!   bare host, or `iss` validation fails.
//! - `jwks_uri` from discovery = `<issuer>oidc/certs` (issuer ends in `/auth/v1/`).
//! - Rauthy serves 4 keys (RS256/RS384/RS512 + EdDSA); we pick by the token
//!   header's `kid` and verify with the alg the header names.
//!
//! ## Two fetch paths (the fetch is the caller's job, not this type's)
//!
//! Building the verifier takes already-fetched JWKS JSON via
//! [`JwksVerifier::from_jwks_json`]. The fetch itself is host-specific —
//! `worker::Fetch` on CF (with `.into_send()` before `.await`), a normal HTTP
//! client native — so it lives in the example Worker / caller, keeping this
//! crate free of an HTTP stack and buildable on wasm32. JWKS are fetched once
//! at boot; refetch only on a `kid` miss, never per request.
//!
//! ## Time
//!
//! [`JwksVerifier::verify`] takes the current unix time as an argument rather
//! than reading the clock — `std::time::SystemTime::now()` panics on
//! `wasm32-unknown-unknown`. The caller supplies it (native: `SystemTime`;
//! wasm: `js_sys::Date`). See `layer.rs::now_unix`.

use crate::claims::Claims;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde::Deserialize;

#[derive(Debug)]
pub enum JwksError {
    /// Token had no `kid`, or the `kid` isn't in the fetched key set.
    UnknownKey,
    /// Token structure / base64 / JSON was malformed.
    Malformed(&'static str),
    /// Signature did not verify against the key.
    BadSignature,
    /// `exp` is in the past.
    Expired,
    /// `iss` did not match the configured issuer.
    WrongIssuer,
    /// `aud` did not contain the configured audience.
    WrongAudience,
    /// Token header named an algorithm we don't support for that key type.
    UnsupportedAlg(String),
    /// Couldn't parse the JWKS document, or it had no usable keys.
    Jwks(String),
}

impl std::fmt::Display for JwksError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for JwksError {}

// ── JWKS wire format ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RawJwks {
    keys: Vec<RawJwk>,
}

#[derive(Deserialize)]
struct RawJwk {
    kid: Option<String>,
    kty: String,
    // RSA
    n: Option<String>,
    e: Option<String>,
    // OKP / Ed25519
    x: Option<String>,
}

#[derive(Deserialize)]
struct JwtHeader {
    alg: String,
    kid: Option<String>,
}

/// A public key ready to verify, indexed by its `kid`.
enum VerifyKey {
    Rsa(rsa::RsaPublicKey),
    Ed(ed25519_dalek::VerifyingKey),
}

/// Verifies tokens against a cached JWKS for one issuer. Build once at boot.
pub struct JwksVerifier {
    issuer: String,
    /// When set, every token's `aud` must contain this value.
    audience: Option<String>,
    keys: Vec<(String, VerifyKey)>,
}

impl JwksVerifier {
    /// Build from already-fetched JWKS JSON. `issuer` must be the full issuer
    /// string including the `/auth/v1/` path (it's checked against every
    /// token's `iss`). Unknown key types (e.g. EC) are skipped; RSA + Ed25519
    /// (Rauthy's default set) are loaded.
    pub fn from_jwks_json(
        issuer: impl Into<String>,
        audience: Option<String>,
        jwks_json: &str,
    ) -> Result<Self, JwksError> {
        let raw: RawJwks =
            serde_json::from_str(jwks_json).map_err(|e| JwksError::Jwks(e.to_string()))?;

        let mut keys = Vec::new();
        for jwk in raw.keys {
            let Some(kid) = jwk.kid else { continue };
            let key = match jwk.kty.as_str() {
                "RSA" => {
                    let n = B64
                        .decode(jwk.n.ok_or(JwksError::Malformed("rsa n missing"))?)
                        .map_err(|_| JwksError::Malformed("rsa n b64"))?;
                    let e = B64
                        .decode(jwk.e.ok_or(JwksError::Malformed("rsa e missing"))?)
                        .map_err(|_| JwksError::Malformed("rsa e b64"))?;
                    let pk = rsa::RsaPublicKey::new(
                        rsa::BigUint::from_bytes_be(&n),
                        rsa::BigUint::from_bytes_be(&e),
                    )
                    .map_err(|_| JwksError::Malformed("rsa key invalid"))?;
                    VerifyKey::Rsa(pk)
                }
                "OKP" => {
                    let x = B64
                        .decode(jwk.x.ok_or(JwksError::Malformed("okp x missing"))?)
                        .map_err(|_| JwksError::Malformed("okp x b64"))?;
                    let xb: [u8; 32] = x
                        .as_slice()
                        .try_into()
                        .map_err(|_| JwksError::Malformed("okp x len"))?;
                    let vk = ed25519_dalek::VerifyingKey::from_bytes(&xb)
                        .map_err(|_| JwksError::Malformed("ed key invalid"))?;
                    VerifyKey::Ed(vk)
                }
                _ => continue,
            };
            keys.push((kid, key));
        }

        if keys.is_empty() {
            return Err(JwksError::Jwks("no usable RSA/Ed25519 keys in JWKS".into()));
        }
        Ok(Self {
            issuer: issuer.into(),
            audience,
            keys,
        })
    }

    /// Verify a bearer token's signature + `iss` + `exp` and return its claims.
    /// `now_unix` is the current time in unix seconds (caller-supplied; see
    /// module docs on why this isn't read here).
    pub fn verify(&self, token: &str, now_unix: u64) -> Result<Claims, JwksError> {
        let mut parts = token.split('.');
        let h = parts.next().ok_or(JwksError::Malformed("no header"))?;
        let p = parts.next().ok_or(JwksError::Malformed("no payload"))?;
        let s = parts.next().ok_or(JwksError::Malformed("no signature"))?;
        if parts.next().is_some() {
            return Err(JwksError::Malformed("too many segments"));
        }

        let header: JwtHeader = serde_json::from_slice(
            &B64.decode(h).map_err(|_| JwksError::Malformed("header b64"))?,
        )
        .map_err(|_| JwksError::Malformed("header json"))?;

        let kid = header.kid.ok_or(JwksError::UnknownKey)?;
        let key = self
            .keys
            .iter()
            .find(|(k, _)| *k == kid)
            .map(|(_, k)| k)
            .ok_or(JwksError::UnknownKey)?;

        let sig = B64.decode(s).map_err(|_| JwksError::Malformed("sig b64"))?;
        // The signature covers the ASCII "<header>.<payload>" exactly as sent.
        let signing_input = format!("{h}.{p}");
        verify_sig(key, &header.alg, signing_input.as_bytes(), &sig)?;

        let claims: Claims = serde_json::from_slice(
            &B64.decode(p).map_err(|_| JwksError::Malformed("payload b64"))?,
        )
        .map_err(|_| JwksError::Malformed("payload json"))?;

        if claims.iss != self.issuer {
            return Err(JwksError::WrongIssuer);
        }
        if (claims.exp as u64) <= now_unix {
            return Err(JwksError::Expired);
        }
        // `aud` is only enforced when an audience is configured (Rauthy's
        // access-token `aud` is the client_id; leave `None` to skip, e.g. when
        // the token is consumed by the same client that minted it).
        if let Some(want) = &self.audience {
            match &claims.aud {
                Some(aud) if aud.contains(want) => {}
                _ => return Err(JwksError::WrongAudience),
            }
        }
        Ok(claims)
    }
}

/// Verify `sig` over `msg` with `key`, dispatched by the token's `alg`.
fn verify_sig(key: &VerifyKey, alg: &str, msg: &[u8], sig: &[u8]) -> Result<(), JwksError> {
    use rsa::signature::Verifier as _;
    match (key, alg) {
        (VerifyKey::Rsa(pk), "RS256") => rsa::pkcs1v15::VerifyingKey::<sha2::Sha256>::new(pk.clone())
            .verify(
                msg,
                &rsa::pkcs1v15::Signature::try_from(sig).map_err(|_| JwksError::BadSignature)?,
            )
            .map_err(|_| JwksError::BadSignature),
        (VerifyKey::Rsa(pk), "RS384") => rsa::pkcs1v15::VerifyingKey::<sha2::Sha384>::new(pk.clone())
            .verify(
                msg,
                &rsa::pkcs1v15::Signature::try_from(sig).map_err(|_| JwksError::BadSignature)?,
            )
            .map_err(|_| JwksError::BadSignature),
        (VerifyKey::Rsa(pk), "RS512") => rsa::pkcs1v15::VerifyingKey::<sha2::Sha512>::new(pk.clone())
            .verify(
                msg,
                &rsa::pkcs1v15::Signature::try_from(sig).map_err(|_| JwksError::BadSignature)?,
            )
            .map_err(|_| JwksError::BadSignature),
        (VerifyKey::Ed(vk), "EdDSA") => {
            use ed25519_dalek::Verifier as _;
            let sb: [u8; 64] = sig.try_into().map_err(|_| JwksError::BadSignature)?;
            vk.verify(msg, &ed25519_dalek::Signature::from_bytes(&sb))
                .map_err(|_| JwksError::BadSignature)
        }
        (_, other) => Err(JwksError::UnsupportedAlg(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::Session;
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
    use ed25519_dalek::{Signer, SigningKey};

    const ISS: &str = "https://id.localhost/auth/v1/";

    /// Build a signed EdDSA token + the matching single-key JWKS JSON.
    fn signed_token(iss: &str, exp: u64) -> (String, String) {
        // Deterministic key (no RNG needed for from_bytes / signing).
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let x = B64.encode(sk.verifying_key().to_bytes());
        let jwks = format!(
            r#"{{"keys":[{{"kty":"OKP","crv":"Ed25519","kid":"test-kid","alg":"EdDSA","x":"{x}"}}]}}"#
        );
        let header = B64.encode(br#"{"alg":"EdDSA","kid":"test-kid"}"#);
        let payload = B64.encode(
            format!(
                r#"{{"sub":"alice","iss":"{iss}","exp":{exp},"aud":"worker-client","email":"a@b.c","roles":["admin","editor"],"groups":["eng"],"scope":"openid write"}}"#
            )
            .as_bytes(),
        );
        let signing_input = format!("{header}.{payload}");
        let sig = sk.sign(signing_input.as_bytes());
        (format!("{signing_input}.{}", B64.encode(sig.to_bytes())), jwks)
    }

    #[test]
    fn eddsa_round_trip_maps_claims_to_session() {
        let (token, jwks) = signed_token(ISS, 10_000);
        let v = JwksVerifier::from_jwks_json(ISS, None, &jwks).unwrap();
        let claims = v.verify(&token, 9_000).expect("valid token must verify");
        assert_eq!(claims.sub, "alice");

        let s = Session::from(claims);
        assert_eq!(s.roles, vec!["admin", "editor"]);
        assert_eq!(s.groups, vec!["eng"]);
        assert_eq!(s.scopes, vec!["openid", "write"]); // space-split
    }

    #[test]
    fn rejects_expired() {
        let (token, jwks) = signed_token(ISS, 10_000);
        let v = JwksVerifier::from_jwks_json(ISS, None, &jwks).unwrap();
        assert!(matches!(v.verify(&token, 10_001), Err(JwksError::Expired)));
    }

    #[test]
    fn audience_enforced_only_when_configured() {
        let (token, jwks) = signed_token(ISS, 10_000);
        // No audience configured → aud ignored, token passes.
        let v = JwksVerifier::from_jwks_json(ISS, None, &jwks).unwrap();
        assert!(v.verify(&token, 9_000).is_ok());
        // Correct audience → passes.
        let v = JwksVerifier::from_jwks_json(ISS, Some("worker-client".into()), &jwks).unwrap();
        assert!(v.verify(&token, 9_000).is_ok());
        // Wrong audience → rejected.
        let v = JwksVerifier::from_jwks_json(ISS, Some("other-client".into()), &jwks).unwrap();
        assert!(matches!(
            v.verify(&token, 9_000),
            Err(JwksError::WrongAudience)
        ));
    }

    #[test]
    fn rejects_wrong_issuer() {
        let (token, jwks) = signed_token(ISS, 10_000);
        let v = JwksVerifier::from_jwks_json("https://evil/auth/v1/", None, &jwks).unwrap();
        assert!(matches!(v.verify(&token, 9_000), Err(JwksError::WrongIssuer)));
    }

    #[test]
    fn rejects_tampered_signature() {
        let (token, jwks) = signed_token(ISS, 10_000);
        // Flip the FIRST char of the signature segment — mid-signature, so it
        // still base64-decodes to 64 bytes (avoiding the canonical trailing-bit
        // check on the last char) but the bytes differ → signature mismatch.
        let dot = token.rfind('.').unwrap();
        let mut bytes: Vec<char> = token.chars().collect();
        let i = dot + 1;
        bytes[i] = if bytes[i] == 'A' { 'B' } else { 'A' };
        let t: String = bytes.into_iter().collect();
        let v = JwksVerifier::from_jwks_json(ISS, None, &jwks).unwrap();
        assert!(matches!(v.verify(&t, 9_000), Err(JwksError::BadSignature)));
    }

    /// The real JWKS Rauthy 0.35.2 serves (captured fixture): 3 RSA + 1 Ed25519.
    /// Proves from_jwks_json handles Rauthy's actual key format end to end.
    #[test]
    fn parses_real_rauthy_jwks() {
        let jwks = include_str!("../tests/rauthy_jwks.json");
        let v = JwksVerifier::from_jwks_json(ISS, None, jwks).unwrap();
        assert_eq!(v.keys.len(), 4, "expected 3 RSA + 1 Ed25519 keys");
    }
}
