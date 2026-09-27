#!/usr/bin/env bash
# Watch one save or load attempt end to end, for a Monitor:
#
#   bash scripts/ds2-save-watch.sh 2>&1 | python3 scripts/monitor-throttle.py 10
#
# Emits the loader log's save-path lines (the picker's choice, the swap, the export, refusals,
# writes the save redirect lets through), every size or modified-time change to a .sl2/.co2 in
# the save folder, ~/Downloads and the picker's remembered folder, and the game exiting.
# Written 2026-09-27 for an overwrite the owner reported did not happen.
shopt -s nullglob
GAME="$HOME/.local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game"
LOG="$GAME/ds2-loader.log"
SAVES="$HOME/.local/share/Steam/steamapps/compatdata/335300/pfx/drive_c/users/steamuser/AppData/Roaming/DarkSoulsII"

tail -n0 -F "$LOG" 2>/dev/null \
  | grep --line-buffered -E "ds2-save-file|ds2-save-block|ds2-save-redirect.*write=true|ds2-continue|REFUSED|panic|PANIC" \
  | grep --line-buffered -v "ds2-menu-row" \
  | sed -u 's/^/LOG /' &

declare -A seen
snap() {
  local dirs=() d f
  for d in "$SAVES"/*/; do dirs+=("$d"); done
  dirs+=("$HOME/Downloads")
  local remembered="$GAME/ds2-save-picker-dir.txt"
  if [ -f "$remembered" ]; then
    d=$(tr -d '\r' < "$remembered" | sed 's#^[Zz]:##; s#\\#/#g')
    [ -d "$d" ] && dirs+=("$d")
  fi
  for d in "${dirs[@]}"; do
    for f in "$d"/*.sl2 "$d"/*.co2 "$d"/*.sl2.* "$d"/*.co2.*; do
      [ -f "$f" ] && echo "$f|$(stat -c '%Y %s' "$f")"
    done
  done
}

while IFS='|' read -r f v; do seen["$f"]="$v"; done < <(snap)
echo "WATCHING ${#seen[@]} save files"
alive=1
while true; do
  declare -A now=()
  while IFS='|' read -r f v; do
    now["$f"]="$v"
    if [ "${seen[$f]}" != "$v" ]; then
      kind=changed; [ -z "${seen[$f]}" ] && kind=new
      echo "FILE $kind $f -> $(stat -c '%y %s bytes' "$f") sha=$(sha256sum "$f" | cut -c1-12)"
      seen["$f"]="$v"
    fi
  done < <(snap)
  for f in "${!seen[@]}"; do
    [ -z "${now[$f]}" ] && { echo "FILE gone $f"; unset "seen[$f]"; }
  done
  unset now
  if pgrep -x DarkSoulsII.exe >/dev/null; then alive=1
  elif [ "$alive" = 1 ]; then echo "GAME exited"; alive=0; fi
  sleep 0.5
done
