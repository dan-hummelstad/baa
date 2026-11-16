#!/usr/bin/env bash
# chromium-next.sh <current version>, with Google's version-history JSON for Chrome stable on stdin.
# Prints the newest stable build of the CURRENT milestone that is newer than <current> and that
# chromium.googlesource.com has a tag for (the build clones it), or nothing. Helium's patches are
# frozen, so only a build of the milestone they were written for can be taken without rebasing them.
# Exits 10 (after printing) when Chrome stable is already on a later milestone.
set -euo pipefail
cur=$1
major=${cur%%.*}

# What chromium/helium/utils/clone.py checks out.
tagged() {
  git ls-remote --exit-code --tags https://chromium.googlesource.com/chromium/src "refs/tags/$1" >/dev/null 2>&1
}

# Versions are four dot-separated numbers; anything else in the feed is dropped, not passed on.
all=$(jq -r '.versions[].version' | grep -E '^[0-9]+(\.[0-9]+){3}$' | sort -Vu)
next=
for v in $(grep "^$major\." <<<"$all" | sort -rV); do
  [ "$v" != "$cur" ] && [ "$(printf '%s\n%s\n' "$cur" "$v" | sort -V | tail -1)" = "$v" ] || break
  if "${TAG_CHECK:-tagged}" "$v"; then
    next=$v
    break
  fi
done
echo "$next"
newest=$(tail -1 <<<"$all")
[ "${newest%%.*}" -le "$major" ] || exit 10
