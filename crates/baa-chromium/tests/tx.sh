#!/usr/bin/env bash
# tx.sh <script> <secs> [lines] [titles]: like t.sh, with the test extension loaded into the Default profile.
W=/Users/danielhummelstad/Documents/Projects/baa/.claude/worktrees/adk
out=/tmp/bt/last.out
export BAA_SPIKE_PROFILE=Default
bash /tmp/bt/run.sh "$1" "$2" --disable-features=DisableLoadExtensionCommandLineSwitch --load-extension=$W/crates/baa-chromium/test-ext > $out 2>&1
tail -5 $out
L=$(grep -o "/tmp/baa-smoke.[A-Za-z0-9]*/log" $out)
echo "LOG=$L"
grep -n "script:\|get_state\|tab_closed\|FATAL\|Check failed\|baa: [a-z_]*" "$L" | grep -v "tab_state\|script: sleep" | cut -c1-330 | tail -${3:-40}
echo "--- titles"
grep -o 'tab=[0-9]* url=[^ ]* title="[^"]*"' "$L" | awk '!seen[$0]++' | tail -${4:-25}
