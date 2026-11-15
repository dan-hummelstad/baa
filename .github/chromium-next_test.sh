#!/usr/bin/env bash
# Offline test of chromium-next.sh; ci.yml runs it on every PR.
set -u
cd "$(dirname "$0")" || exit
feed() { printf '{"versions":['; printf '{"version":"%s"},' "$@" | sed 's/,$//'; printf ']}'; }
# Stands in for the network check: only the versions named in $HAVE have tarballs.
have() { case " $HAVE " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }
export -f have
export TARBALL_CHECK=have
fail=0
check() { # name want_version want_status current feed-versions...
  local name=$1 want=$2 status=$3 cur=$4
  shift 4
  got=$(feed "$@" | ./chromium-next.sh "$cur")
  rc=$?
  [ "$got" = "$want" ] && [ "$rc" = "$status" ] || { echo "FAIL $name: got '$got' ($rc), want '$want' ($status)"; fail=1; }
}

HAVE="157.0.8103.78 157.0.8103.79 158.0.8127.37" export HAVE
check "newest build of the milestone that has tarballs" 157.0.8103.79 0 157.0.8103.50 157.0.8103.80 157.0.8103.79 157.0.8103.50
check "numeric order, not text order" 157.0.8103.79 0 157.0.8103.9 157.0.8103.79 157.0.8103.9
check "nothing newer" "" 0 157.0.8103.79 157.0.8103.79 157.0.8103.50
check "never an older build" "" 0 157.0.8103.80 157.0.8103.79
check "a later milestone is reported, not taken" 157.0.8103.79 10 157.0.8103.50 158.0.8127.37 157.0.8103.79
check "later milestone and nothing to take" "" 10 157.0.8103.79 158.0.8127.37 157.0.8103.79
check "a version that is not four numbers is dropped" "" 0 157.0.8103.50 '157.0.8103.79;id' '157.0.8103.79 x' 157.0.8103.50
HAVE="" check "no tarballs yet" "" 0 157.0.8103.50 157.0.8103.79
echo '{}' | ./chromium-next.sh 157.0.8103.50 >/dev/null 2>&1 && { echo "FAIL: an empty feed must fail, not read as 'nothing newer'"; fail=1; }
exit $fail
