#!/usr/bin/env nu
# Cedar authorize tests for the rauthy-cedar example.
#
# Proves the Rauthy claim shape (roles / groups / scopes) drives the policy
# set correctly. Each row asserts an expected (allow|deny) for a
# (principal, action, resource, context) tuple. Mirrors the harness in
# examples/remy-sport-policies/tests/run.nu.
#
# Run from examples/rauthy-cedar/:
#   nu tests/run.nu

cd ([$env.FILE_PWD ".."] | path join)

let cases = [
  # principal  action  resource  context-scopes        expected  desc
  [alice  read   spec  []          allow  "admin reads any doc"]
  [alice  delete spec  []          allow  "admin deletes any doc (role override)"]
  [bob    read   spec  []          allow  "eng-group member reads eng doc"]
  [bob    write  spec  ["write"]   allow  "eng member with write scope writes"]
  [bob    write  spec  []          deny   "eng member WITHOUT write scope cannot write"]
  [bob    delete spec  ["write"]   deny   "non-admin cannot delete"]
  [carol  read   spec  []          deny   "design member cannot read eng doc"]
]

let combined = (mktemp --suffix .cedar)
glob policies/*.cedar | each { |f| open --raw $f } | str join "\n" | save -f $combined

mut pass = 0
mut fail = 0

for case in $cases {
  let principal = $case.0
  let action = $case.1
  let resource = $case.2
  let scopes = $case.3
  let expected = $case.4
  let desc = $case.5

  let req = {
    principal: $"User::\"($principal)\"",
    action: $"Action::\"($action)\"",
    resource: $"Doc::\"($resource)\"",
    context: { scopes: $scopes }
  }

  let req_file = (mktemp --suffix .json)
  $req | to json | save -f $req_file

  let out = (cedar authorize
    --schema rauthy.cedarschema --schema-format cedar
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
