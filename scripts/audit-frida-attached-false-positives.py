#!/usr/bin/env python3
"""Replay real past turns through the Frida-attached Stop guard and list every turn it would hold.

`DS2-MODS-NO-STOP-WHILE-FRIDA-ATTACHED` is judged on two halves: the transcript (this session ran the
watcher, the closing message hands nothing to the user) and the machine (the game and that watcher
are alive). A replay cannot know what was alive at a past turn end, so it pins the machine half to
"alive, running this session's agent" and reports every turn the transcript half would have held,
with the closing words and what the user wrote next. That is the upper bound on false positives: a
real halt also needs the game up with the watcher attached at that moment. Read the hits; each is
either the defect (the user had to say keep going, or should have had to) or a shape to narrow away.

Usage:
    python3 scripts/audit-frida-attached-false-positives.py [transcript.jsonl ...]

With no arguments it reads every transcript of this repository under ~/.claude/projects/ that ran
the watcher. Read-only.
"""
from __future__ import annotations

import glob
import json
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import cupcake_frida_attached as guard  # noqa: E402
import cupcake_turn_scan as ts  # noqa: E402

PROJECT_KEY = "-home-banon-projects-ds2-mods-rs"


def default_transcripts() -> list[str]:
    tdir = Path(os.path.expanduser("~/.claude/projects")) / PROJECT_KEY
    return sorted(glob.glob(str(tdir / "*.jsonl")), key=os.path.getmtime, reverse=True)


def load(path: str) -> list[dict]:
    return ts.load_events(path)


def main() -> int:
    paths = sys.argv[1:] or default_transcripts()
    total = judged = halts = 0
    for path in paths:
        events = load(path)
        if not any(guard.watcher_runs(guard._bash_command(b)) for ev in events for b in guard._tool_uses(ev)):
            continue
        bounds = [i for i, ev in enumerate(events) if ts.is_real_user_prompt(ev)] + [len(events)]
        for bound in bounds:
            total += 1
            chunk = events[:bound]
            facts = guard.scan(chunk)
            if facts is None:
                continue
            judged += 1
            live = (set(facts["agents"]) - {guard.ANY_AGENT}) or {"any.js"}
            if not guard.decide(facts=facts, game=(1, live)):
                continue
            halts += 1
            turn = ts.last_text_turn(ts.split_turns(chunk))
            closing = " ".join(turn.text_runs[-1].split())[-280:]
            after = " ".join(guard._prompt_text(events[bound]).split())[:160] if bound < len(events) else "<end>"
            print(f"--- {Path(path).name[:8]} event {bound}\n  closing: {closing}\n  user next: {after}")
    print(f"\nTOTAL: {halts} held of {judged} judged turn ends (sessions that ran the watcher), "
          f"{total} turn boundaries read")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
