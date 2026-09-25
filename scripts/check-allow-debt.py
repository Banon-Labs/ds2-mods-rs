#!/usr/bin/env python3
"""Every lint allow in `crates/` names the issue that will remove it.

    check-allow-debt.py              the gate -- scan crates/**/*.rs
    check-allow-debt.py --selftest   the cases this rule is supposed to get right

`docs/COMMENTS.md` is the convention. An `#[allow(...)]` switches off a lint that the root
manifest denies on purpose, so it is a hole in a gate, and a hole nobody is tracking is
indistinguishable from one nobody noticed. The rule:

    # DEBT: ds2-mods-rs-xxx -- MinHook's C ABI; upstream names are not ours to rename.
    #[allow(non_snake_case)]

A prose reason above the attribute is NOT enough on its own. MEASURED before this rule landed:
13 allow sites in this workspace, 9 of them bare and 4 carrying a reason, 0 naming an issue --
so the four good ones read exactly like a decision someone made and forgot, and the nine bare
ones could not be told from an accident.

The issue only has to EXIST, not to be open. A binding file's `non_camel_case_types` allow is
permanent, and its issue says why and is closed; that is still a record a reader can follow,
which a bare attribute is not.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CRATES = REPO_ROOT / "crates"

# `#[allow(`, `#![allow(`, and the `allow(` inside a `cfg_attr`. `expect` is deliberately not
# here: it fails when the lint STOPS firing, so it cannot rot into a silent hole the way an
# allow does.
ALLOW = re.compile(r"^\s*#!?\[.*\ballow\s*\(")
DEBT = re.compile(r"\bDEBT:\s*(ds2-mods-rs-[a-z0-9]+)")
COMMENT = re.compile(r"^\s*//")


def justification(lines: list[str], idx: int) -> str | None:
    """The bd id in the contiguous comment block directly above `lines[idx]`, if any."""
    i = idx - 1
    block = []
    while i >= 0 and COMMENT.match(lines[i]):
        block.append(lines[i])
        i -= 1
    for line in block:
        m = DEBT.search(line)
        if m:
            return m.group(1)
    return None


def scan(text: str) -> list[tuple[int, str | None]]:
    """Every allow site in `text`, as (1-based line, bd id or None)."""
    lines = text.splitlines()
    return [
        (n + 1, justification(lines, n))
        for n, line in enumerate(lines)
        if ALLOW.match(line)
    ]


def issue_exists(issue: str) -> bool:
    return (
        subprocess.run(
            [str(Path.home() / ".local/bin/bd"), "show", issue],
            capture_output=True,
        ).returncode
        == 0
    )


def gate() -> int:
    bad = 0
    seen: set[str] = set()
    for path in sorted(CRATES.rglob("*.rs")):
        for line, issue in scan(path.read_text(encoding="utf-8", errors="replace")):
            rel = path.relative_to(REPO_ROOT)
            if issue is None:
                print(
                    f"  {rel}:{line}: allow with no `# DEBT: <issue>` comment above it",
                    file=sys.stderr,
                )
                bad += 1
            else:
                seen.add(issue)
    for issue in sorted(seen):
        if not issue_exists(issue):
            print(f"  `# DEBT: {issue}` names no issue bd knows about", file=sys.stderr)
            bad += 1
    if bad:
        print(f"  {bad} unaccounted lint allow(s) -- docs/COMMENTS.md", file=sys.stderr)
        return 1
    print(f"  {len(seen)} tracked allow(s), every one naming a live issue")
    return 0


CASES = [
    # (source, expected [(line, issue)])
    ("#[allow(dead_code)]\nfn f() {}\n", [(1, None)]),
    ("// DEBT: ds2-mods-rs-abc -- why.\n#[allow(dead_code)]\n", [(2, "ds2-mods-rs-abc")]),
    # the reason may sit above the marker in the same block
    (
        "// PARITY: upstream header shape.\n// DEBT: ds2-mods-rs-xyz\n#[allow(non_snake_case)]\n",
        [(3, "ds2-mods-rs-xyz")],
    ),
    # prose alone is not enough -- this is the case the rule exists for
    ("// PARITY: upstream header shape.\n#[allow(non_snake_case)]\n", [(2, None)]),
    # a blank line breaks the block, so the marker no longer applies to the attribute
    ("// DEBT: ds2-mods-rs-abc\n\n#[allow(dead_code)]\n", [(3, None)]),
    # inner attribute, and the cfg_attr form
    ("// DEBT: ds2-mods-rs-abc\n#![allow(dead_code)]\n", [(2, "ds2-mods-rs-abc")]),
    (
        "// DEBT: ds2-mods-rs-abc\n#[cfg_attr(not(windows), allow(unused))]\n",
        [(2, "ds2-mods-rs-abc")],
    ),
    # `expect` is out of scope on purpose
    ("#[expect(dead_code)]\nfn f() {}\n", []),
    # a doc comment counts as part of the block above
    ("/// What it does.\n// DEBT: ds2-mods-rs-abc\n#[allow(dead_code)]\n", [(3, "ds2-mods-rs-abc")]),
]


def selftest() -> int:
    bad = 0
    for src, want in CASES:
        got = scan(src)
        if got != want:
            print(f"  FAILED: {src!r}\n    want {want}\n    got  {got}", file=sys.stderr)
            bad += 1
    if bad:
        return 1
    print(f"  selftest: {len(CASES)} cases")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--selftest", action="store_true", help="run the rule's own cases")
    args = ap.parse_args()
    return selftest() if args.selftest else gate()


if __name__ == "__main__":
    sys.exit(main())
