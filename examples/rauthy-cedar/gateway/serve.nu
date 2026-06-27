#!/usr/bin/env nu
# MULTI-WORKER CF e2e — inter-Worker analog of ../worker/serve.nu. Boots the SAME
# Rauthy, runs BOTH workers in ONE `wrangler dev` (gateway primary on :8787 +
# api auxiliary via the [[services]] binding), asserts the gateway ProxyRead →
# binding → backend hop (200 w/ token, 401 propagated w/o).
#   nu examples/rauthy-cedar/gateway/serve.nu   # needs a local Docker daemon + wrangler
use ../harness.nu *

def main [] {
  cleanup
  let r = (boot-rauthy)
  let token = (mint-token $r.secret)

  # Gateway config FIRST = primary on :8787; backend (rauthy-cedar-api) runs as
  # an auxiliary worker reachable via the gateway's `API` service binding.
  print "==> starting BOTH workers via one wrangler dev on :8787 (gateway + api; wasm compile is slow) ..."
  let log = $"($r.dir)/wrangler.log"
  let gcfg = ([$env.PWD "examples/rauthy-cedar/gateway/wrangler.toml"] | path join)
  let acfg = ([$env.PWD "examples/rauthy-cedar/worker/wrangler.toml"]   | path join)
  let job = (job spawn {|| ^wrangler dev -c $gcfg -c $acfg --port $WP --ip 127.0.0.1 out+err> $log })

  # Readiness: the PRIMARY is the GATEWAY (no /healthz). Probe ProxyRead w/o a
  # token — once gateway + binding are live the backend answers 401 (real status).
  let rpc = $"http://localhost:($WP)/gateway.v1.GatewayService/ProxyRead"
  mut up = false
  for _ in 0..120 {
    sleep 2sec
    let c = (http-code [-H $CT -d "{}" -X POST $rpc])
    if ($c == "401" or $c == "200") { $up = true; break }
  }
  if not $up {
    print -e "==> workers did not come up; wrangler log tail:"
    do { open $log | lines | last 80 | str join "\n" | print -e $in } | complete | ignore
    job kill $job; cleanup; rm -rf $r.dir
    print -e "GATEWAY E2E FAILED — wrangler dev never answered ProxyRead"; exit 1
  }

  mut fail = 0

  # 1) ProxyRead WITHOUT a token → backend OidcLayer denies (401), propagated.
  $fail = $fail + (assert (http-code [-H $CT -d "{}" -X POST $rpc]) "401" "ProxyRead no-token → backend 401 propagated through gateway")

  # 2) ProxyRead WITH a valid token → gateway forwards Authorization → backend
  #    OIDC verifies + Cedar allows Read → 200, backend Session (subject) echoed.
  let auth = $"Authorization: Bearer ($token)"
  let okf = $"($r.dir)/proxyread.out"
  let ok_code = (http-code [-H $auth -H $CT -d "{}" -X POST $rpc])
  ^curl -s -H $auth -H $CT -d "{}" -X POST $rpc | save -f $okf
  let ok_body = ((do { ^cat $okf } | complete).stdout | str trim)
  $fail = $fail + (assert ($ok_code == "200" and ($ok_body | str contains "subject") and ($ok_body | str contains "upstream")) true $"ProxyRead token → 200 via service binding; backend Session echoed [($ok_code)] ($ok_body)")

  job kill $job
  cleanup
  rm -rf $r.dir
  if $fail > 0 { print -e "GATEWAY E2E FAILED"; exit 1 }
  print "==> GATEWAY E2E OK — gateway → [[services]] binding → backend; backend OIDC+Cedar enforced over real Rauthy tokens, FetcherTransport proven."
}
