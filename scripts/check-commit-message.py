#!/usr/bin/env python3
"""Check a commit message against this repo's conventional-commits rule.

Three callers, one rule:

    check-commit-message.py <file>        the commit-msg hook -- one message, as git wrote it
    check-commit-message.py --range A..B  the gate -- every commit a branch adds on top of A
    check-commit-message.py --selftest    the cases this rule is supposed to get right

The scope list is not written down here. It is read from `crates/` on every run, so adding a crate
needs no edit to this file, and a scope naming no crate is a typo rather than a new area. The seven
non-crate scopes below are the directories that are not crates but are still places work happens.

History before this rule landed is not retroactively wrong and is not checked: the hook sees only
new messages, and `--range` is given `origin/main..HEAD`, so only commits a branch is proposing are
judged. That is also why `Merge`, `Revert "..."` and the `fixup!`/`squash!` autosquash prefixes are
exempt -- git composes all three itself, and a rule that refuses git's own wording would be a rule
nobody can satisfy without rewriting the message git needs.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# The kind of change, never the place it happened -- that is what the scope is for. A new flag in
# `scripts/ds2-run.py` is a `feat(scripts)`, not a `chore`, because someone can now do something
# they could not do before, and which directory that lives in does not change the fact.
TYPES = {
    "feat": "new behaviour a player, an agent or a caller can reach",
    "fix": "a defect in behaviour that already shipped",
    "perf": "the same behaviour, measurably faster",
    "refactor": "the same behaviour, a different shape",
    "docs": "documentation and reverse-engineering notes only",
    "test": "tests and selftests only",
    "build": "the workspace, its dependencies, the toolchain, vendored sources",
    "ci": "automation that runs outside a developer's shell",
    "chore": "housekeeping that changes nobody's behaviour",
    "revert": "backs out an earlier commit",
}

# Directories that are not crates but are still somewhere a commit can land.
NON_CRATE_SCOPES = {
    "scripts",  # everything under scripts/, the gate included
    "docs",
    "cupcake",  # the .cupcake/ guard layer
    "beads",  # .beads/ plumbing and hooks
    "workspace",  # Cargo.toml, the toolchain pin, .cargo/
    "vendor",
    "github",  # .github/, including the pull request template
}

# Long enough that this repo's prose subjects survive the type and scope being bolted on the front:
# the longest subject in the history that predates this rule needed a little over a hundred
# characters on its own. Short enough that a header still fits one terminal line in `git log
# --oneline`.
HEADER_LIMIT = 120

# Matches the body wrapping the repo already writes by hand.
BODY_LIMIT = 100

HEADER_PATTERN = re.compile(
    r"^(?P<type>[a-z]+)"
    r"(?:\((?P<scope>[^()]*)\))?"
    r"(?P<breaking>!)?"
    r"(?P<colon>:)(?P<space>[ ]?)"
    r"(?P<subject>.*)$"
)

# Wording git composes itself. Refusing any of these refuses git, not the author.
EXEMPT_PREFIXES = ("Merge ", "Revert ", "fixup! ", "squash! ", "amend! ")


def crate_scopes() -> set[str]:
    crates = REPO_ROOT / "crates"
    if not crates.is_dir():
        return set()
    return {p.name for p in crates.iterdir() if (p / "Cargo.toml").is_file()}


def allowed_scopes() -> set[str]:
    return crate_scopes() | NON_CRATE_SCOPES


def comment_char() -> str:
    """Whatever git will strip from the message, so this rule strips the same thing.

    `core.commentChar` is configurable and can be `auto`, in which case git picks a character that
    does not start any line of the message. Reading it back is not possible, so `auto` falls back
    to the default and the worst case is that a line starting with `#` is judged as prose.
    """
    try:
        value = subprocess.run(
            ["git", "config", "--get", "core.commentChar"],
            capture_output=True,
            text=True,
            check=False,
        ).stdout.strip()
    except OSError:
        return "#"
    return value if len(value) == 1 else "#"


def strip_git_furniture(raw: str, char: str = "#") -> str:
    """Remove what git removes: comment lines, and everything below the scissors line."""
    lines = []
    for line in raw.split("\n"):
        if line.startswith(char + " ---") and ">8" in line:
            break
        if line.startswith(char):
            continue
        lines.append(line)
    return "\n".join(lines).strip("\n")


def is_exempt(header: str) -> bool:
    return header.startswith(EXEMPT_PREFIXES)


def unbreakable(line: str, limit: int) -> bool:
    """A line no rewrap can shorten -- a url, a path, a register dump -- is not a wrapping fault."""
    return max((len(token) for token in line.split()), default=0) > limit


def check(message: str, scopes: set[str] | None = None) -> list[str]:
    """Return one plain sentence per problem. An empty list means the message is fine."""
    scopes = allowed_scopes() if scopes is None else scopes
    text = strip_git_furniture(message, comment_char())
    lines = text.split("\n")
    header = lines[0] if lines else ""

    if not header.strip():
        return ["The commit message is empty."]

    if is_exempt(header):
        return []

    problems: list[str] = []
    match = HEADER_PATTERN.match(header)
    if not match:
        return [
            "The header is not a conventional commit. It must read "
            "`type(scope): subject`, `type: subject`, or either with a `!` before the colon for a "
            "breaking change. Types: " + ", ".join(sorted(TYPES)) + "."
        ]

    kind = match.group("type")
    scope = match.group("scope")
    subject = match.group("subject")

    if kind not in TYPES:
        problems.append(
            f"`{kind}` is not a type this repo uses. Pick one of: " + ", ".join(sorted(TYPES)) + "."
        )

    if scope is not None:
        if scope == "":
            problems.append("The scope is empty. Write `type: subject` instead of `type(): subject`.")
        elif scope not in scopes:
            problems.append(
                f"`{scope}` is not a scope in this repo. A scope is a crate directory under "
                "`crates/`, or one of: " + ", ".join(sorted(NON_CRATE_SCOPES)) + "."
            )

    if match.group("space") != " ":
        problems.append("There is no space after the colon.")

    if not subject.strip():
        problems.append("The subject is empty.")
    else:
        if subject != subject.strip():
            problems.append("The subject has leading or trailing whitespace.")
        if subject.rstrip().endswith("."):
            problems.append("The subject ends with a full stop. Drop it -- a header is not a sentence.")

    if len(header) > HEADER_LIMIT:
        problems.append(
            f"The header is longer than {HEADER_LIMIT} characters. Move the detail into the body."
        )

    if len(lines) > 1 and lines[1].strip():
        problems.append("The line after the header is not blank. A body needs a blank line above it.")

    in_fence = False
    for number, line in enumerate(lines[1:], start=2):
        if line.lstrip().startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence or unbreakable(line, BODY_LIMIT):
            continue
        if len(line) > BODY_LIMIT:
            problems.append(f"Line {number} of the body is longer than {BODY_LIMIT} characters.")
            break

    return problems


def report(header: str, problems: list[str], source: str) -> None:
    print(f"commit message rejected ({source}):", file=sys.stderr)
    print(f"  {header}", file=sys.stderr)
    for problem in problems:
        print(f"  - {problem}", file=sys.stderr)
    print("", file=sys.stderr)
    print("  The convention, with examples: docs/COMMITS.md", file=sys.stderr)


def check_file(path: Path) -> int:
    message = path.read_text(encoding="utf-8", errors="replace")
    problems = check(message)
    if not problems:
        return 0
    header = strip_git_furniture(message, comment_char()).split("\n")[0]
    report(header, problems, str(path))
    return 1


def check_range(rev_range: str) -> int:
    result = subprocess.run(
        ["git", "log", "--format=%H", rev_range],
        capture_output=True,
        text=True,
        cwd=REPO_ROOT,
    )
    if result.returncode != 0:
        print(result.stderr.strip(), file=sys.stderr)
        return 1

    shas = [sha for sha in result.stdout.split() if sha]
    if not shas:
        print(f"  no commits in {rev_range}")
        return 0

    scopes = allowed_scopes()
    failed = 0
    for sha in shas:
        body = subprocess.run(
            ["git", "log", "-1", "--format=%B", sha],
            capture_output=True,
            text=True,
            cwd=REPO_ROOT,
            check=True,
        ).stdout
        problems = check(body, scopes)
        if problems:
            failed += 1
            report(f"{sha[:12]} {body.split(chr(10))[0]}", problems, rev_range)
    if failed:
        return 1
    plural = "" if len(shas) == 1 else "s"
    print(f"  {len(shas)} commit{plural} in {rev_range}: OK")
    return 0


# Each case is (message, expected_to_pass, what it pins). The scope set is fixed here rather than
# read from disk, so the selftest keeps testing the rule after a crate is renamed.
SELFTEST_SCOPES = {"ds2-save-block", "ds2-menu-row", "scripts", "docs", "cupcake"}

SELFTEST_CASES = [
    ("feat(ds2-save-block): stop the game saving by itself while the save row is on", True),
    ("fix: a matching item id is not an item you own", True),
    ("feat(ds2-menu-row)!: a row is registered by name, and the numbers are gone", True),
    ("docs(cupcake): the porting rule, and what it refused", True),
    ("Merge pull request #57 from Banon-Labs/no-default-saving-while-the-save-row-is-on", True),
    ('Revert "feat(scripts): --all-menu-rows"', True),
    ("fixup! feat(scripts): --all-menu-rows", True),
    ("Stop the game saving by itself while Save Game to File is on the menu", False),
    ("Feat(scripts): the type is capitalised", False),
    ("feat(ds2-nonesuch): the scope names no crate", False),
    ("feat(): the scope is empty", False),
    ("feat:no space after the colon", False),
    ("feat(scripts): ", False),
    ("fix(scripts): the subject ends in a full stop.", False),
    ("feat(scripts): " + "x" * HEADER_LIMIT, False),
    ("feat(scripts): a subject\nthe body starts with no blank line above it", False),
    ("feat(scripts): a subject\n\n" + "w " * BODY_LIMIT, False),
    ("feat(scripts): a subject\n\nA body that is wrapped like the rest of this repo.", True),
    (
        "feat(scripts): a subject\n\n```\n" + "x" * (BODY_LIMIT + 40) + "\n```",
        True,
    ),
    (
        "feat(docs): a subject\n\nhttps://example.invalid/" + "p" * (BODY_LIMIT + 40),
        True,
    ),
    ("feat(scripts): a subject\n\n# a comment line git would strip\nA body.", True),
]


def selftest() -> int:
    failures = 0
    for message, should_pass in SELFTEST_CASES:
        problems = check(message, SELFTEST_SCOPES)
        passed = not problems
        if passed != should_pass:
            failures += 1
            want = "accepted" if should_pass else "rejected"
            print(f"  expected {want}: {message.splitlines()[0]!r}", file=sys.stderr)
            for problem in problems:
                print(f"    got: {problem}", file=sys.stderr)

    # The scope list has to come off disk, or a renamed crate silently stops being a legal scope.
    if not crate_scopes():
        failures += 1
        print("  crates/ produced no scopes -- the scope list is not being read", file=sys.stderr)

    if failures:
        print(f"check-commit-message.py selftest: {failures} failed", file=sys.stderr)
        return 1
    print("  check-commit-message.py: OK")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("file", nargs="?", help="a commit message file, as git hands one to a hook")
    parser.add_argument("--range", dest="rev_range", help="a git revision range, e.g. origin/main..HEAD")
    parser.add_argument("--selftest", action="store_true", help="run the cases this rule must get right")
    args = parser.parse_args()

    if args.selftest:
        return selftest()
    if args.rev_range:
        return check_range(args.rev_range)
    if args.file:
        return check_file(Path(args.file))

    parser.print_help(sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
