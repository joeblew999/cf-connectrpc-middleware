#!/usr/bin/env nu
# Reproducible native full-stack server demo: boot Rauthy, mint a real user
# token, run the server, assert the middleware cases, tear down. Exercises the
# whole shared stack (tracing → rate-limit → oidc → cedar → service with the
# metrics + body-aware-cedar interceptors) — same make() the Worker uses.
#
#   nu examples/rauthy-cedar/server/serve.nu        # needs a local Docker daemon
#
# Cases asserted:
#   /healthz                      (no token)     → 200   (skip path)
#   /demo.v1.Api/Read             (no token)     → 401   (OidcLayer: AuthN)
#   /demo.v1.Api/Read             (admin token)  → 200   (CedarLayer: path allow)
#   /demo.v1.Api/Admin            (admin token)  → 200   (allow — has admin role)
#   /demo.v1.Api/Super            (admin token)  → 403   (deny — lacks superuser)
#   /demo.v1.Api/GetDoc {public}  (admin token)  → 200   (CedarInterceptor: body allow)
#   /demo.v1.Api/GetDoc {secret}  (admin token)  → 403   (CedarInterceptor: body deny)

const RP = 8088   # rauthy port
const SP = 8090   # server port
const ISS = "http://localhost:8088/auth/v1/"
const PW = "LocalDevAdminPassword123456"
def rand [n: int] { random chars --length $n }

def main [] {
  print "==> building server ..."
  ^cargo build -q -p rauthy-cedar-server

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

  print "==> minting user token ..."
  let token = (^curl -fsS -X POST $"http://localhost:($RP)/auth/v1/oidc/token"
    -d $"grant_type=password&client_id=worker-client&client_secret=($secret)&username=admin@localhost&password=($PW)&scope=openid profile groups"
    | from json | get access_token)

  print "==> starting server on :8090 ..."
  let job = (job spawn {||
    with-env { RAUTHY_ISSUER: $ISS, RAUTHY_JWKS_URL: $"http://localhost:($RP)/auth/v1/oidc/certs", PORT: ($SP | into string) } {
      ^target/debug/rauthy-cedar-server
    }
  })
  for _ in 0..20 {
    sleep 1sec
    if (do { ^curl -fsS $"http://localhost:($SP)/healthz" } | complete).exit_code == 0 { break }
  }

  def code [args: list] { (^curl -s -o /dev/null -w "%{http_code}" ...$args) }
  let cases = [
    [(code [$"http://localhost:($SP)/healthz"])                                                              "200" "healthz (no token)"]
    [(code [-X POST $"http://localhost:($SP)/demo.v1.Api/Read"])                                             "401" "Read no-token → AuthN deny"]
    [(code [-H $"Authorization: Bearer ($token)" -X POST $"http://localhost:($SP)/demo.v1.Api/Read"])        "200" "Read token → allow"]
    [(code [-H $"Authorization: Bearer ($token)" -X POST $"http://localhost:($SP)/demo.v1.Api/Admin"])       "200" "Admin admin-role → allow"]
    [(code [-H $"Authorization: Bearer ($token)" -X POST $"http://localhost:($SP)/demo.v1.Api/Super"])       "403" "Super no-superuser → deny"]
    # Body-aware: the CedarInterceptor reads doc_id from the JSON body. Connect
    # unary takes the request message as the JSON body (application/json).
    [(code [-H $"Authorization: Bearer ($token)" -H "Content-Type: application/json" -d "{\"docId\":\"public\"}" $"http://localhost:($SP)/demo.v1.Api/GetDoc"])  "200" "GetDoc(public) → body allow"]
    [(code [-H $"Authorization: Bearer ($token)" -H "Content-Type: application/json" -d "{\"docId\":\"secret\"}" $"http://localhost:($SP)/demo.v1.Api/GetDoc"])  "403" "GetDoc(secret) → body deny"]
  ]
  mut fail = 0
  for c in $cases {
    if $c.0 == $c.1 { print $"  ✓ ($c.2)  [($c.0)]" } else { print $"  ✗ ($c.2)  expected ($c.1) got ($c.0)"; $fail = $fail + 1 }
  }

  job kill $job
  do { ^docker rm -f rauthy-srv } | complete | ignore
  rm -rf $dir
  if $fail > 0 { print -e "SERVER E2E FAILED"; exit 1 }
  print "==> SERVER E2E OK — full middleware stack enforced over HTTP on real Rauthy tokens."
}
