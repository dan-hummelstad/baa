#!/usr/bin/env bash
# pack-tree.sh <dir>: packs <dir> of the current directory into tree.tar.zst beside it, with
# tree.tar.zst.sha256, removing <dir> as it goes. chromium-phase.yml's hand-off.
# tar's exit status does not decide whether the hours in the tree are kept: tar also fails when
# it could not remove something it had already packed (run 43888560258 lost a 4.75 h phase to one
# directory). Then the archive decides: it has to read to its end and hold siso's state.
set -uo pipefail
TAR=${TAR:-gtar}
# --remove-files: the archive never sits beside the whole tree on disk.
# posix (pax) keeps sub-second mtimes, which siso compares; the default format drops them.
"$TAR" --format=posix --remove-files --exclude='.git' -cf - "$1" | zstd -3 -T0 -q -f -o tree.tar.zst
codes=${PIPESTATUS[*]}
if [ "$codes" != "0 0" ]; then
  if ! { zstd -dc tree.tar.zst | "$TAR" -tf - > tree.list && grep -q '/out/Default/\.siso_fs_state$' tree.list; }; then
    echo "::error::packing the tree failed (tar and zstd exited $codes) and the archive is not whole"
    exit 1
  fi
  rm tree.list
  echo "::warning::tar exited ${codes% *}, but the archive reads to its end and holds siso's state: it is handed on"
fi
shasum -a 256 tree.tar.zst > tree.tar.zst.sha256
