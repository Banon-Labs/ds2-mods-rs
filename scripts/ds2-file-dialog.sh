#!/usr/bin/env bash
# Answer one of ds2-save-file's Wine file dialogs with a path, the way a player typing it would.
#
#   scripts/ds2-file-dialog.sh 'Save this character to a file' 'ds2-test.sl2'
#   scripts/ds2-file-dialog.sh 'Load a character from a save file' 'ds2 saves\new\ds2sofs0000.sl2'
#
# For a deterministic answer use scripts/frida/answer-file-dialog.js instead: it replaces the
# comdlg32 call in the game process and never shows a window. This script is for exercising the
# dialog itself, and on 2026-09-26 it also met an untitled "File does not exist" box it cannot see.
#
# Three things measured on 2026-09-26 decide how it types:
#   * The dialog needs X input focus (`xdotool windowfocus --sync`) before it takes keys;
#     compositor focus alone is not enough, and without it every synthetic key is dropped.
#   * Paths with Shift characters (`Z:\...`, `Z:/...`) were answered "Invalid character(s) in
#     path" and a lower-case relative one was not; the likely cause, not proven, is that Shift does
#     not survive a synthetic key. So a path may hold only unshifted characters -- lower case, digits,
#     space, `\ . - = ; ' , /` -- and is taken relative to the folder the dialog opens in (the
#     save-file rows open in Downloads; Wine matches names without regard to case).
#   * A typed path opens the name box's autocomplete, which takes the first Return. Return is
#     pressed again while the dialog is still up and no error box has appeared.
# A second window with the dialog's own title is the overwrite confirmation, answered Yes. The
# dialog is found by its exact title only; no other window is listed.
#
# Exit status: 0 when every window with that title has closed, 1 otherwise.
set -u
title=${1:?dialog title}
path=${2:?path or file name}
error_title='Invalid character(s) in path'

if [[ "$path" =~ [A-Z~!@#\$%^\&*\(\)_+{}|:\"\<\>?] ]]; then
    echo "refused: '$path' holds a character typed with Shift, which these dialogs never receive;"
    echo "use lower case and a path relative to the dialog's folder"
    exit 1
fi

find_window() { xdotool search --name "^$1\$" 2>/dev/null; }

focus() {
    local wid=$1 name=$2
    hyprctl repl "for _, x in ipairs(hl.get_windows()) do if x.title == \"${name}\" then hl.dispatch(hl.dsp.focus({ window = \"address:\" .. tostring(x.address) })) end end" >/dev/null 2>&1
    xdotool windowfocus --sync "$wid" 2>/dev/null
}

wid=""
for _ in $(seq 1 30); do
    wid=$(find_window "$title" | head -1)
    [ -n "$wid" ] && break
    sleep 0.2
done
[ -n "$wid" ] || { echo "no window titled '$title'"; exit 1; }

focus "$wid" "$title"
xdotool key --window "$wid" --clearmodifiers ctrl+a BackSpace
xdotool type --window "$wid" --delay 25 "$path"
sleep 0.3
xdotool key --window "$wid" Return

returns=1
for tick in $(seq 1 30); do
    sleep 0.2
    error=$(find_window "$error_title" | head -1)
    if [ -n "$error" ]; then
        focus "$error" "$error_title"
        xdotool key --window "$error" Return 2>/dev/null
        echo "dialog '$title' said '$error_title' for '$path'"
        exit 1
    fi
    current=$(find_window "$title")
    [ -z "$current" ] && { echo "dialog '$title' answered with '$path'"; exit 0; }
    for other in $current; do
        if [ "$other" != "$wid" ]; then
            focus "$other" "$title"
            xdotool key --window "$other" --clearmodifiers alt+y 2>/dev/null
            wid=$other
        fi
    done
    if [ $((tick % 5)) -eq 0 ] && [ "$returns" -lt 3 ]; then
        focus "$wid" "$title"
        xdotool key --window "$wid" Return 2>/dev/null
        returns=$((returns + 1))
    fi
done
echo "dialog '$title' is still open after answering '$path'"
exit 1
