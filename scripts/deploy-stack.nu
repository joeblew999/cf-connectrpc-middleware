#!/usr/bin/env nu
# One command to deploy the whole Rauthy + Cedar auth stack across its repos +
# Hetzner + Cloudflare. The map lives in deploy-stack.nuon; this is the runner.
#
#   mise run deploy:stack            # DRY — prints the ordered plan, no spend
#   mise run deploy:stack -- --execute
#
# DRY is the default on purpose: --execute spends real money (a Hetzner node)
# and pushes to Cloudflare. Read the dry plan first.

def banner [n: string] { print $"\n══ ($n) ══" }

def main [--execute] {
  let cfg = (open deploy-stack.nuon)
  let dom = $cfg.domain
  let mode = (if $execute { "EXECUTE" } else { "DRY" })
  print $"deploy-stack  domain=($dom)  mode=($mode)"

  # ── Phase 0: the contract ────────────────────────────────────────────────
  banner "0 · contract"
  print $"  shared secret: ($cfg.shared_secret_item)  \(fnox keychain — the bridge"
  print $"                 webhook ?key= MUST equal saasmail RAUTHY_WEBHOOK_SECRET\)"

  # ── Phase 1: server plane (Hetzner, via vm-uncloud) ──────────────────────
  banner "1 · server plane → Hetzner (vm-uncloud)"
  print $"  cd ($cfg.server.path)  &&  ($cfg.server.up)  &&  ($cfg.server.recipe)"
  print $"    → Rauthy + email bridge at id.($dom)"
  if $execute {
    cd $cfg.server.path
    ^nu -c $cfg.server.up
    ^nu -c $cfg.server.recipe
    cd $env.FILE_PWD
  }

  # ── Phase 2: verify the seam (issuer must be https) ──────────────────────
  banner "2 · verify issuer"
  let disco = $"https://id.($dom)/auth/v1/.well-known/openid-configuration"
  print $"  GET ($disco) → assert issuer starts with https://"
  if $execute {
    let iss = (do { ^curl -fsS $disco } | complete | get stdout | from json | get issuer)
    if ($iss | str starts-with "https://") {
      print $"  ✓ issuer = ($iss)"
    } else {
      print -e $"  ✗ issuer is NOT https: ($iss) — fix Caddy X-Forwarded-Proto before edge deploy"
      exit 1
    }
  }

  # ── Phase 3: edge plane (Cloudflare) ─────────────────────────────────────
  banner "3 · edge plane → Cloudflare"
  for e in $cfg.edge {
    print $"  • ($e.name)  \(($e.path)\)"
    print $"      secrets: ($e.wrangler_secrets? | default [] | str join ', ')"
    print $"      deploy:  ($e.deploy)"
    if $execute {
      if not ($e.path | path exists) { print -e $"  ✗ missing repo: ($e.path)"; continue }
      cd $e.path
      ^nu -c $e.deploy
      cd $env.FILE_PWD
    }
  }

  # ── Phase 4: the one manual gate ─────────────────────────────────────────
  banner "4 · manual (one-time, can't be scripted)"
  print $"  Onboard the sending domain at Cloudflare Email Service \(DNS records\):"
  print $"    https://dash.cloudflare.com/?to=/:account/email-service"
  print $"  Until done, CF won't send Rauthy's mail. Verify by triggering a"
  print $"  password reset and watching `uc logs bridge`."

  if (not $execute) {
    print "\n(DRY — nothing deployed. Re-run with --execute to spend + ship.)"
  }
}
