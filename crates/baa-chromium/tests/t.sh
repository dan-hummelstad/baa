#!/usr/bin/env bash
# t.sh <script> <secs> [grep-extra]: run one harness script through smoke.sh and summarise.
W=/Users/danielhummelstad/Documents/Projects/baa/.claude/worktrees/adk
out=/tmp/bt/last.out
bash /tmp/bt/run.sh "$1" "$2" > $out 2>&1
tail -5 $out
L=$(grep -o "/tmp/baa-smoke.[A-Za-z0-9]*/log" $out)
echo "LOG=$L"
grep -n "script:\|get_state\|tab_closed\|FATAL\|Check failed\|baa: [a-z_]*" "$L" | grep -v "tab_state\|script: sleep" | cut -c1-200 | tail -${3:-40}
echo "--- titles"
grep -o 'tab=[0-9]* url=[^ ]* title="[^"]*"' "$L" | awk '!seen[$0]++' | sed -E 's/url=[^ ]* //' | tail -${4:-25}
