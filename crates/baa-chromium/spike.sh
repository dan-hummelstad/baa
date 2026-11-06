#!/usr/bin/env bash
# Builds the Rust spike and runs it inside a copy of the GN-built baa.app, in
# the foreground so Chromium's log lands here.
#   crates/baa-chromium/spike.sh [chromium switches]
#   BAA_SPIKE_URL=file://$PWD/crates/baa-chromium/spike-test.html crates/baa-chromium/spike.sh
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT=/Users/Shared/hb/helium-macos/build/src/out/Default
# Component builds find libchrome_dll.dylib through an rpath to out/Default,
# so the bundle has to live there.
APP="$OUT/baa Spike.app"
FRAMEWORK=Contents/Frameworks/BaaCore.framework

export BAA_SPIKE_URL=${BAA_SPIKE_URL:-file://$PWD/crates/baa-chromium/spike-test.html}

cargo build -p baa-chromium --bin spike
if [ ! -d "$APP/$FRAMEWORK" ] || [ "$OUT/baa.app/$FRAMEWORK" -nt "$APP/$FRAMEWORK" ]; then
  rm -rf "$APP"
  ditto "$OUT/baa.app" "$APP"
fi
cp "${CARGO_TARGET_DIR:-target}/debug/spike" "$APP/Contents/MacOS/baa"
codesign --force --sign - "$APP/Contents/MacOS/baa"
exec "$APP/Contents/MacOS/baa" "$@"
