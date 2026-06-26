#!/usr/bin/env nu
# Cedar authorize tests for the rauthy-cedar example.
#
# Tests the SAME policies the app serves: app/policies/demo.cedar +
# app/policies/demo.cedarschema. The model is the `demo.v1.Api` service:
# actions `demo.v1.Api.Read|Admin|Super` on resource `Api` (PATH-based authz,
# what the CedarLayer enforces) plus `demo.v1.Api.GetDoc` on resource `Doc`
# (BODY-aware authz, what the CedarInterceptor enforces by reading `doc_id`).
# Roles ride in `context.roles` (mapped from the Rauthy token). A 200 in the
# running app means Cedar allowed the action; a 403 means it denied.
#
# Run from examples/rauthy-cedar/:
#   nu tests/run.nu

cd $env.FILE_PWD
cd ..

let cases = [
  # action                 roles        resource          expected  desc
  ["demo.v1.Api.Read"   []           "Api::\"main\""     allow  "authed user can Read (any valid token)"]
  ["demo.v1.Api.Admin"  ["admin"]    "Api::\"main\""     allow  "admin role can Admin"]
  ["demo.v1.Api.Admin"  []           "Api::\"main\""     deny   "no admin role cannot Admin"]
  ["demo.v1.Api.Super"  ["admin"]    "Api::\"main\""     deny   "admin (no superuser role) cannot Super"]
  # BODY-aware: the doc_id from the request body becomes the Doc resource.
  ["demo.v1.Api.GetDoc" []           "Doc::\"public\""   allow  "GetDoc(public) → allow (body-aware)"]
  ["demo.v1.Api.GetDoc" []           "Doc::\"secret\""   deny   "GetDoc(secret) → deny (body-aware)"]
]

mut pass = 0
mut fail = 0

for case in $cases {
  let action = $case.0
  let roles = $case.1
  let resource = $case.2
  let expected = $case.3
  let desc = $case.4

  let req = {
    principal: "User::\"alice\"",
    action: $"Action::\"($action)\"",
    resource: $resource,
    context: { roles: $roles, scopes: [] }
  }

  let req_file = (mktemp --suffix .json)
  $req | to json | save -f $req_file

  let out = (cedar authorize
    --schema app/policies/demo.cedarschema --schema-format cedar
    --policies app/policies/demo.cedar
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
    print ($out.stderr | lines | first 3 | str join "\n        ")
    $fail = $fail + 1
  }
}

print ""
print $"  pass=($pass)  fail=($fail)  total=(($pass + $fail))"
if $fail > 0 { exit 1 }
