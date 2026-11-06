#!/usr/bin/env bash
# Drives the BaaCore dev app with real keystrokes (System Events; needs
# Accessibility for the terminal) and screenshots each step. Keys are only
# sent while baa is frontmost, so nothing lands in another app.
#   crates/baa/ui-check.sh <out dir> [data dir to reuse]
set -uo pipefail
out=$1; mkdir -p "$out"
data=${2:-$(mktemp -d /tmp/baa-ui.XXXX)/data}
app="/Users/Shared/hb/helium-macos/build/src/out/Default/baa Dev.app"
BAA_DATA_DIR="$data" "$app/Contents/MacOS/baa" --enable-logging=stderr >"$out/log" 2>&1 &
pid=$!
sleep 8
front() { [ "$(osascript -e 'tell application "System Events" to get unix id of first process whose frontmost is true')" = "$pid" ]; }
key() {  # key <char> [command|shift|option|control …]
  osascript -e "tell application \"$app\" to activate" >/dev/null; sleep 0.5
  front || { echo "baa not frontmost; stopping"; return 1; }
  local char=$1 mods=""; shift
  for m in "$@"; do mods="${mods:+$mods, }$m down"; done
  local using=""
  [ -n "$mods" ] && using=" using {$mods}"
  osascript -e "tell application \"System Events\" to keystroke \"$char\"$using"
}
code() { osascript -e "tell application \"$app\" to activate" >/dev/null; sleep 0.5; front && osascript -e "tell application \"System Events\" to key code $1"; }
shot() { screencapture -x "$out/$1.png"; echo "shot $1"; }
step=0
while IFS= read -r line || [ -n "$line" ]; do
  set -- $line
  case $1 in
    key) shift; key "$@" || break ;;
    type) key "${line#type }" || break ;;
    return) code 36 ;;
    escape) code 53 ;;
    wait) sleep "$2" ;;
    shot) shot "$2" ;;
  esac
  sleep 0.4
done
osascript -e "tell application \"$app\" to quit" >/dev/null
for _ in $(seq 30); do kill -0 "$pid" 2>/dev/null || break; sleep 1; done
kill -0 "$pid" 2>/dev/null && { echo "did not quit"; kill -9 "$pid"; }
wait "$pid"; echo "exit=$?"
grep -E "FATAL|Check failed|panicked" "$out/log" | head -5
echo "data: $data"
