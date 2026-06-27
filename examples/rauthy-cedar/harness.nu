#!/usr/bin/env nu
# Shared e2e harness for the three rauthy-cedar host demos (server / worker /
# gateway). Each host's serve.nu does `use ../harness.nu *` and provides ONLY
# the host-specific bits (start its host + its [code, expected, desc] cases).
# Everything below — booting Rauthy in Docker, bootstrapping the OIDC client,
# minting a real user token, the curl/assert helpers, teardown — is identical
# across hosts and lives here once.

export const RP = 8088                              # rauthy port (Docker)
export const WP = 8787                              # wrangler dev port (worker/gateway)
export const ISS = "http://localhost:8088/auth/v1/" # OIDC issuer
export const PW = "LocalDevAdminPassword123456"     # bootstrapped admin password
export const CT = "content-type: application/json"  # Connect unary content-type

export def rand [n: int] { random chars --length $n }

# curl status-only helper: returns the HTTP status code as a string. Guarded
# with `do { } | complete` because during the readiness loop the target isn't up
# yet — curl exits non-zero (e.g. 7 "couldn't connect"), and an unguarded
# external failure aborts the whole nu script. On connect failure curl's `-w`
# still prints "000", which the loop treats as not-ready.
export def http-code [args: list] {
  (do { ^curl -s -o /dev/null -w "%{http_code}" ...$args } | complete | get stdout)
}

# Boot Rauthy + bootstrap the `worker-client` OIDC client, wait until the
# discovery doc answers. Returns { secret, dir } — `dir` is a tmpdir the caller
# rm -rf's at the end. The 64-char client secret is what mint-token needs.
export def boot-rauthy [] {
  let dir = (mktemp -d)
  let secret = (rand 64)
  "# e2e\n" | save -f $"($dir)/config.toml"
  mkdir $"($dir)/bootstrap"
  [{ id: "worker-client", name: "Worker", secret: { Plain: $secret },
     redirect_uris: ["http://localhost:9999/callback"], enabled: true,
     flows_enabled: ["client_credentials","password"], access_token_alg: "EdDSA",
     id_token_alg: "EdDSA", auth_code_lifetime: 60, access_token_lifetime: 3600,
     scopes: ["openid","profile","groups","email"],
     default_scopes: ["openid","profile","groups"], force_mfa: false }
  ] | to json | save -f $"($dir)/bootstrap/clients.json"

  let enc = $"(rand 16)/(^openssl rand -base64 32 | str trim)"
  do { ^docker rm -f rauthy-srv } | complete | ignore
  print "==> booting Rauthy on :8088 ..."
  (^docker run -d --name rauthy-srv -p $"($RP):8080"
    -v $"($dir)/config.toml:/app/config.toml" -v $"($dir)/bootstrap:/app/bootstrap"
    -e BOOTSTRAP_DIR=/app/bootstrap -e $"PUB_URL=localhost:($RP)" -e LISTEN_SCHEME=http
    -e LISTEN_ADDRESS=0.0.0.0 -e LISTEN_PORT_HTTP=8080 -e HIQLITE=true -e HQL_NODE_ID=1
    -e $"HQL_SECRET_RAFT=(rand 48)" -e $"HQL_SECRET_API=(rand 48)"
    -e $"ENC_KEYS=($enc)" -e $"ENC_KEY_ACTIVE=($enc | split row '/' | first)"
    -e RP_ID=localhost -e $"RP_ORIGIN=http://localhost:($RP)" -e RP_NAME=e2e
    -e BOOTSTRAP_ADMIN_EMAIL=admin@localhost -e $"BOOTSTRAP_ADMIN_PASSWORD_PLAIN=($PW)"
    ghcr.io/sebadob/rauthy:0.35.2) | complete | ignore

  for _ in 0..30 {
    sleep 2sec
    if (do { ^curl -fsS $"http://localhost:($RP)/auth/v1/.well-known/openid-configuration" } | complete).exit_code == 0 { break }
  }
  { secret: $secret, dir: $dir }
}

# Password-grant a real admin access token for the bootstrapped client.
export def mint-token [secret: string] {
  print "==> minting user token ..."
  (^curl -fsS -X POST $"http://localhost:($RP)/auth/v1/oidc/token"
    -d $"grant_type=password&client_id=worker-client&client_secret=($secret)&username=admin@localhost&password=($PW)&scope=openid profile groups"
    | from json | get access_token)
}

# Tear down: kill the Rauthy container + any leftover wrangler/workerd holding
# the dev port. `wrangler dev` spawns a detached `workerd` that survives
# `pkill -f "wrangler dev"`, so a previous run can leave :8787 bound to a worker
# pinned to OLD JWKS (fresh tokens then 401). Port-scoped so we never touch
# another project's wrangler. No-op for the native server host (nothing on :8787).
export def cleanup [] {
  do { ^pkill -f "wrangler dev" } | complete | ignore
  do { ^pkill -f $"entry=127.0.0.1:($WP)" } | complete | ignore
  let pids = (do { ^lsof -ti $"tcp:($WP)" } | complete | get stdout | str trim)
  if ($pids | is-not-empty) { $pids | lines | each { |p| do { ^kill -9 $p } | complete | ignore } }
  do { ^docker rm -f rauthy-srv } | complete | ignore
}

# Assert one case: ✓/✗ print, returns 0 on pass / 1 on fail so the caller can
# `mut fail = ($fail + (assert ...))`. Used for the [code, expected, desc] loop
# AND the ad-hoc body assertions (SERVING / reflection / subject).
export def assert [actual: any, expected: any, desc: string] {
  if $actual == $expected {
    print $"  ✓ ($desc)  [($actual)]"; 0
  } else {
    print $"  ✗ ($desc)  expected ($expected) got ($actual)"; 1
  }
}
