#!/usr/bin/env nu
# Native full-stack e2e: boot Rauthy, mint a real token, run the native hyper
# server, assert the oidc→cedar middleware cases, tear down.
#   nu examples/rauthy-cedar/server/serve.nu   # needs a local Docker daemon
use ../harness.nu *

def main [] {
  print "==> building server ..."
  ^cargo build -q -p rauthy-cedar-server

  let r = (boot-rauthy)
  let token = (mint-token $r.secret)

  print "==> starting server on :8090 ..."
  let SP = 8090
  let job = (job spawn {||
    with-env { RAUTHY_ISSUER: $ISS, RAUTHY_JWKS_URL: $"http://localhost:($RP)/auth/v1/oidc/certs", PORT: ($SP | into string) } {
      ^target/debug/rauthy-cedar-server
    }
  })
  for _ in 0..20 {
    sleep 1sec
    if (do { ^curl -fsS $"http://localhost:($SP)/healthz" } | complete).exit_code == 0 { break }
  }

  let auth = $"Authorization: Bearer ($token)"
  let base = $"http://localhost:($SP)"
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

  # gRPC Health service actually answers SERVING (not just 200).
  let health_body = (^curl -s -H $CT -d "{}" -X POST $"($base)/grpc.health.v1.Health/Check")
  $fail = $fail + (assert ($health_body | str contains "SERVING") true $"Health/Check body is SERVING [($health_body)]")

  job kill $job
  cleanup
  rm -rf $r.dir
  if $fail > 0 { print -e "SERVER E2E FAILED"; exit 1 }
  print "==> SERVER E2E OK — full middleware stack enforced over HTTP on real Rauthy tokens."
}
