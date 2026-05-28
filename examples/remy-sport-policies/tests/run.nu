#!/usr/bin/env nu
# Cedar authorize tests for the remy-sport policy set.
#
# Each row asserts an expected (allow|deny) for a (principal, action,
# resource, context) tuple. Exit non-zero on first mismatch so this
# can wire into CI without ceremony.
#
# Run from examples/remy-sport-policies/:
#   nu tests/run.nu

cd ([$env.FILE_PWD ".."] | path join)

let cases = [
  # ── Platform actions ───────────────────────────────────────────────
  [admin     MANAGE_ALL_USERS  platform  { role: "ADMIN" }      allow "admin can MANAGE_ALL_USERS"],
  [wichai    MANAGE_ALL_USERS  platform  { role: "COACH" }      deny  "coach cannot MANAGE_ALL_USERS"],
  [somchai   CREATE_EVENT      platform  { role: "ORGANIZER" }  allow "organizer can CREATE_EVENT"],
  [pim       CREATE_EVENT      platform  { role: "SPECTATOR" }  deny  "spectator cannot CREATE_EVENT"],
  [wichai    CREATE_TEAM       platform  { role: "COACH" }      allow "coach can CREATE_TEAM"],
  [thanakorn CREATE_TEAM       platform  { role: "PLAYER" }     deny  "player cannot CREATE_TEAM"],

  # ── Team actions (object-instance) ────────────────────────────────
  [wichai    MANAGE_ROSTER  asm-u16-boys  { role: "COACH", team_relations: ["HEAD_COACH"] }     allow "head coach can MANAGE_ROSTER of his team"],
  [pranom    MANAGE_ROSTER  asm-u16-boys  { role: "COACH", team_relations: ["ASSISTANT_COACH"] } allow "assistant coach can MANAGE_ROSTER"],
  [wichai    MANAGE_ROSTER  tu-u18-girls  { role: "COACH" }                                      deny  "coach with no relation to team cannot MANAGE_ROSTER"],
  [wichai    EDIT_TEAM_PROFILE asm-u16-boys { role: "COACH", team_relations: ["ASSISTANT_COACH"] } deny "assistant coach cannot EDIT_TEAM_PROFILE (head/manager only)"],
  [admin     MANAGE_ROSTER  tu-u18-girls  { role: "ADMIN" }                                      allow "admin can MANAGE_ROSTER on any team"],

  # ── Event actions (with subtype gating) ───────────────────────────
  [adisorn   ENTER_SCORES   evt-tournament-001  { role: "REFEREE", event_type: "T" }            allow "referee can ENTER_SCORES on a Tournament"],
  [adisorn   ENTER_SCORES   evt-camp-001        { role: "REFEREE", event_type: "K" }            deny  "referee CANNOT ENTER_SCORES on a Camp (subtype K not scored)"],
  [somchai   EDIT_EVENT     evt-tournament-001  { role: "ORGANIZER", event_relations: ["OWNER"] } allow "event owner can EDIT_EVENT"],
  [pranom    EDIT_EVENT     evt-tournament-001  { role: "COACH" }                               deny  "coach with no event relation cannot EDIT_EVENT"],
  [somchai   DELETE_EVENT   evt-tournament-001  { role: "ORGANIZER", event_relations: ["CO_ORGANIZER"] } deny "co-organizer CANNOT DELETE_EVENT (owner+admin only)"],

  # ── Player actions ─────────────────────────────────────────────────
  [thanakorn EDIT_PLAYER_PROFILE ply-thanakorn  { role: "PLAYER", player_relations: ["SELF"] }  allow "player can EDIT own profile"],
  [thanakorn EDIT_PLAYER_PROFILE ply-stranger   { role: "PLAYER" }                              deny  "player cannot EDIT another's profile"],
  [pim       EDIT_PLAYER_PROFILE ply-thanakorn  { role: "SPECTATOR", player_relations: ["GUARDIAN"] } allow "guardian can EDIT child's profile"],
  [wichai    EDIT_PLAYER_PROFILE ply-thanakorn  { role: "COACH", team_relations: ["HEAD_COACH"] } allow "head coach can EDIT player profile (player is on coach's team)"]
]

# Resource type per resource id.
def resource-type [id: string] {
  if ($id == "platform") { "Platform" } else {
    if ($id | str starts-with "evt-") { "Event" } else {
      if ($id | str starts-with "ply-") { "Player" } else { "Team" }
    }
  }
}

# cedar CLI takes a single --policies FILE. Concat all .cedar into a temp.
let combined = (mktemp --suffix .cedar)
glob policies/*.cedar | each { |f| open --raw $f } | str join "\n\n" | save -f $combined

mut pass = 0
mut fail = 0

for case in $cases {
  let principal = $case.0
  let action    = $case.1
  let resource  = $case.2
  let ctx       = $case.3
  let expected  = $case.4
  let desc      = $case.5

  let rtype = (resource-type $resource)
  let req = {
    principal: $"User::\"($principal)\"",
    action: $"Action::\"($action)\"",
    resource: $"($rtype)::\"($resource)\"",
    context: $ctx
  }

  let req_file = (mktemp --suffix .json)
  $req | to json | save -f $req_file

  let out = (cedar authorize
    --schema remy-sport.cedarschema --schema-format cedar
    --policies $combined
    --entities tests/entities.json
    --request-json $req_file
    | complete)

  rm -f $req_file
  let decision = (if ($out.stdout =~ "ALLOW") { "allow" } else { "deny" })

  if $decision == $expected {
    print $"  ✓ ($desc)"
    $pass = $pass + 1
  } else {
    print $"  ✗ ($desc)"
    print $"      expected=($expected)  got=($decision)"
    print ($out.stdout | lines | first 3 | str join "\n        ")
    $fail = $fail + 1
  }
}

rm -f $combined
print ""
print $"  pass=($pass)  fail=($fail)  total=(($pass + $fail))"
if $fail > 0 { exit 1 }
