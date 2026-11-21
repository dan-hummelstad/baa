#!/usr/bin/env bash
# Offline test of engine-key.sh; ci.yml runs it on every push.
# A directory the build takes that the key leaves out would let a stale engine pass for a
# current one, so the two lists must be the same.
set -u
cd "$(dirname "$0")" || exit
fail=0
keyed=$(sed -n 's/^git ls-tree HEAD:chromium \(.*\) | git hash-object.*/\1/p' engine-key.sh | tr ' ' '\n' | sort | tr '\n' ' ')
built=$(grep 'cp -R .*/baa/chromium/' workflows/chromium-phase.yml | grep -o 'baa/chromium/[a-z-]*"' | sed 's|.*/||; s|"||' | sort | tr '\n' ' ')
[ -n "$keyed" ] && [ "$keyed" = "$built" ] || { echo "FAIL: engine-key.sh hashes '$keyed', chromium-phase.yml builds from '$built'"; fail=1; }

key=$(./engine-key.sh)
case $key in
  *[!0-9a-f]* | "") echo "FAIL: '$key' is not a key"; fail=1 ;;
esac
# ls-tree leaves out a name that is not there without failing.
for d in $keyed; do [ -d "../chromium/$d" ] || { echo "FAIL: chromium/$d is in the key and does not exist"; fail=1; }; done

[ $fail -eq 0 ] && echo "engine-key.sh: ok"
exit $fail
