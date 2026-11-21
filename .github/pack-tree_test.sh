#!/usr/bin/env bash
# Offline test of pack-tree.sh; ci.yml runs it on every push.
set -u
here=$(cd "$(dirname "$0")" && pwd)
command -v gtar >/dev/null || export TAR=tar
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
fail=0
tree() { # name: a tree with siso's state and a .git, which the archive skips
  mkdir -p "$T/$1/hb/src/out/Default" "$T/$1/hb/depot_tools/.git"
  echo state > "$T/$1/hb/src/out/Default/.siso_fs_state"
  echo head > "$T/$1/hb/depot_tools/.git/HEAD"
  cd "$T/$1" || exit
}
lists() { zstd -dc tree.tar.zst | "${TAR:-gtar}" -tf - | grep -q "$1"; }

# What run 43888560258 met: tar cannot remove the directory that still holds the skipped .git.
tree whole
"$here/pack-tree.sh" hb >/dev/null 2>&1 || { echo "FAIL: a whole archive was thrown away because tar could not remove a directory"; fail=1; }
lists 'hb/src/out/Default/.siso_fs_state' || { echo "FAIL: the archive does not hold siso's state"; fail=1; }
shasum -a 256 -c tree.tar.zst.sha256 >/dev/null 2>&1 || { echo "FAIL: no checksum of the archive"; fail=1; }

tree stateless
rm hb/src/out/Default/.siso_fs_state
"$here/pack-tree.sh" hb >/dev/null 2>&1 && { echo "FAIL: tar failed and the archive has no siso state, yet it was handed on"; fail=1; }

tree clean
rm -r hb/depot_tools
"$here/pack-tree.sh" hb >/dev/null 2>&1 || { echo "FAIL: a clean pack failed"; fail=1; }
[ ! -e hb ] || { echo "FAIL: the packed tree was not removed"; fail=1; }

[ $fail -eq 0 ] && echo "pack-tree.sh: ok"
exit $fail
