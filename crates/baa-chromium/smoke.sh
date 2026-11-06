#!/usr/bin/env bash
# Starts a bundled app executable with a fresh profile, waits, then quits it
# the way ⌘Q does (an AppleScript quit is -[NSApp terminate:]) and reports:
# process count, exit code, fatal log lines, crash reports.
#   crates/baa-chromium/smoke.sh <app>/Contents/MacOS/<exe> <seconds> [switches]
set -uo pipefail
exe=$1 secs=$2
shift 2
app=$(cd "$(dirname "$exe")/../.." && pwd)
dir=$(mktemp -d /tmp/baa-smoke.XXXX)
"$exe" --user-data-dir="$dir/profile" --no-first-run --enable-logging=stderr "$@" >"$dir/log" 2>&1 &
pid=$!
sleep "$secs"
if ! kill -0 "$pid" 2>/dev/null; then
  wait "$pid"
  # Expected with BAA_SPIKE_QUIT_AFTER below <seconds>.
  echo "exited by itself: exit=$?"
else
  echo "processes: $(pgrep -f "$dir/profile" | wc -l | tr -d ' ')"
  # SHOT=<png>: what was on screen at that point, with the app in front (one
  # started from a background shell is not activated).
  if [ -n "${SHOT:-}" ]; then
    osascript -e "tell application \"$app\" to activate" >/dev/null
    sleep 2
    screencapture -x "$SHOT"
  fi
  echo "windows: $(osascript -e "tell application \"System Events\" to get name of every window of (first process whose unix id is $pid)" 2>&1)"
  osascript -e "tell application \"$app\" to quit" >/dev/null
  start=$(date +%s)
  for i in $(seq 30); do
    kill -0 "$pid" 2>/dev/null || break
    # A quit still going after 5 s is a hang worth a stack.
    [ "$i" = 5 ] && sample "$pid" 2 -file "$dir/quit-sample.txt" >/dev/null 2>&1 &
    sleep 1
  done
  echo "quit took $(( $(date +%s) - start )) s"
  if kill -0 "$pid" 2>/dev/null; then
    echo "did not quit in 30 s"
    kill -9 "$pid"
  fi
  wait "$pid"
  echo "exit=$?"
fi
grep -E "FATAL|Check failed|Trying to load the allocator" "$dir/log" | head -5
echo "crash reports: $(find "$dir/profile/Crashpad" -name '*.dmp' 2>/dev/null | wc -l | tr -d ' ')"
echo "log: $dir/log"
