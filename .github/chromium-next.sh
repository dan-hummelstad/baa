#!/usr/bin/env bash
# chromium-next.sh <current version>, with Google's version-history JSON for Chrome stable on stdin.
# Prints the newest stable build of the CURRENT milestone that is newer than <current> and whose
# source tarballs imputnet has published, or nothing. Helium's patches are frozen, so only a
# build of the milestone they were written for can be taken without rebasing them.
# Exits 10 (after printing) when Chrome stable is already on a later milestone.
set -euo pipefail
cur=$1
major=${cur%%.*}

# Both archives the build downloads (chromium/helium/downloads.ini, helium-macos/downloads.arm64.ini).
published() {
  local base="https://github.com/imputnet/chromium-tarballs/releases/download/chromium-$1/chromium-$1"
  curl -fsSIL --proto '=https' --proto-redir '=https' -o /dev/null "$base-base.tar.zst.hashes" 2>/dev/null &&
    curl -fsSIL --proto '=https' --proto-redir '=https' -o /dev/null "$base-mac-arm64.tar.zst.hashes" 2>/dev/null
}

# Versions are four dot-separated numbers; anything else in the feed is dropped, not passed on.
all=$(jq -r '.versions[].version' | grep -E '^[0-9]+(\.[0-9]+){3}$' | sort -Vu)
next=
for v in $(grep "^$major\." <<<"$all" | sort -rV); do
  [ "$v" != "$cur" ] && [ "$(printf '%s\n%s\n' "$cur" "$v" | sort -V | tail -1)" = "$v" ] || break
  if "${TARBALL_CHECK:-published}" "$v"; then
    next=$v
    break
  fi
done
echo "$next"
newest=$(tail -1 <<<"$all")
[ "${newest%%.*}" -le "$major" ] || exit 10
