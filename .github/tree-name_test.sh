#!/usr/bin/env bash
# Offline test of tree-name.sh; ci.yml runs it on every push.
set -u
cd "$(dirname "$0")" || exit
export TREE=chromium-tree-xcode27.2
arts() { printf '['; printf '{"name":"%s"},' "$@" | sed 's/,$//'; printf ']'; }
fail=0
check() { # name want artifact-names...
  local name=$1 want=$2
  shift 2
  got=$(arts "$@" | ./tree-name.sh) || got="(exit $?)"
  [ "$got" = "$want" ] || { echo "FAIL $name: got '$got', want '$want'"; fail=1; }
}
T=$TREE
check "the last phase that ended, by number" "$T-phase10" "$T-phase9" "$T-phase9.sha256" "$T-phase10" "$T-phase10.sha256" "$T-phase2"
check "a finished tree before any phase's" "$T" "$T-phase4" "$T" "$T.sha256" "baa-engine-abc"
check "a run from before trees were kept per phase" "$T" "$T" "$T.sha256"
check "another Xcode's tree is not this one's" "" "chromium-tree-xcode27.20" "chromium-tree-xcode27.20-phase3" "$T.sha256"
check "no tree" "" "harness-logs"
check "no artifacts" ""
[ $fail -eq 0 ] && echo "tree-name.sh: ok"
exit $fail
