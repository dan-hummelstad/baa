#!/usr/bin/env bash
# Offline test of trusted.sh; ci.yml runs it on every PR.
set -u
cd "$(dirname "$0")" || exit
R=dan-hummelstad/baa
run() { # event path head_repo
  printf '{"event":"%s","path":"%s","head_repository":{"full_name":"%s"},"repository":{"full_name":"%s"}}' "$1" "$2" "$3" "$R"
}
fail=0
check() { # want_rejected(0|1) name json
  ./trusted.sh "$R" <<<"$3"
  got=$?
  [ $((got != 0)) -eq "$1" ] || { echo "FAIL: $2 (exit $got)"; fail=1; }
}
check 0 dispatch "$(run workflow_dispatch .github/workflows/chromium.yml "$R")"
check 1 pull_request "$(run pull_request .github/workflows/chromium.yml "$R")"
check 1 pull_request_target "$(run pull_request_target .github/workflows/chromium.yml "$R")"
check 1 fork "$(run workflow_dispatch .github/workflows/chromium.yml evil/baa)"
check 1 wrong-path "$(run workflow_dispatch .github/workflows/ci.yml "$R")"
check 1 no-head-repo '{"event":"workflow_dispatch","path":".github/workflows/chromium.yml","repository":{"full_name":"dan-hummelstad/baa"}}'
check 1 empty ''
[ $fail -eq 0 ] && echo "trusted.sh: ok"
exit $fail
