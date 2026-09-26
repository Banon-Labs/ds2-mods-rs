#!/usr/bin/env bash
# Answer one of ds2-save-file's Wine file dialogs with a path, the way a player typing it would.
#
#   scripts/ds2-file-dialog.sh 'Save this character to a file' 'ds2-test.sl2'
#   scripts/ds2-file-dialog.sh 'Load a character from a save file' 'Z:\home\me\Downloads\x.sl2'
#
# The dialog is found by its exact title only; no other window is listed. It needs X input focus
# (`xdotool windowfocus --sync`) before it takes keys: compositor focus alone is not enough, and a
# dialog without X focus drops every synthetic key while looking active (measured 2026-09-26: two
# load dialogs ignored the typed path until windowfocus was added). A second window with the same
# title after Return is the overwrite confirmation, and it is answered Yes.
#
# Exit status: 0 when every window with that title has closed, 1 otherwise.
set -u
title=${1:?dialog title}
path=${2:?path or file name}

find_dialog() { xdotool search --name "^${title}\$" 2>/dev/null | head -1; }

focus() {
    local wid=$1
    hyprctl repl "for _, x in ipairs(hl.get_windows()) do if x.title == \"${title}\" then hl.dispatch(hl.dsp.focus({ window = \"address:\" .. tostring(x.address) })) end end" >/dev/null 2>&1
    xdotool windowfocus --sync "$wid" 2>/dev/null
}

wid=""
for _ in $(seq 1 30); do
    wid=$(find_dialog)
    [ -n "$wid" ] && break
    sleep 0.2
done
[ -n "$wid" ] || { echo "no window titled '$title'"; exit 1; }

focus "$wid"
xdotool key --window "$wid" --clearmodifiers ctrl+a BackSpace
xdotool type --window "$wid" --delay 25 "$path"
sleep 0.3
xdotool key --window "$wid" Return

for _ in $(seq 1 25); do
    sleep 0.2
    current=$(xdotool search --name "^${title}\$" 2>/dev/null)
    [ -z "$current" ] && { echo "dialog '$title' answered with '$path'"; exit 0; }
    for other in $current; do
        if [ "$other" != "$wid" ]; then
            focus "$other"
            xdotool key --window "$other" --clearmodifiers alt+y 2>/dev/null
            wid=$other
        fi
    done
done
echo "dialog '$title' is still open after answering '$path'"
exit 1
