#!/usr/bin/env bash
# windows test: script + extension probe over CDP, ends with Browser.close.
W=/Users/danielhummelstad/Documents/Projects/baa/.claude/worktrees/adk
EXE="/Users/Shared/hb/helium-macos/build/src/out/Default/baa Spike.app/Contents/MacOS/baa"
dir=$(mktemp -d /tmp/baa-win.XXXX)
export BAA_SPIKE_PROFILE=Default
export BAA_SPIKE_SCRIPT=$W/crates/baa-chromium/tests/windows.txt
export BAA_SPIKE_URL=file://$W/crates/baa-chromium/spike-test.html
"$EXE" --user-data-dir="$dir/profile" --no-first-run --enable-logging=stderr --remote-debugging-port=9333 \
  --disable-features=DisableLoadExtensionCommandLineSwitch --load-extension=$W/crates/baa-chromium/test-ext >"$dir/log" 2>&1 &
pid=$!
sleep 48
echo "--- targets"; node $W/crates/baa-chromium/cdp.mjs 9333 targets
echo "--- probe"; node $W/crates/baa-chromium/cdp.mjs 9333 'probe()' sw.js
echo "--- ext tabs.create"; node $W/crates/baa-chromium/cdp.mjs 9333 'chrome.tabs.create({url:"https://example.com/?ext"}).then(t=>t.id)' sw.js
sleep 4
echo "--- probe2"; node $W/crates/baa-chromium/cdp.mjs 9333 'probe()' sw.js
node $W/crates/baa-chromium/cdp.mjs 9333 Browser.close
sleep 6
kill -0 $pid 2>/dev/null && { echo "still running"; kill -9 $pid; }
wait $pid; echo "exit=$?"
grep -n "script:\|baa:" "$dir/log" | grep -v tab_state | cut -c1-220
grep -n "FATAL\|Check failed" "$dir/log" | head
echo "log: $dir/log"
