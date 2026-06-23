#!/usr/bin/env nu
# Bring the WHOLE Rauthy + Cedar stack up LOCALLY — no cloud, no spend — and
# drive a REAL token through every hop, then leave Rauthy's GUI up to poke.
#
#   mise run stack:local         # up + verify + leave Rauthy running
#   mise run stack:local --down  # tear it all down
#
# Needs a local Docker daemon (OrbStack / Docker Desktop) + the sibling
# ../vm-uncloud repo (the Rauthy IdP runner — the infra SSOT). This script
# lives IN cf-connectrpc-middleware now (folded in from the old `iam` repo);
# the oidc→cedar server (rauthy-cedar-server) is THIS repo's own example.

const VMU = "../vm-uncloud"
const CFM = "."
const RAUTHY = "http://localhost:8080"
const ISSUER = "http://localhost:8080/auth/v1/"
const SRV = "http://127.0.0.1:8090"
const PW = "LocalDevAdminPassword123456"

def need-repo [p: string] {
  if not ($p | path exists) { print -e $"missing sibling repo: ($p) — clone it next to cf-connectrpc-middleware/"; exit 1 }
}

def main [--down] {
  need-repo $VMU
  if $down {
    print "tearing down ..."
    do { cd $VMU; ^mise run recipe:local rauthy --down } | complete | ignore
    do { ^pkill -f rauthy-cedar-server } | complete | ignore
    print "stack down."
    return
  }
  need-repo $CFM

  # ── 1. Rauthy + email bridge (Docker), with the worker-client bootstrapped ──
  print "1/4 · Rauthy + email bridge (Docker) ..."
  do { cd $VMU; ^mise run recipe:local rauthy --down } | complete | ignore
  let up = (do { cd $VMU; ^mise run recipe:local rauthy } | complete)
  let secret = ($up.stdout ++ $up.stderr | lines
    | where ($it =~ "worker-client' secret:")
    | get 0? | default "" | parse -r "secret: (?<s>[A-Za-z0-9]+)" | get s.0? | default "")
  if ($secret | is-empty) { print -e "could not capture worker-client secret from recipe:local"; exit 1 }
  mut ready = false
  for _ in 0..30 {
    sleep 2sec
    if (do { ^curl -fsS $"($RAUTHY)/auth/v1/.well-known/openid-configuration" } | complete).exit_code == 0 { $ready = true; break }
  }
  if not $ready { print -e "Rauthy didn't come up"; exit 1 }
  print "      ✓ Rauthy up + worker-client bootstrapped"

  # ── 2. mint a REAL user token (password grant) ──────────────────────────────
  print "2/4 · minting a real user token ..."
  let body = $"grant_type=password&client_id=worker-client&client_secret=($secret)&username=admin@localhost&password=($PW)&scope=openid profile groups"
  let token = (^curl -fsS -X POST $"($RAUTHY)/auth/v1/oidc/token" -d $body | from json | get access_token)
  print "      ✓ token minted"

  # ── 3. the oidc→cedar service (the shared app, native host) ─────────────────
  print "3/4 · oidc→cedar service (the shared app) ..."
  do { cd $CFM; ^cargo build -q -p rauthy-cedar-server } | complete | ignore
  let job = (job spawn {||
    with-env { RAUTHY_ISSUER: $ISSUER, RAUTHY_JWKS_URL: $"($RAUTHY)/auth/v1/oidc/certs", RAUTHY_AUD: "worker-client", PORT: "8090" } {
      cd $CFM; ^target/debug/rauthy-cedar-server
    }
  })
  for _ in 0..20 { sleep 1sec; if (do { ^curl -fsS $"($SRV)/healthz" } | complete).exit_code == 0 { break } }

  # ── 4. drive requests through the stack ─────────────────────────────────────
  print "4/4 · driving requests through oidc → cedar ..."
  def code [args: list] { (^curl -s -o /dev/null -w "%{http_code}" ...$args) }
  let cases = [
    [(code [$"($SRV)/healthz"])                                                        "200" "healthz (no token)"]
    [(code [-X POST $"($SRV)/demo.v1.Api/Read"])                                       "401" "Read no-token → AuthN deny"]
    [(code [-H $"Authorization: Bearer ($token)" -X POST $"($SRV)/demo.v1.Api/Read"])  "200" "Read token → allow"]
    [(code [-H $"Authorization: Bearer ($token)" -X POST $"($SRV)/demo.v1.Api/Admin"]) "200" "Admin admin-role → allow"]
    [(code [-H $"Authorization: Bearer ($token)" -X POST $"($SRV)/demo.v1.Api/Super"]) "403" "Super no-superuser → deny"]
  ]
  mut fail = 0
  for c in $cases {
    if $c.0 == $c.1 { print $"      ✓ ($c.2)  [($c.0)]" } else { print $"      ✗ ($c.2)  expected ($c.1) got ($c.0)"; $fail = $fail + 1 }
  }
  job kill $job

  if $fail > 0 { print -e "\nSTACK LOCAL FAILED"; exit 1 }
  print "\n══ stack is UP and GREEN — poke it ══"
  print $"  Rauthy GUI : ($RAUTHY)/auth/v1/admin   \(admin@localhost / ($PW)\)"
  print "  add users / change roles in the GUI → re-run to watch the AuthZ decision change"
  print "  edge runtime: cd examples/rauthy-cedar/worker && wrangler dev"
  print "  down: mise run stack:local --down"
}
