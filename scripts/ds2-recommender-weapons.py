#!/usr/bin/env python3
"""Write the build recommender panel's weapon table: name, class, kind and allowed infusions.

    scripts/ds2-recommender-weapons.py --out crates/ds2-build-recommender-core/data/weapons.tsv

The table is compiled into `ds2-build-recommender-core` (`include_str!`), so regenerating it is
the whole update: rebuild and the panel lists the new rows.

The in-game panel needs three things per weapon to draw its two dropdowns and its class filter,
and nothing else: the SoulsPlanner key and display name, the weapon class, and which infusions
the weapon takes. Attack values, scaling and requirements stay out; the backend that ranks
weapons owns those, so this file stays a few kilobytes of names.

# Where each column comes from

* `key`, `name`, `infusions`: SoulsPlanner's `ds2planner.min.js`, dumped to `sp-tables.json` by
  `scripts/ds2-builds-recommend.py` (`dump_site_tables`). `infusions` is one letter per infusion
  the weapon takes, in that table's own order; `INFUSION_CODES` below is the legend, and the
  header line repeats it so the file reads on its own.
* `name` is the game's own (itemname.fmg, read from the install) where SoulsPlanner's name
  matches no weapon's in the game but its key does: the site spells the Black Flamestone Dagger
  `Black Flamestone Dagge`. The same rule as the recommender's `regulation_weapon_names`, so both
  say one name for it.
* `class`: MugenMonkey's `darkSouls2WeaponDetails[*].type`, joined by normalized name after the
  same abbreviation expansion `ds2-builds-recommend.py`'s `Data.weapon_class` does (`UGS`, `GS`,
  `GA`). Empty where MugenMonkey has no row for the weapon; the count is printed.
* `kind`: `W` weapon, `S` shield (SoulsPlanner's `isShield`), `C` catalyst (the recommender's
  `CATALYST` name pattern). The recommender's weapon ranking skips `S` and `C`; the dropdowns
  still list them so a build can name one.

`Bare_Fists` is dropped: it is SoulsPlanner's empty hand, not a weapon.

# Format

Tab-separated, UTF-8, one header line starting with `#`, then one row per weapon in
SoulsPlanner's order:

    #key	name	class	kind	infusions	N=No_Infusion M=Magic ...
    Bandit_Axe	Bandit Axe	Axe	W	NMFLDPBREU
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

TABLES = Path.home() / ".cache/soulsplanner/site-tables"
CATALYST = re.compile(r"Staff|Chime|Pyromancy_Flame|Talisman|Scepter|Branch|Bellvine|Sunlight_Blade|Rod", re.I)
ABBREV = [(re.compile(r"\bUGS\b"), "Ultra Greatsword"), (re.compile(r"\bGS\b"), "Greatsword"),
          (re.compile(r"\bGA\b"), "Greataxe")]
SKIP = {"Bare_Fists"}
# One letter per SoulsPlanner infusion key. Mundane is `U` because `M` is Magic.
INFUSION_CODES = {"No_Infusion": "N", "Magic": "M", "Fire": "F", "Lightning": "L", "Dark": "D",
                  "Poison": "P", "Bleed": "B", "Raw": "R", "Enchanted": "E", "Mundane": "U"}
HEADER = "#key\tname\tclass\tkind\tinfusions\t" + " ".join(f"{c}={k}" for k, c in INFUSION_CODES.items())


def norm(name: str) -> str:
    return re.sub(r"[^a-z0-9]", "", name.lower())


def game_names() -> dict[str, str] | None:
    """Normalized name -> the game's own name, for every ItemParam row naming a WeaponParam row
    (weapons, shields and catalysts). None when the install cannot be read."""
    import importlib.util
    import struct

    spec = importlib.util.spec_from_file_location(
        "ds2attacks", Path(__file__).parent / "ds2-attacks-extract.py")
    ex = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(ex)
    reg = ex.load_module("ds2regulation", "ds2-regulation.py")
    try:
        names = ex.item_names(reg.GAME_DIR, reg.DEFAULT_REGULATION)
        items = reg.Param("ItemParam.param",
                          reg.load(reg.DEFAULT_REGULATION, reg.REGULATION_KEY_HEX)["ItemParam.param"])
    except (OSError, SystemExit, KeyError) as e:
        print(f"the game's item names are unreadable ({e}); SoulsPlanner's names kept", file=sys.stderr)
        return None
    out = {}
    for index, item in enumerate(items.ids):
        if struct.unpack_from("<21i", items.row(index), 0)[5] != -1:
            out.setdefault(norm(names[str(item)]), names[str(item)])
    return out


def rows(sp: dict, mm: dict, game: dict[str, str] | None = None
         ) -> tuple[list[tuple[str, str, str, str, str]], list[str]]:
    by_name = {}
    for k, v in mm["darkSouls2WeaponDetails"].items():
        for pat, full in ABBREV:
            k = pat.sub(full, k)
        by_name[norm(k)] = v["type"]
    out, unclassed = [], []
    for key, w in sp["weapons"].items():
        if key in SKIP:
            continue
        name = w.get("name", key)
        if game and norm(name) not in game and norm(key) in game:
            print(f"name from the game: {name!r} -> {game[norm(key)]!r}", file=sys.stderr)
            name = game[norm(key)]
        cls = by_name.get(norm(name)) or by_name.get(norm(key)) or ""
        if not cls:
            unclassed.append(name)
        kind = "S" if w.get("isShield") else "C" if CATALYST.search(key) else "W"
        for field in (key, name, cls):
            if "\t" in field or "\n" in field:
                raise SystemExit(f"{key!r}: a tab or newline in {field!r} would break the table")
        unknown = [i for i in w["infusions"] if i not in INFUSION_CODES]
        if unknown:
            raise SystemExit(f"{key!r}: infusion {unknown} has no code in INFUSION_CODES")
        out.append((key, name, cls, kind, "".join(INFUSION_CODES[i] for i in w["infusions"])))
    return out, unclassed


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--tables", type=Path, default=TABLES,
                    help="directory holding sp-tables.json and mm-tables.json")
    ap.add_argument("--out", type=Path, help="where to write the table (default: stdout)")
    a = ap.parse_args()
    sp = json.loads((a.tables / "sp-tables.json").read_text())
    mm = json.loads((a.tables / "mm-tables.json").read_text())
    table, unclassed = rows(sp, mm, game_names())
    text = HEADER + "\n" + "".join("\t".join(r) + "\n" for r in table)
    if a.out:
        a.out.parent.mkdir(parents=True, exist_ok=True)
        a.out.write_text(text, encoding="utf-8")
    else:
        sys.stdout.write(text)
    print(f"{len(table)} weapons, {len(unclassed)} with no MugenMonkey class"
          + (f": {', '.join(unclassed[:12])}{'...' if len(unclassed) > 12 else ''}" if unclassed else ""),
          file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
