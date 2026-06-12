#!/usr/bin/env nu
# Live end-to-end: boot a real Rauthy, mint a real user token, and verify it
# through connectrpc-oidc. Proves the whole AuthN seam against the actual IdP,
# not a self-signed stand-in. Self-contained (its own throwaway Rauthy on :8088,
# separate from any vm-uncloud recipe:local instance on :8080).
#
#   nu examples/rauthy-cedar/e2e.nu
#
# Requires a local Docker daemon (OrbStack/Docker Desktop). Tears itself down.
#
# Hard-won details baked in (see commit history / Rauthy source):
#   - distroless image panics without /app/config.toml (bind-mount a minimal one)
#   - webauthn RP_ID/RP_ORIGIN/RP_NAME + ENC_KEYS + HQL_SECRET_* are mandatory
#   - a bootstrap client secret must be EXACTLY 64 chars: bootstrap checks >=64
#     but token validation uses constant_time_eq_64, so anything longer can
#     never match ("Invalid 'client_secret'").

const PORT = 8088
const ISSUER = "http://localhost:8088/auth/v1/"
const ADMIN_PW = "LocalDevAdminPassword123456"

def rand [n: int] { random chars --length $n }

def main [] {
  let dir = (mktemp -d)
  let secret = (rand 64)   # EXACTLY 64 — see header note.

  "# rauthy e2e — values via env\n" | save -f $"($dir)/config.toml"
  mkdir $"($dir)/bootstrap"
  [{
    id: "worker-client", name: "Worker", secret: { Plain: $secret },
    redirect_uris: ["http://localhost:9999/callback"], enabled: true,
    flows_enabled: ["client_credentials", "password"],
    access_token_alg: "EdDSA", id_token_alg: "EdDSA",
    auth_code_lifetime: 60, access_token_lifetime: 3600,
    scopes: ["openid", "profile", "groups", "email"],
    default_scopes: ["openid", "profile", "groups"], force_mfa: false
  }] | to json | save -f $"($dir)/bootstrap/clients.json"

  let enc = $"(rand 16)/(^openssl rand -base64 32 | str trim)"
  do { ^docker rm -f rauthy-e2e } | complete | ignore
  print "==> booting throwaway Rauthy on :8088 ..."
  (^docker run -d --name rauthy-e2e -p $"($PORT):8080"
    -v $"($dir)/config.toml:/app/config.toml" -v $"($dir)/bootstrap:/app/bootstrap"
    -e BOOTSTRAP_DIR=/app/bootstrap
    -e $"PUB_URL=localhost:($PORT)" -e LISTEN_SCHEME=http -e LISTEN_ADDRESS=0.0.0.0 -e LISTEN_PORT_HTTP=8080
    -e HIQLITE=true -e HQL_NODE_ID=1 -e $"HQL_SECRET_RAFT=(rand 48)" -e $"HQL_SECRET_API=(rand 48)"
    -e $"ENC_KEYS=($enc)" -e $"ENC_KEY_ACTIVE=($enc | split row '/' | first)"
    -e RP_ID=localhost -e $"RP_ORIGIN=http://localhost:($PORT)" -e RP_NAME=e2e
    -e BOOTSTRAP_ADMIN_EMAIL=admin@localhost -e $"BOOTSTRAP_ADMIN_PASSWORD_PLAIN=($ADMIN_PW)"
    ghcr.io/sebadob/rauthy:0.35.2) | complete | ignore

  mut up = false
  for _ in 0..30 {
    sleep 2sec
    let r = (do { ^curl -fsS $"http://localhost:($PORT)/auth/v1/.well-known/openid-configuration" } | complete)
    if $r.exit_code == 0 { $up = true; break }
  }
  if not $up { print -e "rauthy did not come up"; ^docker rm -f rauthy-e2e | ignore; exit 1 }

  print "==> minting a user token (password grant) ..."
  let body = $"grant_type=password&client_id=worker-client&client_secret=($secret)&username=admin@localhost&password=($ADMIN_PW)&scope=openid profile groups"
  let tok = (^curl -fsS -X POST $"http://localhost:($PORT)/auth/v1/oidc/token" -d $body | from json | get access_token)
  $tok | save -f $"($dir)/token.txt"
  ^curl -fsS $"http://localhost:($PORT)/auth/v1/oidc/certs" | save -f $"($dir)/jwks.json"

  print "==> running the full oidc→cedar demo on the REAL token ..."
  let res = (with-env {
    RAUTHY_TOKEN_FILE: $"($dir)/token.txt",
    RAUTHY_JWKS_FILE: $"($dir)/jwks.json",
    RAUTHY_ISSUER: $ISSUER
  } { do { ^cargo run -q -p rauthy-cedar-demo } | complete })

  print $res.stdout
  do { ^docker rm -f rauthy-e2e } | complete | ignore
  rm -rf $dir
  if $res.exit_code != 0 { print -e "E2E FAILED"; exit 1 }
  print "==> E2E OK — real Rauthy token verified end to end."
}
