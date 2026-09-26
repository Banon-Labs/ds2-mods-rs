#!/usr/bin/env bash
# Does ds2-input-harness's `block` hold across a focus change, with real pointer input?
#
# Needs a running game launched with `--input-harness --invasion-path --invasion-path-on` (the
# overlay publishes the camera yaw this reads) and in-world. It moves the real pointer across the
# game window with xdotool and clicks it -- targeted at the game's own X window -- and reads the
# harness's own `status` line for the camera yaw and the clicks-suppressed counter at each step:
#   1  unblocked, pointer motion        -> the control: yaw moves
#   3  blocked, focused, motion + click -> yaw holds, clicks-suppressed rises
#   4  blocked, focus away and back     -> the case a person reported leaking
# It changes Hyprland focus (to a kitty window and back) and moves the pointer.
set -u
G="$HOME/.local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game"
LOG="$G/ds2-loader.log"
CMD="$G/ds2-input-harness-cmd.txt"
SEQ=$(( $(head -1 "$CMD" 2>/dev/null || echo 0) + 1000 ))
WIN=""
for w in $(xdotool search --class steam_app_335300); do
  [ "$(xdotool getwindowname "$w")" = "DARK SOULS II" ] && WIN=$w
done
[ -n "$WIN" ] || { echo "no game window"; exit 1; }

send() {
  local before; before=$(grep -c 'ds2-input-harness' "$LOG")
  SEQ=$((SEQ + 1)); printf '%s\n%s\n' "$SEQ" "$1" > "$CMD"
  for _ in $(seq 1 40); do [ "$(grep -c 'ds2-input-harness' "$LOG")" -gt "$before" ] && break; sleep 0.1; done
}
status() {
  send status; sleep 0.3
  local s p
  s=$(grep 'ds2-input-harness: status: block' "$LOG" | tail -1 | sed 's/.*block-frames-left=\([0-9]*\).*yaw=\([^ ]*\).*/left=\1 yaw=\2/')
  p=$(grep 'clicks-suppressed' "$LOG" | tail -1 | sed 's/.*clicks-suppressed=/suppressed=/')
  echo "$1: $s $p"
}
focus_class() {
  hyprctl repl "for _, x in ipairs(hl.get_windows()) do if x.class == \"$1\" then hl.dispatch(hl.dsp.focus({ window = \"address:\" .. tostring(x.address) })) return \"ok\" end end return \"none\"" >/dev/null
}
wiggle() {
  for x in 700 800 900 1000 1100 1200 1300 1400; do
    xdotool mousemove --window "$WIN" "$x" 602; sleep 0.05
  done
}

focus_class steam_app_335300; sleep 0.5
xdotool mousemove --window "$WIN" 1068 602; sleep 0.3
status "0 unblocked, before"
wiggle; sleep 0.3
status "1 unblocked, after motion (control: yaw moves)"
send "block 3000"
status "2 blocked"
wiggle; xdotool click --window "$WIN" 1; sleep 0.3
status "3 blocked, focused, motion + click (yaw holds)"
focus_class kitty; sleep 0.5; focus_class steam_app_335300; sleep 0.2
wiggle; xdotool click --window "$WIN" 1; sleep 0.3
status "4 blocked, focus away and back, motion + click (yaw holds)"
send unblock
status "5 unblocked"
wiggle; sleep 0.3
status "6 unblocked, after motion (yaw moves again)"
