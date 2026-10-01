#!/usr/bin/env python3
"""Print what a poison or bleed proc is, read from the regulation in memory: the events of
SpEffectAbnormalState.emevd (the SpEffect band 900000..902400, docs/DS2-SPEFFECT.md section 3)
with every instruction's words. Read-only; it writes nothing.

    python3 scripts/ds2-status-procs.py                 # 900100 (poison) and 900200 (bleed)
    python3 scripts/ds2-status-procs.py --id 900110 --id 900210
    python3 scripts/ds2-status-procs.py --all           # every event in the file

The proc ids are the executable's: applyStatusDamage (0x140145c60) applies SpEffect 900100 when the
poison gauge reaches 100 and 900200 for bleed (docs/DS2-DPS-MECHANICS.md "Status build-up per hit")."""
import argparse
import importlib.util
import sys
from pathlib import Path

HERE = Path(__file__).parent


def load(name: str, file: str):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod  # a @dataclass looks its own module up while the module runs
    spec.loader.exec_module(mod)
    return mod


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--id", type=int, action="append", dest="ids", help="event id (repeatable)")
    ap.add_argument("--all", action="store_true", help="every event in SpEffectAbnormalState.emevd")
    ap.add_argument("--file", default="SpEffectAbnormalState.emevd", help="regulation member to read")
    a = ap.parse_args()
    reg = load("ds2regulation", "ds2-regulation.py")
    emevd = load("ds2emevd", "ds2-emevd.py")
    members = reg.load(reg.DEFAULT_REGULATION, reg.REGULATION_KEY_HEX)
    ev = emevd.Emevd(Path(a.file), members[a.file])
    wanted = None if a.all else set(a.ids or [900100, 900200])
    for e in ev.events:
        if wanted is not None and e.id not in wanted:
            continue
        print(f"event {e.id}  instructions={e.instr_count}")
        for i in e.instructions:
            words = " ".join(f"{s}/{f:g}f/0x{r:x}" for s, f, r in i.words())
            print(f"  {i.bank}[{i.index}] {i.render_args()}   words: {words}")


if __name__ == "__main__":
    main()
