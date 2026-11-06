#!/usr/bin/env bash
# Launches the BaaCore dev app (built by core.sh --no-run) with a fresh data
# dir, screenshots it after <seconds>, quits it like ⌘Q and reports the exit.
#   crates/baa/run-check.sh <seconds> [shot.png] [baa args]
set -uo pipefail
secs=$1 shot=${2:-}
shift; [ $# -gt 0 ] && shift
app="/Users/Shared/hb/helium-macos/build/src/out/Default/baa Dev.app"
dir=$(mktemp -d /tmp/baa-run.XXXX)
BAA_DATA_DIR="$dir/data" "$app/Contents/MacOS/baa" --enable-logging=stderr "$@" >"$dir/log" 2>&1 &
pid=$!
sleep "$secs"
if ! kill -0 "$pid" 2>/dev/null; then
  wait "$pid"; echo "exited early: $?"
else
  osascript -e "tell application \"$app\" to activate" >/dev/null; sleep 2
  [ -n "$shot" ] && screencapture -x "$shot"
  osascript -e "tell application \"$app\" to quit" >/dev/null
  for _ in $(seq 30); do kill -0 "$pid" 2>/dev/null || break; sleep 1; done
  kill -0 "$pid" 2>/dev/null && { echo "did not quit in 30 s"; sample "$pid" 2 -file "$dir/sample.txt" >/dev/null 2>&1; kill -9 "$pid"; }
  wait "$pid"; echo "exit=$?"
fi
grep -E "FATAL|Check failed|panicked" "$dir/log" | head -5
echo "crash reports: $(find "$dir/data" -name '*.dmp' 2>/dev/null | wc -l | tr -d ' ')"
echo "log: $dir/log"
