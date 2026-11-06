#!/usr/bin/env bash
# Builds baa on BaaCore (feature `core`) and runs it inside a copy of the
# GN-built baa.app, in the foreground so Chromium's log lands here.
#   crates/baa/core.sh [--no-run] [baa args and Chromium switches]
#   BAA_DATA_DIR=$(mktemp -d) crates/baa/core.sh https://example.com
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT=/Users/Shared/hb/helium-macos/build/src/out/Default
# Component builds find libchrome_dll.dylib through an rpath to out/Default,
# so the bundle has to live there.
APP="$OUT/baa Dev.app"
FRAMEWORK=Contents/Frameworks/BaaCore.framework

NO_RUN=
if [ "${1:-}" = --no-run ]; then NO_RUN=1; shift; fi

cargo build -p baa --features core
if [ ! -d "$APP/$FRAMEWORK" ] || [ "$OUT/baa.app/$FRAMEWORK" -nt "$APP/$FRAMEWORK" ]; then
  rm -rf "$APP"
  ditto "$OUT/baa.app" "$APP"
fi
cp "${CARGO_TARGET_DIR:-target}/debug/baa" "$APP/Contents/MacOS/baa"
# A stable identity, so the Keychain's "Always Allow" for "baa Storage Key"
# survives rebuilds (an ad-hoc signature's requirement is the binary's hash).
# BAA_SIGN_IDENTITY overrides; "-" signs ad-hoc.
IDENTITY=${BAA_SIGN_IDENTITY:-$(security find-identity -v -p codesigning | grep -m1 -o '"Apple Development: [^"]*"' | tr -d '"' || true)}
codesign --force --sign "${IDENTITY:--}" --identifier me.baa.browser "$APP"
[ -n "$NO_RUN" ] && exit 0
exec "$APP/Contents/MacOS/baa" "$@"
