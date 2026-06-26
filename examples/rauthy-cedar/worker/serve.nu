#!/usr/bin/env nu
# Reproducible CF WORKER full-stack e2e — the edge analog of ../server/serve.nu.
# Boots the SAME Rauthy on :8088 in Docker, mints a real user token, runs the
# BACKEND (api) Worker via `wrangler dev` (miniflare) on :8787, asserts the SAME
# middleware cases over HTTP plus the worker-specific gRPC reflection, then tears
# everything down. The multi-Worker gateway shape is a separate e2e — see
# ../gateway/serve.nu (mise run example:gateway).
#
#   nu examples/rauthy-cedar/worker/serve.nu     # needs a local Docker daemon + wrangler
#
# The Worker hosts the SAME shared `app::make()` stack as the native server,
# via `worker::event(fetch)`. wrangler.toml [vars] point at the Rauthy this
# script boots (RAUTHY_ISSUER/JWKS_URL = http://localhost:8088/auth/v1/...);
# miniflare reaches host localhost.
#
# Cases asserted (mirror serve.nu, same outcomes):
#   /healthz                          (no token)     → 200      (plain-HTTP liveness, skip path)
#   grpc.health.v1.Health/Check       (no token)     → 200 + SERVING
#   /demo.v1.Api/Read                 (no token)     → 401      (OidcLayer: AuthN)
#   /demo.v1.Api/Read                 (admin token)  → 200      (CedarLayer: path allow)
#   /demo.v1.Api/Super                (admin token)  → 403      (deny — lacks superuser)
#   /demo.v1.Api/GetDoc {public}      (admin token)  → 200      (CedarInterceptor: body allow)
#   /demo.v1.Api/GetDoc {secret}      (admin token)  → 403      (CedarInterceptor: body deny)
#   grpc.reflection.v1.ServerReflection/ServerReflectionInfo (no token) → 200 listing services

const RP = 8088   # rauthy port
const WP = 8787   # worker port (wrangler dev)
const ISS = "http://localhost:8088/auth/v1/"
const PW = "LocalDevAdminPassword123456"
def rand [n: int] { random chars --length $n }

