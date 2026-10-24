#!/usr/bin/env bash
# Rank every weapon of every infusion at each Seamless bracket ceiling by the moveset rate, then
# generate the bracket builds JSON (scripts/ds2-bracket-builds.py) from the top row per level.
#
#   setsid nohup scripts/ds2-bracket-sweep.sh DIR [LEVELS] [MULE] [SPEND] > DIR/run.log 2>&1 &
#
# MULE: a save whose slot 1 the builds are written from (scripts/ds2-sl2-builds.py); its bag's
# rings are the only ones the builds wear (ds2-bracket-builds.py --held-from).
#
# Hours at --jobs 16: one infusion at one level is 1-4 minutes. Each <infusion>.json is written
# when its sweep ends; DIR/done holds the final exit status.
set -u
DIR=${1:?usage: ds2-bracket-sweep.sh DIR [LEVELS] [MULE]}
LEVELS=${2:-20,40,70,100,125,155,200,250,838}
MULE=${3:-}
SPEND=${4:-curves}  # how free points are spent (ds2-builds-recommend.py --spend): curves or ehp
cd "$(dirname "$0")/.." || exit 1
mkdir -p "$DIR"
rc=0
for inf in No_Infusion Fire Magic Lightning Dark Poison Bleed Raw Enchanted Mundane; do
  echo "=== $(date +%T) $inf"
  python3 scripts/ds2-builds-recommend.py --best-weapons "$inf" --sweep "$LEVELS" --rank moveset --top 5 \
    --spend "$SPEND" --json --jobs 16 > "$DIR/$inf.json" 2> "$DIR/$inf.err" || { echo "FAILED $inf"; rc=1; }
done
echo "=== $(date +%T) bracket builds"
python3 scripts/ds2-bracket-builds.py --best-dir "$DIR" --levels "$LEVELS" -o "$DIR/builds.json" --spend "$SPEND" \
  ${MULE:+--held-from "$MULE" --base-slot 1} || rc=1
echo "$rc" > "$DIR/done"
echo "=== $(date +%T) done rc=$rc"
