#!/usr/bin/env bash
# Takes imputnet/helium-macos release $1 onto the current branch as one plain commit:
# upstream's own change since chromium/HELIUM_RELEASE, applied 3-way under
# chromium/helium and chromium/helium-macos, so baa's edits there survive. A conflict
# with one of them is left in the index and stops the take before its commit. A plain
# commit (not a subtree merge) rebases onto a newer main.
# helium-macos pins Helium as its helium-chromium submodule; baa vendors Helium
# itself, so that gitlink and .gitmodules are left out.
set -eu
NEW=$1
OLD=$(cat chromium/HELIUM_RELEASE)
pin() { gh api "repos/imputnet/helium-macos/contents/helium-chromium?ref=$1" -q .sha; }

take() { # <repo> <old ref> <new ref>
  git fetch -q --no-tags "https://github.com/imputnet/$1.git" "+$2:refs/take/$1-old" "+$3:refs/take/$1-new"
  set -- "$1" "refs/take/$1-old" "refs/take/$1-new" -- . ':!helium-chromium' ':!.gitmodules'
  # A file baa deleted stays deleted: git apply would refuse the whole take over it.
  gone=$(git diff --name-only --diff-filter=M "${@:2}" |
    while read -r f; do [ -e "chromium/$1/$f" ] || echo ":!$f"; done)
  [ -z "$gone" ] || echo "$1: upstream changed files baa removed; not taken: $gone" >&2
  # shellcheck disable=SC2086 # one pathspec per word
  git diff --binary "${@:2}" $gone |
    git apply --3way --index --whitespace=nowarn --directory="chromium/$1/" ||
    # Conflicts stay in the index to resolve; any other failure applied nothing.
    git diff --name-only --diff-filter=U | grep -q .
}

HELIUM=$(pin "$NEW")
take helium "$(pin "$OLD")" "$HELIUM"
take helium-macos "refs/tags/$OLD" "refs/tags/$NEW"
echo "$NEW" > chromium/HELIUM_RELEASE
git add chromium/HELIUM_RELEASE
MSG="chromium: Helium $NEW (imputnet/helium $HELIUM)"
if git diff --name-only --diff-filter=U | grep -q .; then
  echo "Conflicts with baa's edits (git diff --name-only --diff-filter=U). Resolve them, then: git commit -m '$MSG'" >&2
  exit 1
fi
git commit -q -m "$MSG"