# Kill any stale wrangler/workerd holding the worker port and the old Rauthy
# container. `wrangler dev` spawns a detached `workerd` child that survives
# `pkill -f "wrangler dev"`, so a previous run can leave :8787 bound to a worker
# pinned to an OLD JWKS — fresh tokens then 401. Always start (and end) clean.
# Port-scoped (kill whatever holds :8787) so we never touch another project's
# unrelated wrangler/workerd; plus our own workerd whose argv names entry=:8787.
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

  # Start wrangler dev in the background. worker-build (--release wasm compile)
  # is slow, so allow a generous readiness wait. We launch from the repo ROOT
  # with `-c` (not `cd worker`) so the launch cwd matches what wrangler.toml's
  # `[build] cwd = "examples/rauthy-cedar/worker"` is resolved against — the SAME
  # convention the multi-Worker gateway/serve.nu uses.
  print "==> starting worker via wrangler dev on :8787 (worker-build wasm compile is slow) ..."
  let log = $"($dir)/wrangler.log"
  let cfg = ([$env.PWD "examples/rauthy-cedar/worker/wrangler.toml"] | path join)
  let job = (job spawn {||
    ^wrangler dev -c $cfg --port $WP --ip 127.0.0.1 out+err> $log
  })

  mut up = false
  for _ in 0..120 {
    sleep 2sec
    if (do { ^curl -fsS $"http://localhost:($WP)/healthz" } | complete).exit_code == 0 { $up = true; break }
  }
  if not $up {
    print -e "==> worker did not come up; wrangler log tail:"
    do { open $log | lines | last 60 | str join "\n" | print -e $in } | complete | ignore
    job kill $job
    cleanup
    rm -rf $dir
    print -e "WORKER E2E FAILED — wrangler dev never answered /healthz"
    exit 1
  }

  def code [args: list] { (^curl -s -o /dev/null -w "%{http_code}" ...$args) }
  let ct = "content-type: application/json"
  let base = $"http://localhost:($WP)"
  let cases = [
    [(code [$"($base)/healthz"])                                                                          "200" "healthz (no token)"]
    [(code [-H $ct -d "{}" -X POST $"($base)/grpc.health.v1.Health/Check"])                               "200" "Health/Check (no token) → public 200"]
    [(code [-H $ct -d "{}" -X POST $"($base)/demo.v1.Api/Read"])                                          "401" "Read no-token → AuthN deny"]
    [(code [-H $"Authorization: Bearer ($token)" -H $ct -d "{}" -X POST $"($base)/demo.v1.Api/Read"])     "200" "Read token → allow"]
    [(code [-H $"Authorization: Bearer ($token)" -H $ct -d "{}" -X POST $"($base)/demo.v1.Api/Admin"])    "200" "Admin admin-role → allow"]
    [(code [-H $"Authorization: Bearer ($token)" -H $ct -d "{}" -X POST $"($base)/demo.v1.Api/Super"])    "403" "Super no-superuser → deny"]
    [(code [-H $"Authorization: Bearer ($token)" -H "Content-Type: application/json" -d "{\"docId\":\"public\"}" $"($base)/demo.v1.Api/GetDoc"])  "200" "GetDoc(public) → body allow"]
    [(code [-H $"Authorization: Bearer ($token)" -H "Content-Type: application/json" -d "{\"docId\":\"secret\"}" $"($base)/demo.v1.Api/GetDoc"])  "403" "GetDoc(secret) → body deny"]
  ]
  mut fail = 0
  for c in $cases {
    if $c.0 == $c.1 { print $"  ✓ ($c.2)  [($c.0)]" } else { print $"  ✗ ($c.2)  expected ($c.1) got ($c.0)"; $fail = $fail + 1 }
  }

  # gRPC Health body actually SERVING (not just 200).
  let health_body = (^curl -s -H $ct -d "{}" -X POST $"($base)/grpc.health.v1.Health/Check")
  if ($health_body | str contains "SERVING") {
    print $"  ✓ Health/Check body is SERVING  [($health_body)]"
  } else {
    print $"  ✗ Health/Check body expected SERVING got ($health_body)"; $fail = $fail + 1
  }

  # gRPC reflection (no token) → 200 listing services. ServerReflectionInfo is a
  # bidi-STREAMING Connect RPC, so it is NOT a plain unary JSON POST: the body is
  # a Connect stream envelope (5-byte prefix: 1 flag byte 0x00 + 4-byte BE length,
  # then the JSON message) and the content-type is application/connect+json. The
  # response is enveloped too, but the listed service names appear verbatim in it.
  let refl_msg = "{\"listServices\":\"*\"}"
  let n = ($refl_msg | into binary | bytes length)
  # 5-byte prefix: flags byte 0x00, then big-endian u32 length.
  let pbytes = [0 (($n bit-shr 24) mod 256) (($n bit-shr 16) mod 256) (($n bit-shr 8) mod 256) ($n mod 256)]
  let prefix = ($pbytes | each { |b| ($b | into binary --compact | bytes at 0..0) } | bytes collect)
  let reqfile = $"($dir)/refl.bin"
  ($prefix ++ ($refl_msg | into binary)) | save -f $reqfile
  let refl_code = (^curl -s -o /dev/null -w "%{http_code}" -H "content-type: application/connect+json" --data-binary $"@($reqfile)" -X POST $"($base)/grpc.reflection.v1.ServerReflection/ServerReflectionInfo")
  # The Connect stream RESPONSE is also enveloped (binary 5-byte prefix), so curl
  # returns binary. Save it and grep the service names out of the embedded JSON.
  let rbf = $"($dir)/refl.out"
  ^curl -s -H "content-type: application/connect+json" --data-binary $"@($reqfile)" -X POST $"($base)/grpc.reflection.v1.ServerReflection/ServerReflectionInfo" | save -f $rbf
  let refl_has = ((do { ^grep -a -c "demo.v1.Api" $rbf } | complete).stdout | str trim)
  if ($refl_code == "200" and $refl_has != "0" and $refl_has != "") {
    print $"  ✓ ServerReflection lists demo.v1.Api  [($refl_code)]"
  } else {
    print $"  ✗ ServerReflection expected 200 listing demo.v1.Api got ($refl_code) match-count ($refl_has)"; $fail = $fail + 1
  }

  job kill $job
  cleanup
  rm -rf $dir
  if $fail > 0 { print -e "WORKER E2E FAILED"; exit 1 }
  print "==> WORKER E2E OK — full middleware stack enforced on wrangler dev/miniflare over real Rauthy tokens."
}
