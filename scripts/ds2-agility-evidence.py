#!/usr/bin/env python3
"""What agility buys, read from the game: PhysicalStatsPerLevelStatValuesParam's evasion columns per
agility index, and the i-frame windows of the player's evasion animations.

    python3 scripts/ds2-agility-evidence.py            # the per-index table
    python3 scripts/ds2-agility-evidence.py --tae      # plus every 110300 window in c000100_pl.tae

The index is trunc((3*ADP + ATT)/4), as the stats builder 0x14038d790 computes it
(docs/DS2-BUILD-MECHANICS.md section 3). The mechanism that turns the roll column into i-frames is
in the same document, under "Evasion i-frames"."""
import argparse
import importlib.util
import xml.etree.ElementTree as ET
from pathlib import Path

HERE = Path(__file__).parent
IFRAME_EVENT = 110300  # TAE event: invincible for the first `rate` of its window (0x140326240)
FULL_IFRAME_EVENT = 110100  # TAE event: invincible for its whole window, agility or not


def load_ex():
    spec = importlib.util.spec_from_file_location("ds2attacks", HERE / "ds2-attacks-extract.py")
    ex = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(ex)
    return ex


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--tae", action="store_true", help="also list the TAE i-frame windows")
    a = ap.parse_args()
    ex = load_ex()
    reg = ex.load_module("ds2regulation", "ds2-regulation.py")
    d = ex.decode_params(reg.DEFAULT_REGULATION, ex.DEFAULT_DEFS, {
        "PhysicalStatsPerLevelStatValuesParam": "PHYS_STATS_PER_LEVEL_STAT_PARAM",
        "MenuStatsParam": "MENU_STATS_PARAM"})
    rows, menu = d["PhysicalStatsPerLevelStatValuesParam"], d.get("MenuStatsParam", {})
    print("idx AGL step roll jump useItem changeEquip menuEvasion")
    for i in range(1, 100):
        r, m = rows[str(i)], menu.get(str(i), {})
        print(i, m.get("actionSpeed"), *(round(r[k], 4) for k in (
            "stepInvincibleTimeRate", "rollingInvincibleTimeRate", "jumpInvincibleTimeRate",
            "useItemSpeedScale", "changeEquipSpeedScale")), m.get("evasionInvincibleTime"))
    if a.tae:
        print("anim event start_s end_s")
        for f in sorted(ex.DEFAULT_TAE_DIR.glob("anim-*.xml")):
            for e in ET.parse(f).getroot().find("events"):
                t = int(e.find("type").text)
                if t in (IFRAME_EVENT, FULL_IFRAME_EVENT):
                    print(int(f.stem[5:]), t, e.find("startTime").text, e.find("endTime").text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
