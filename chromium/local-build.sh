#!/usr/bin/env bash
# Bring the unpacked dev tree at /Users/Shared/hb up to this checkout's chromium/
# and build chrome incrementally. Siso keys records by absolute path and compares
# mtimes, so the tree stays where it is and unchanged files keep their mtimes.
#   chromium/local-build.sh [extra autoninja args]
set -euo pipefail
CR=$(cd "$(dirname "$0")" && pwd)
ROOT=/Users/Shared/hb
PREP=/Users/Shared/hb-new
# Outside both trees, so a re-prep never downloads the ~6 GB Chromium tarball again.
CACHE=/Users/Shared/hb-downloads
SRC=$ROOT/helium-macos/build/src
RSYNC=/opt/homebrew/bin/rsync  # the rsync CI uses

# One run at a time: two re-preps share $PREP and delete each other's half-unpacked tree.
LOCK=/Users/Shared/hb-local-build.lock
mkdir "$LOCK" 2>/dev/null || { echo "another local-build.sh is running (stale? rmdir $LOCK)" >&2; exit 1; }
trap 'rmdir "$LOCK"' EXIT
! pgrep -x siso >/dev/null || { echo "a siso build is running; wait for it" >&2; exit 1; }
[ -f "$SRC/out/Default/args.gn" ] || { echo "no dev tree at $ROOT; unpack a chromium-tree-xcode* artifact there" >&2; exit 1; }

# Any account can create a directory in /Users/Shared first; build and run only
# what is yours alone: no symlink, owned by you, not group/other writable.
mine() { # mine <dir> <top>: <dir> and each parent up to <top>
  local p=$1
  while :; do
    if [ -L "$p" ] || [ "$(stat -f %u "$p")" != "$(id -u)" ] || (( 0$(stat -f %Lp "$p") & 022 )); then
      echo "$p is a symlink, not yours, or writable by group/others; fix it before building" >&2; exit 1
    fi
    [ "$p" != "$2" ] || return 0
    p=$(dirname "$p")
  done
}
mine "$SRC/out/Default" "$ROOT"
[ ! -e "$CACHE" ] || mine "$CACHE" "$CACHE"

# By content, no times: a changed file gets a new mtime and rebuilds, the rest stay.
sync() { "$RSYNC" -rlpc --delete "$@"; }

# Helium's steps rewrite files in place and cannot be undone, so any change to the
# patch inputs means a fresh prep beside the tree, synced over it like CI does.
stale=
for d in helium helium-macos patches; do
  # Skipped: outputs (build/, generated icons, bytecode) and the empty helium-chromium/ gitlink dir.
  if [ -n "$(sync -n -i --exclude=/build --exclude=/resources/generated --exclude=__pycache__ \
    --exclude=/helium-chromium "$CR/$d/" "$ROOT/$d/")" ]; then stale=1; fi
done
if [ -n "$stale" ]; then
  echo "== chromium/ patch inputs changed: preparing a fresh tree in $PREP"
  python3 -c 'import PIL, requests' || { echo "prep needs: python3 -m pip install --user --break-system-packages pillow requests (Homebrew Python refuses a plain --user install)" >&2; exit 1; }
  rm -rf "$PREP"; mkdir -p "$PREP" "$CACHE"
  cp -R "$CR/helium" "$CR/helium-macos" "$CR/patches" "$CR/baa" "$PREP/"
  rm -rf "$PREP/helium-macos/build"; mkdir -p "$PREP/helium-macos/build"
  ln -s "$CACHE" "$PREP/helium-macos/build/download_cache"
  (cd "$PREP/helium-macos" && source devutils/shared.sh && prepare_sources true)
  for d in helium patches baa; do sync "$PREP/$d/" "$ROOT/$d/"; done
  # out/ is the build; download_cache is a link to $CACHE.
  sync --stats --exclude=/build/src/out --exclude=/build/download_cache "$PREP/helium-macos/" "$ROOT/helium-macos/"
  rm -rf "$PREP"
else
  # ponytail: //baa alone needs no prep; this is the minutes-long edit loop.
  sync "$CR/baa/" "$ROOT/baa/"
  sync -i "$CR/baa/" "$SRC/baa/"
fi

cd "$ROOT/helium-macos"
source devutils/shared.sh
# Siso regenerates build.ninja itself when a BUILD.gn or args.gn changes.
[ -f "$_out_dir/build.ninja" ] || gn_gen
cd "$_src_dir"
SISO_PATH="$_siso_path" caffeinate -i python3 "$_depot_tools_dir/autoninja.py" -C "$_out_dir" chrome "$@"
