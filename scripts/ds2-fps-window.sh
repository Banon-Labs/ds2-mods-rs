#!/bin/bash
# Measure DS2 frame rate over a fixed window after the character is in the world.
#
# Needs MangoHud injected into the game with `MANGOHUD_CONFIGFILE=$CONF` (e.g. via Proton's
# user_settings.py). MangoHud writes its CSV only when logging stops, and a change to the watched
# config file stops it ("Stopped logging because config reloaded"). So: the config autostarts a
# log at launch, this script waits for the loader's in-world line, lets the scene settle, waits
# out the window, then rewrites the config to stop and flush the log. The window is the last
# $WINDOW seconds of the CSV, so no clock alignment between MangoHud and this script is needed.
#
# Start it before launching: it records the current loader log's inode and ignores that file, so
# the previous run's in-world line can never be mistaken for this run's.
#
# usage: ds2-fps-window.sh [settle_s=2] [window_s=5]
set -u
SETTLE=${1:-2}
WINDOW=${2:-5}
LOG="$HOME/.local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game/ds2-loader.log"
OUT=${DS2_FPS_OUT:-$HOME/DS2/fps-ab}
CONF=${DS2_FPS_CONF:-$OUT/MangoHud.conf}
MARK='ds2-hp-gauge: first number written'

mkdir -p "$OUT"
printf 'no_display\nautostart_log=1\noutput_folder=%s\n' "$OUT" > "$CONF"

start=$(date +%s)
ino0=$(stat -c %i "$LOG" 2>/dev/null)
until [ "$(stat -c %i "$LOG" 2>/dev/null)" != "$ino0" ]; do
  [ $(( $(date +%s) - start )) -gt 300 ] && { echo "no new loader log in 300s"; exit 2; }
  sleep 0.25
done
echo "new loader log at +$(( $(date +%s) - start ))s"

until grep -q "$MARK" "$LOG" 2>/dev/null; do
  [ $(( $(date +%s) - start )) -gt 600 ] && { echo "no in-world line in 600s"; exit 2; }
  sleep 0.25
done
echo "in world at +$(( $(date +%s) - start ))s; settling ${SETTLE}s, measuring ${WINDOW}s"
sleep $(( SETTLE + WINDOW ))
printf 'no_display\noutput_folder=%s\n' "$OUT" > "$CONF"
echo "log stop requested at $(date +%T)"

newest=""
for _ in $(seq 1 40); do
  newest=$(find "$OUT" -name '*.csv' -newermt "@$start" -printf '%T@ %p\n' | sort -n | tail -1 | cut -d' ' -f2-)
  [ -n "$newest" ] && break
  sleep 0.25
done
[ -z "$newest" ] && { echo "MangoHud wrote no CSV"; exit 3; }
echo "csv $newest"

# MangoHud CSV: system-info header, then a header row starting with "fps", then one row per frame
# with frametime (ms) and elapsed (ns since log start).
awk -F, -v win="$WINDOW" '
  /^fps,/ { for (i = 1; i <= NF; i++) col[$i] = i; hdr = 1; next }
  hdr && NF > 2 { n++; ft[n] = $col["frametime"]; el[n] = $col["elapsed"] }
  END {
    if (n == 0) { print "no frames"; exit 1 }
    cut = el[n] - win * 1e9; k = 0; sum = 0; worst = 0
    for (i = 1; i <= n; i++) if (el[i] >= cut) { k++; sum += ft[i]; f[k] = ft[i]; if (ft[i] > worst) worst = ft[i] }
    asort(f)
    p99 = f[int(k * 0.99) > 0 ? int(k * 0.99) : 1]
    printf "window %.1fs frames=%d avg_fps=%.1f avg_frametime=%.2fms p99_frametime=%.2fms worst=%.2fms\n",
      win, k, 1000 * k / sum, sum / k, p99, worst
  }' "$newest"
