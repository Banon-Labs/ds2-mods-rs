#!/usr/bin/env bash
# Send ds2-input-harness commands to a running game, one at a time, and print what each logged.
#
#   scripts/ds2-harness.sh 'buttons 0x0010 6' 'status'
#   SETTLE=1.5 scripts/ds2-harness.sh 'buttons 0x0200 4'
#
# The harness runs a command when the sequence number on the file's first line changes, and a
# second write before it has read the first replaces it ("command LOST"). So each command waits
# for a new ds2-input-harness line in ds2-loader.log before the next is written, and then SETTLE
# seconds more (default 0.6) so a button hold has finished and the game has reacted. Prints every
# log line from any crate that appeared while each command ran.
set -u
G="$HOME/.local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game"
LOG="$G/ds2-loader.log"
CMD="$G/ds2-input-harness-cmd.txt"
SETTLE=${SETTLE:-0.6}
seq=$(( $(head -1 "$CMD" 2>/dev/null | tr -cd '0-9' || echo 0) + 1 ))
[ -r "$LOG" ] || { echo "no $LOG"; exit 1; }

for command in "$@"; do
    lines=$(wc -l < "$LOG")
    harness=$(grep -c 'ds2-input-harness' "$LOG")
    printf '%s\n%s\n' "$seq" "$command" > "$CMD"
    seq=$((seq + 1))
    for _ in $(seq 1 50); do
        [ "$(grep -c 'ds2-input-harness' "$LOG")" -gt "$harness" ] && break
        sleep 0.1
    done
    sleep "$SETTLE"
    echo "> $command"
    tail -n +"$((lines + 1))" "$LOG" | sed 's/^/  /'
done
