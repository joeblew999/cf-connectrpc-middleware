#!/usr/bin/env nu
# CF WORKER e2e — edge analog of ../server/serve.nu. Boots the SAME Rauthy, mints
# a token, runs the backend (api) Worker via `wrangler dev`, asserts the SAME
# oidc→cedar cases plus worker-specific gRPC reflection, tears down.
#   nu examples/rauthy-cedar/worker/serve.nu   # needs a local Docker daemon + wrangler
use ../harness.nu *

def main [] {
  cleanup
  let r = (boot-rauthy)
  let token = (mint-token $r.secret)

  # Launch from repo ROOT with `-c` so cwd matches wrangler.toml's `[build] cwd`.
  print "==> starting worker via wrangler dev on :8787 (worker-build wasm compile is slow) ..."
  let log = $"($r.dir)/wrangler.log"
  let cfg = ([$env.PWD "examples/rauthy-cedar/worker/wrangler.toml"] | path join)
  let job = (job spawn {|| ^wrangler dev -c $cfg --port $WP --ip 127.0.0.1 out+err> $log })

  mut up = false
  for _ in 0..120 {
    sleep 2sec
    if (do { ^curl -fsS $"http://localhost:($WP)/healthz" } | complete).exit_code == 0 { $up = true; break }
  }
  if not $up {
    print -e "==> worker did not come up; wrangler log tail:"
    do { open $log | lines | last 60 | str join "\n" | print -e $in } | complete | ignore
    job kill $job; cleanup; rm -rf $r.dir
    print -e "WORKER E2E FAILED — wrangler dev never answered /healthz"; exit 1
  }

  let auth = $"Authorization: Bearer ($token)"
  let base = $"http://localhost:($WP)"
  let cases = [
    [(http-code [$"($base)/healthz"])                                                            "200" "healthz (no token)"]
    [(http-code [-H $CT -d "{}" -X POST $"($base)/grpc.health.v1.Health/Check"])                  "200" "Health/Check (no token) → public 200"]
    [(http-code [-H $CT -d "{}" -X POST $"($base)/demo.v1.Api/Read"])                             "401" "Read no-token → AuthN deny"]
    [(http-code [-H $auth -H $CT -d "{}" -X POST $"($base)/demo.v1.Api/Read"])                    "200" "Read token → allow"]
    [(http-code [-H $auth -H $CT -d "{}" -X POST $"($base)/demo.v1.Api/Admin"])                   "200" "Admin admin-role → allow"]
    [(http-code [-H $auth -H $CT -d "{}" -X POST $"($base)/demo.v1.Api/Super"])                   "403" "Super no-superuser → deny"]
    [(http-code [-H $auth -H $CT -d "{\"docId\":\"public\"}" $"($base)/demo.v1.Api/GetDoc"])      "200" "GetDoc(public) → body allow"]
    [(http-code [-H $auth -H $CT -d "{\"docId\":\"secret\"}" $"($base)/demo.v1.Api/GetDoc"])      "403" "GetDoc(secret) → body deny"]
  ]
  mut fail = 0
  for c in $cases { $fail = $fail + (assert $c.0 $c.1 $c.2) }

  # gRPC Health body actually SERVING (not just 200).
  let health_body = (^curl -s -H $CT -d "{}" -X POST $"($base)/grpc.health.v1.Health/Check")
  $fail = $fail + (assert ($health_body | str contains "SERVING") true $"Health/Check body is SERVING [($health_body)]")

  # gRPC reflection (no token) → 200 listing services. ServerReflectionInfo is a
  # bidi-STREAMING Connect RPC: the body is a Connect stream envelope (1 flag
  # byte 0x00 + 4-byte BE length, then JSON), content-type application/connect+json.
  let refl_msg = "{\"listServices\":\"*\"}"
  let n = ($refl_msg | into binary | bytes length)
  let pbytes = [0 (($n bit-shr 24) mod 256) (($n bit-shr 16) mod 256) (($n bit-shr 8) mod 256) ($n mod 256)]
  let prefix = ($pbytes | each { |b| ($b | into binary --compact | bytes at 0..0) } | bytes collect)
  let reqfile = $"($r.dir)/refl.bin"
  ($prefix ++ ($refl_msg | into binary)) | save -f $reqfile
  let refl_url = $"($base)/grpc.reflection.v1.ServerReflection/ServerReflectionInfo"
  let refl_code = (^curl -s -o /dev/null -w "%{http_code}" -H "content-type: application/connect+json" --data-binary $"@($reqfile)" -X POST $refl_url)
  let rbf = $"($r.dir)/refl.out"
  ^curl -s -H "content-type: application/connect+json" --data-binary $"@($reqfile)" -X POST $refl_url | save -f $rbf
  let refl_has = ((do { ^grep -a -c "demo.v1.Api" $rbf } | complete).stdout | str trim)
  $fail = $fail + (assert ($refl_code == "200" and $refl_has != "0" and $refl_has != "") true $"ServerReflection lists demo.v1.Api [($refl_code)]")

  job kill $job
  cleanup
  rm -rf $r.dir
  if $fail > 0 { print -e "WORKER E2E FAILED"; exit 1 }
  print "==> WORKER E2E OK — full middleware stack enforced on wrangler dev/miniflare over real Rauthy tokens."
}
