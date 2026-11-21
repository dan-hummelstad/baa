#!/usr/bin/env bash
# The key of the engine this checkout builds (the artifact is baa-engine-<key>): one hash over
# the five directories of chromium/ that go into the build, the ones chromium-phase.yml copies
# into its tree (engine-key_test.sh holds the two lists together). Equal keys mean the same
# baa.h ABI, patches and sources. The rest of chromium/ is not in it, so a change to
# local-build.sh or to packaging/ (which release.yml signs with from its own checkout) leaves
# the engine valid.
set -eu
cd "$(dirname "$0")/.."
git ls-tree HEAD:chromium helium helium-macos patches baa vendor | git hash-object --stdin
