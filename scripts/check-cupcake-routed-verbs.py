#!/usr/bin/env python3
"""Every decision verb a policy exports must be routed in .cupcake/system/evaluate.rego.

WHY THIS EXISTS (2026-09-23). `evaluate.rego` collects decisions by naming each policy's verb
explicitly, one line per policy, because the `walk()` form it replaced crashed the WASM runtime and
failed open. That file's own comment says:

    The cost of the explicit form is one line per decision-exporting policy, and forgetting that
    line means the policy is inert. scripts/check-cupcake-wasm-builtins.py exists to catch exactly
    that: it fails when a policy exports a verb this file does not route.

It did not. `no_unchecked_game_alive_claim` landed with its signal, its policy, 8/8 under
`opa test`, a passing signal unit test, `cupcake verify` green and the routing map listing it under
`Stop` -- and it was inert, because nothing added its line here. The gate was green the whole time.
The user found it by asking for the guard to be demonstrated and watching it not fire.

That is the same class of failure as the 36-day silent Stop guards, arriving through a different
door, and it is invisible in exactly the way that matters: every layer of the suite passes, the
engine loads and routes the policy, and the decision set comes back empty.

So this is the check the comment promised. It parses every `.rego` under .cupcake/policies/ for its
package name and the decision verbs it defines, and fails when evaluate.rego does not name that
package for that verb.

Run: python3 scripts/check-cupcake-routed-verbs.py
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
CUPCAKE_DIR = REPO_ROOT / ".cupcake"
POLICY_DIR = CUPCAKE_DIR / "policies"
EVALUATE = CUPCAKE_DIR / "system" / "evaluate.rego"

#: The verbs evaluate.rego aggregates. A policy defining any of these has to be routed for it.
VERBS = ("halt", "deny", "block", "ask", "modify", "add_context")

PACKAGE_RE = re.compile(r"^package\s+([A-Za-z0-9_.]+)\s*$", re.MULTILINE)


def verbs_defined(text: str) -> set[str]:
    """Which decision verbs this policy actually defines.

    Matches the two spellings a decision rule is written in -- `halt contains decision if {` and
    `halt[decision] {` -- at the start of a line, so a mention inside a comment or a helper's body
    is not mistaken for an export.
    """
    found = set()
    for verb in VERBS:
        pattern = re.compile(rf"^{verb}\s*(?:contains\b|\[)", re.MULTILINE)
        if pattern.search(text):
            found.add(verb)
    return found


def main() -> int:
    if not EVALUATE.is_file():
        print(f"check-cupcake-routed-verbs: {EVALUATE} is missing", file=sys.stderr)
        return 1
    aggregation = EVALUATE.read_text(encoding="utf-8")

    problems: list[str] = []
    checked = 0
    for path in sorted(POLICY_DIR.rglob("*.rego")):
        text = path.read_text(encoding="utf-8")
        package = PACKAGE_RE.search(text)
        if not package:
            problems.append(f"{path.relative_to(REPO_ROOT)}: no package declaration")
            continue
        exported = verbs_defined(text)
        if not exported:
            continue
        checked += 1
        for verb in sorted(exported):
            routed = f"data.{package.group(1)}.{verb}"
            if routed not in aggregation:
                problems.append(
                    f"{path.relative_to(REPO_ROOT)} exports `{verb}` but "
                    f"{EVALUATE.relative_to(REPO_ROOT)} does not route it.\n"
                    f"    The policy will load, route and evaluate, and its decision will be "
                    f"DISCARDED -- a silent allow.\n"
                    f"    Add:  all_{'halts' if verb == 'halt' else verb + 's'} contains decision "
                    f"if {{ some decision in {routed} }}"
                )

    if problems:
        print("check-cupcake-routed-verbs: FAILED", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1
    print(f"check-cupcake-routed-verbs: OK ({checked} decision-exporting policies, all routed)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
