#!/usr/bin/env bash
# Do chromium/'s patches apply to the pinned Chromium? Run it before starting a chromium.yml
# build: a patch that does not apply otherwise fails that build's first phase half an hour in.
# It applies them the way the build does (Helium's utils/patches.py: Helium's, helium-macos's,
# then baa's, with this Mac's `patch`), to a checkout that holds only the files the patches
# touch. No tree is needed: about two minutes the first time for a Chromium version, a minute
# after that. It says nothing about whether the patched code compiles (chromium/local-build.sh).
#   xtask/check-patches.sh
set -euo pipefail
cd "$(dirname "$0")/.."
H=chromium/helium
M=chromium/helium-macos
C=${XDG_CACHE_HOME:-$HOME/Library/Caches}/baa-check-patches
S=$C/src
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
mkdir -p "$C/downloads"

python3 $H/utils/patches.py merge "$T/patches" $H/patches $M/patches chromium/patches
grep -rh '^--- a/' "$T/patches" | sed 's|^--- a/||; s|	.*||' | sort -u > "$T/touched"

# sparse <dir> <url> <commit> <paths file>: <dir> holds exactly those files of <commit>, unpatched.
# Blobless and shallow, so only the files asked for are downloaded, in one request.
sparse() {
  if [ ! -d "$1/.git" ]; then
    git init -q "$1"
    git -C "$1" remote add origin "$2"
    git -C "$1" config remote.origin.promisor true
    git -C "$1" config remote.origin.partialclonefilter blob:none
  fi
  git -C "$1" cat-file -e "$3^{commit}" 2>/dev/null || git -C "$1" fetch -q --depth 1 --filter=blob:none origin "$3"
  # Matching the patterns against Chromium's half a million paths takes most of a minute.
  sed 's|^|/|' "$4" > "$T/patterns"
  cmp -s "$T/patterns" "$1/.git/info/sparse-checkout" || git -C "$1" sparse-checkout set --no-cone --stdin < "$T/patterns"
  git -C "$1" checkout -q --force --detach "$3"
  git -C "$1" clean -qfdx
}

# Google says which commit the version is; the files come from Chromium's GitHub mirror, because
# chromium.googlesource.com does not answer a request for this many single files (10 minutes
# without a byte, 2026-11-22). A commit id names its content, so the mirror cannot change it.
V=$(cat $H/chromium_version.txt)
rev=$(git ls-remote https://chromium.googlesource.com/chromium/src "refs/tags/$V" | cut -f1)
[ -n "$rev" ] || { echo "chromium.googlesource.com has no tag $V" >&2; exit 1; }
{ cat "$T/touched"; echo .gitmodules; } > "$T/want"
sparse "$S" https://github.com/chromium/chromium "$rev" "$T/want"

# What the build unpacks into the tree before it patches (uBlock from chromium/vendor, Sparkle).
cp chromium/vendor/* "$C/downloads/"
python3 $H/utils/downloads.py retrieve -i $M/downloads.ini $H/deps.ini -c "$C/downloads"
python3 $H/utils/downloads.py unpack -i $M/downloads.ini $H/deps.ini -c "$C/downloads" "$S"

# Touched files under one of Chromium's DEPS repositories: a gitlink in src, its URL in .gitmodules.
git -C "$S" ls-tree -r "$rev" | awk '$2 == "commit" { print $3, $4 }' > "$T/links"
while read -r sub dir; do
  grep "^$dir/" "$T/touched" | sed "s|^$dir/||" > "$T/sub" || continue
  sparse "$S/$dir" "$(git config -f "$S/.gitmodules" "submodule.$dir.url")" "$sub" "$T/sub"
done < "$T/links"

python3 $H/utils/patches.py apply "$S" $H/patches $M/patches chromium/patches
echo "check-patches: every patch applies to Chromium $V"
