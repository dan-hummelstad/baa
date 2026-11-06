#!/usr/bin/env bash
# run.sh <script> <secs> [switches]: scripted harness run through smoke.sh.
W=/Users/danielhummelstad/Documents/Projects/baa/.claude/worktrees/adk
export BAA_SPIKE_SCRIPT=$1
export BAA_SPIKE_URL=${BAA_SPIKE_URL:-file://$W/crates/baa-chromium/spike-test.html}
secs=$2
shift 2
exec bash "$W/crates/baa-chromium/smoke.sh" "/Users/Shared/hb/helium-macos/build/src/out/Default/baa Spike.app/Contents/MacOS/baa" "$secs" "$@"
