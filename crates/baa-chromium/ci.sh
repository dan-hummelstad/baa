#!/usr/bin/env bash
# Bundles the harness (crates/baa-chromium) into an engine and runs every tests/*.expect against it.
# Exits non-zero, with what differed, if any test fails.
#   crates/baa-chromium/ci.sh [--engine DIR] [--only NAME]... [--keep]
# The engine is a directory with baa.app, as for `cargo bundle`: a component dev tree
# (/Users/Shared/hb/.../out/Default) or an official `baa-engine` artifact unpacked into target/engine.
# Runs real windows on this Mac; CI runs it on a macOS runner.
set -euo pipefail
cd "$(dirname "$0")/../.."

engine=
rest=()
while [ $# -gt 0 ]; do
  case $1 in
    --engine) engine=$2; shift 2 ;;
    *) rest+=("$1"); shift ;;
  esac
done
target=${CARGO_TARGET_DIR:-$PWD/target}
for dir in "$engine" "${BAA_ENGINE:-}" /Users/Shared/hb/helium-macos/build/src/out/Default "$target/engine"; do
  if [ -n "$dir" ] && [ -d "$dir/baa.app" ]; then engine=$dir; break; fi
done
[ -d "$engine/baa.app" ] || { echo "no engine (a directory with baa.app); pass --engine DIR or set BAA_ENGINE" >&2; exit 2; }

cargo build -p baa-chromium --bin spike

framework=Contents/Frameworks/BaaCore.framework
# A component engine finds libchrome_dll.dylib through an rpath to its out directory, so its
# bundle has to live there; an official engine is relocatable.
if grep -q 'is_component_build *= *true' "$engine/args.gn" 2>/dev/null; then
  app="$engine/baa Harness.app"
  if [ ! -d "$app/$framework" ] || [ "$engine/baa.app/$framework" -nt "$app/$framework" ]; then
    rm -rf "$app"; ditto "$engine/baa.app" "$app"
  fi
else
  app="$target/ci/baa Harness.app"
  rm -rf "$app"; mkdir -p "$(dirname "$app")"; ditto "$engine/baa.app" "$app"
fi
cp "$target/debug/spike" "$app/Contents/MacOS/baa"
# "-" signs ad hoc (a runner has no identity); a stable identity keeps the Keychain's "Always Allow".
identity=${BAA_SIGN_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null | grep -m1 -o '"Apple Development: [^"]*"' | tr -d '"' || true)}
codesign --force --sign "${identity:--}" --identifier me.baa.browser "$app"

exec python3 crates/baa-chromium/ci.py --app "$app" ${rest[@]+"${rest[@]}"}
