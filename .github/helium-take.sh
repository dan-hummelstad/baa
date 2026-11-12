#!/usr/bin/env bash
# Takes imputnet/helium-macos release $1 onto the current branch as one plain commit:
# upstream's own change since chromium/HELIUM_RELEASE, applied 3-way under
# chromium/helium and chromium/helium-macos, so baa's edits there survive and a real
# conflict stops it. A plain commit (not a subtree merge) rebases onto a newer main.
# helium-macos pins Helium as its helium-chromium submodule; baa vendors Helium
# itself, so that gitlink and .gitmodules are left out.
set -eu
NEW=$1
OLD=$(cat chromium/HELIUM_RELEASE)
pin() { gh api "repos/imputnet/helium-macos/contents/helium-chromium?ref=$1" -q .sha; }

take() { # <repo> <old ref> <new ref>
  git fetch -q --no-tags "https://github.com/imputnet/$1.git" "+$2:refs/take/$1-old" "+$3:refs/take/$1-new"
  git diff --binary "refs/take/$1-old" "refs/take/$1-new" -- . ':!helium-chromium' ':!.gitmodules' |
    git apply --3way --index --whitespace=nowarn --directory="chromium/$1/"
}

HELIUM=$(pin "$NEW")
take helium "$(pin "$OLD")" "$HELIUM"
take helium-macos "refs/tags/$OLD" "refs/tags/$NEW"
echo "$NEW" > chromium/HELIUM_RELEASE
git add chromium/HELIUM_RELEASE
git commit -q -m "chromium: Helium $NEW (imputnet/helium $HELIUM)"
