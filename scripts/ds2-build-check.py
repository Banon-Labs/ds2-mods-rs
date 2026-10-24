#!/usr/bin/env python3
"""Read the loaded character out of a running DARK SOULS II and compare it with what
`scripts/ds2-sl2-builds.py` wrote into that slot.

    python3 scripts/ds2-frida-up.py
    uv run --with frida python3 scripts/ds2-build-check.py --report REPORT.json --builds BUILDS.json --slot N

One read-only attach of `scripts/frida/build-check.js` (hooks nothing), then a detach. Exit 0 only
when level, the nine stats, both soul-memory fields and every equipped item id match; exit 2 when
no character is loaded yet, so a caller can poll until the world is up.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
AGENT = HERE / "frida/build-check.js"
STATS = ["vigor", "endurance", "vitality", "attunement", "strength", "dexterity", "intelligence",
         "faith", "adaptability"]


def read_live(endpoint: str) -> dict | None:
    import frida

    dev = frida.get_device_manager().add_remote_device(endpoint)
    pid = next((p.pid for p in dev.enumerate_processes() if p.name.lower() == "darksoulsii.exe"), None)
    if pid is None:
        raise SystemExit("DarkSoulsII.exe is not running")
    session = dev.attach(pid)
    lines: list[str] = []
    script = session.create_script(AGENT.read_text())
    # console.log reaches the log handler, never the "message" signal (whose default log handler
    # printed the agent's lines to stdout while `lines` stayed empty and every run exited 2).
    script.set_log_handler(lambda _level, text: lines.append(text))
    script.on("message", lambda m, _d: lines.append(str(m)))
    script.load()
    deadline = time.time() + 5
    while time.time() < deadline and not any("done" in ln for ln in lines):
        time.sleep(0.1)
    session.detach()
    for ln in lines:
        body = ln.removeprefix("[build-check] ")
        if body.startswith("{"):
            return json.loads(body)
    print("\n".join(lines), file=sys.stderr)
    return None


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--report", type=Path, required=True, help="ds2-sl2-builds.py --report output")
    ap.add_argument("--builds", type=Path, required=True, help="the builds JSON that was written")
    ap.add_argument("--slot", type=int, required=True)
    ap.add_argument("--endpoint", default="127.0.0.1:27042")
    a = ap.parse_args()
    rep = next(r for r in json.loads(a.report.read_text()) if r["slot"] == a.slot)
    build = next(b for b in json.loads(a.builds.read_text())["builds"] if b["slot"] == a.slot)
    live = read_live(a.endpoint)
    if live is None:
        return 2
    print(json.dumps(live))
    bad = []
    if live["level"] != rep["level"]:
        bad.append(f"level {live['level']} != {rep['level']}")
    want_stats = {s: int(build["stats"][s]) for s in STATS}
    if live["stats"] != want_stats:
        bad.append(f"stats {live['stats']} != {want_stats}")
    if live["soul_memory"] != [rep["soul_memory"]] * 2:
        bad.append(f"soul memory {live['soul_memory']} != {rep['soul_memory']}")
    worn = {e[0] for e in live.get("equipped") or []}
    for slot_name, e in rep["equipped"].items():
        if e["item"] not in worn:
            bad.append(f"{slot_name} {e['name']} ({e['item']}) not equipped")
    if live["hp"][0] != live["hp"][1]:
        bad.append(f"hp {live['hp'][0]}/{live['hp'][1]} not full")
    for b in bad:
        print(f"MISMATCH slot {a.slot}: {b}")
    print(f"slot {a.slot}: {'OK' if not bad else 'FAILED'}  SL {live['level']}  SM {live['soul_memory'][0]:,}  "
          f"HP {live['hp'][0]}/{live['hp'][1]}  {len(worn)} equipped")
    return 0 if not bad else 1


if __name__ == "__main__":
    raise SystemExit(main())
