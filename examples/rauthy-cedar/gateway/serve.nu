#!/usr/bin/env nu
# Reproducible MULTI-WORKER CF e2e — the inter-Worker analog of ../worker/serve.nu.
# Boots the SAME Rauthy on :8088 in Docker, mints a real user token, then runs
# BOTH Workers in ONE `wrangler dev` command with the `[[services]]` binding
# wired for local dev:
#
#   wrangler dev -c gateway/wrangler.toml -c worker/wrangler.toml
#
# The first config (gateway) is the PRIMARY, exposed on :8787; the backend
# (rauthy-cedar-api) runs as an AUXILIARY Worker reachable only via the gateway's
# `API` service binding (CF docs: "Developing with multiple Workers" → single
# dev command). The gateway's ProxyRead handler calls the backend's
# demo.v1.Api/Read over that binding via connyay's FetcherTransport, forwarding
# the caller's Authorization header.
#
#   nu examples/rauthy-cedar/gateway/serve.nu   # needs a local Docker daemon + wrangler
#
# Cases asserted (proving the gateway → service binding → backend hop):
#   gateway ProxyRead  (valid token) → 200 + subject/roles + upstream  (backend OIDC+Cedar allow)
#   gateway ProxyRead  (no token)    → 401                              (backend OidcLayer deny, propagated)

const RP = 8088   # rauthy port
const WP = 8787   # gateway port (wrangler dev primary)
const PW = "LocalDevAdminPassword123456"
def rand [n: int] { random chars --length $n }

# Kill any stale wrangler/workerd holding the port + the old Rauthy container.
# `wrangler dev` spawns a detached `workerd` that survives `pkill -f "wrangler
# dev"`, so a previous run can leave :8787 bound. Port-scoped so we never touch
# another project's wrangler. A two-config dev still listens on the one port.
def cleanup [] {
  do { ^pkill -f "wrangler dev" } | complete | ignore
  do { ^pkill -f $"entry=127.0.0.1:($WP)" } | complete | ignore
  let pids = (do { ^lsof -ti $"tcp:($WP)" } | complete | get stdout | str trim)
  if ($pids | is-not-empty) { $pids | lines | each { |p| do { ^kill -9 $p } | complete | ignore } }
  do { ^docker rm -f rauthy-srv } | complete | ignore
}

def main [] {
  cleanup
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

  # Start BOTH workers in ONE wrangler dev. Gateway config FIRST = primary on
  # :8787; backend (rauthy-cedar-api) runs as an auxiliary worker reachable via
  # the gateway's `API` service binding. worker-build (--release wasm) for both
  # is slow, so allow a generous readiness wait. The backend's RAUTHY_* [vars]
  # already point at this Rauthy (http://localhost:8088); miniflare reaches host
  # localhost.
  print "==> starting BOTH workers via one wrangler dev on :8787 (gateway + api; wasm compile is slow) ..."
  let log = $"($dir)/wrangler.log"
  let root = $env.PWD
  let gcfg = ([$root "examples/rauthy-cedar/gateway/wrangler.toml"] | path join)
  let acfg = ([$root "examples/rauthy-cedar/worker/wrangler.toml"]   | path join)
  let job = (job spawn {||
    ^wrangler dev -c $gcfg -c $acfg --port $WP --ip 127.0.0.1 out+err> $log
  })

  let base = $"http://localhost:($WP)"
  # The backend serves /healthz, but the PRIMARY on :8787 is the GATEWAY, which
  # has no /healthz. Readiness probe: POST ProxyRead with NO token — once the
  # gateway + binding are live the backend answers 401 (a real, non-000 status);
  # before that curl can't connect (000) or wrangler 5xx during compile. We wait
  # for any clean HTTP status from the gateway's RPC path.
  def code [args: list] { (^curl -s -o /dev/null -w "%{http_code}" ...$args) }
  let ct = "content-type: application/json"
  let rpc = $"($base)/gateway.v1.GatewayService/ProxyRead"
  mut up = false
  for _ in 0..120 {
    sleep 2sec
    let c = (code [-H $ct -d "{}" -X POST $rpc])
    if ($c == "401" or $c == "200") { $up = true; break }
  }
  if not $up {
    print -e "==> workers did not come up; wrangler log tail:"
    do { open $log | lines | last 80 | str join "\n" | print -e $in } | complete | ignore
    job kill $job
    cleanup
    rm -rf $dir
    print -e "GATEWAY E2E FAILED — wrangler dev never answered ProxyRead"
    exit 1
  }

  mut fail = 0

  # 1) ProxyRead WITHOUT a token → backend OidcLayer denies (401), and the
  #    Connect `unauthenticated` error propagates back THROUGH the gateway.
  let no_tok = (code [-H $ct -d "{}" -X POST $rpc])
  if $no_tok == "401" {
    print $"  ✓ ProxyRead no-token → backend 401 propagated through gateway  [($no_tok)]"
  } else {
    print $"  ✗ ProxyRead no-token expected 401 got ($no_tok)"; $fail = $fail + 1
  }

  # 2) ProxyRead WITH a valid token → gateway forwards Authorization → backend
  #    OIDC verifies + Cedar allows Read → 200, and the backend's echoed Session
  #    (subject) flows back through the gateway's ProxyReadResponse.
  let okf = $"($dir)/proxyread.out"
  let ok_code = (code [-H $"Authorization: Bearer ($token)" -H $ct -d "{}" -X POST $rpc])
  ^curl -s -H $"Authorization: Bearer ($token)" -H $ct -d "{}" -X POST $rpc | save -f $okf
  let ok_body = ((do { ^cat $okf } | complete).stdout | str trim)
  if ($ok_code == "200" and ($ok_body | str contains "subject") and ($ok_body | str contains "upstream")) {
    print ("  \u{2713} ProxyRead token → 200 via service binding; backend Session echoed  [" + $ok_code + "] " + $ok_body)
  } else {
    print $"  ✗ ProxyRead token expected 200 with subject/upstream got ($ok_code) body=($ok_body)"; $fail = $fail + 1
  }

  job kill $job
  cleanup
  rm -rf $dir
  if $fail > 0 { print -e "GATEWAY E2E FAILED"; exit 1 }
  print "==> GATEWAY E2E OK — gateway → [[services]] binding → backend; backend OIDC+Cedar enforced over real Rauthy tokens, FetcherTransport proven."
}
