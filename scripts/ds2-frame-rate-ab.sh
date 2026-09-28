#!/usr/bin/env bash
# Frame rate per launch arm, measured at the settled title screen with the player's input blocked.
#
#   scripts/ds2-frame-rate-ab.sh                       # the three frnw arms below
#   scripts/ds2-frame-rate-ab.sh 'bare:' 'hp:--no-hp-gauge'
#
# Each argument is NAME:FLAGS for scripts/ds2-run.py (always run with --no-seamless --input-harness,
# because the harness is the frame counter: its DirectInput mouse poll fires once per frame). Every
# arm is a fresh launch, so the scene is the title screen in all of them and nobody has to walk
# anywhere. The harness blocks input for the whole sample, so a hand on the mouse cannot change it.
# Prints polls/second over SAMPLE seconds, and the host load average at both ends, because host
# load was the other unexcluded cause in ds2-mods-rs-frnw.
set -u
R="$(cd "$(dirname "$0")/.." && pwd)"
G="$HOME/.local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game"
SETTLE_S=${SETTLE_S:-20}
SAMPLE=${SAMPLE:-15}
[ $# -gt 0 ] || set -- "bare:" "present:--invasion-path" "net_effects:--net-effects"

sample() { # "<epoch> <dinput-mouse polls>"
    local polls
    polls=$(SETTLE=0.2 bash "$R/scripts/ds2-harness.sh" status 2>&1 \
        | grep -o 'dinput-mouse=[0-9]*' | head -1 | cut -d= -f2)
    echo "$(date +%s.%N) ${polls:-0}"
}

for arm in "$@"; do
    name=${arm%%:*}
    flags=${arm#*:}
    echo "=== arm $name flags='$flags'"
    # shellcheck disable=SC2086
    python3 "$R/scripts/ds2-run.py" --no-seamless --input-harness $flags 2>&1 \
        | grep -E 'RUNNING|FAILED|REFUS|arxan status' | head -3
    if ! pgrep -x DarkSoulsII.exe >/dev/null; then
        echo "arm $name: game not running -- no sample"
        continue
    fi
    grep -m1 'build git' "$G/ds2-loader.log"
    sleep "$SETTLE_S"
    SETTLE=0.2 bash "$R/scripts/ds2-harness.sh" "block $(( (SAMPLE + 10) * 60 ))" | grep -m1 'block:'
    a=$(sample); l1=$(cut -d' ' -f1 /proc/loadavg)
    sleep "$SAMPLE"
    b=$(sample); l2=$(cut -d' ' -f1 /proc/loadavg)
    SETTLE=0.2 bash "$R/scripts/ds2-harness.sh" unblock >/dev/null
    awk -v a="$a" -v b="$b" -v n="$name" -v l1="$l1" -v l2="$l2" 'BEGIN {
        split(a, x, " "); split(b, y, " ");
        printf "arm %s fps=%.2f polls=%d over %.2fs load=%s..%s\n",
            n, (y[2] - x[2]) / (y[1] - x[1]), y[2] - x[2], y[1] - x[1], l1, l2 }'
done
