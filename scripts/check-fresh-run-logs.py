#!/usr/bin/env python3
"""A log describes exactly one process run. This makes that executable.

    check-fresh-run-logs.py              the gate -- scan crates/**/*.rs
    check-fresh-run-logs.py --selftest   the cases this rule is supposed to get right

Standing rule, inherited from er-mods-rs (2026-08-04) and documented in
`crates/ds2-game-base/src/log.rs`: no DLL, shell or harness may append to a log across runs.
Each log file is truncated by the first write of the process that owns it; keeping an older run
means copying the file aside yourself, not letting it accumulate.

The failure that set the rule, in the sibling repo: a mod DLL opened its log with a plain
`OpenOptions::new().append(true)` on a fixed name next to the game executable. Twelve launches
piled into one 565 KB file, so a count taken over it ("37 confirms") read as one run doing
something 37 times when it was really twelve runs.

Until 2026-09-25 that rule held here by convention only -- the log module's own header said so.
Ported from `../er-mods-rs/scripts/check-fresh-run-logs.py`; the shape pinned is the same:

  * `ds2_game_base::log::begin_fresh_run(path)` is the one-shot. The first call for a path in a
    process rotates the previous run's file aside and truncates. Every later call is a no-op.
  * `ds2_game_base::log::open_fresh_run_append(path)` runs that one-shot and hands back an
    appending handle. It is the only sanctioned appending opener.
  * Therefore: an `OpenOptions`-style `.append(...)` may appear only in the helper module, or in
    a file listed in EXEMPT with a stated reason.

`Vec::append` / `String::append` take `&mut ...`, so an argument starting with `&` is not a file
opener and is not flagged. `File::create` / `truncate(true)` are already fresh and are ignored.

Exit status is 1 on any failure, so this can gate.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# Every Rust source in this workspace lives under crates/ (measured 2026-09-25: 154 files, none
# elsewhere outside target/ and .claude/), which is the same scope check-allow-debt.py scans.
HELPER = Path("crates/ds2-game-base/src/log.rs")

# `.append(` whose argument does not start with `&`. That one character separates an OpenOptions
# builder (`.append(true)`, `.append(flag)`) from `Vec::append(&mut other)`.
APPEND_CALL = re.compile(r"\.append\s*\(\s*(?!&)([^)]*)\)")

# The sanctioned entry points. A file that opens for append must be the module defining these.
HELPER_FUNCTIONS = ("begin_fresh_run", "open_fresh_run_append")

# Files allowed to open a log for append. Each needs a reason: an unexplained exemption reads as
# considered and silently drops a real violation.
EXEMPT: dict[str, str] = {
    HELPER.as_posix(): (
        "the one-shot truncation helper itself -- `begin_fresh_run` rotates + truncates on the "
        "first write of the process, and `open_fresh_run_append` / `open_truncated_with_header` "
        "are the appending openers every other crate routes through"
    ),
    "crates/ds2-loader/src/lib.rs": (
        "`detach_log_line` runs at DLL_PROCESS_DETACH, where the loader lock is held and every "
        "other thread is already dead -- possibly inside `begin_fresh_run`'s mutex, so taking it "
        "could hang the game on quit. The file was freshened earlier in the same process by the "
        "probe's install lines, so a plain append cannot reach across runs"
    ),
}


def rust_files(root: Path) -> list[Path]:
    """Every Rust source under `root/crates`, in a stable order."""
    return sorted((root / "crates").rglob("*.rs"))


def append_openers(text: str) -> list[tuple[int, str]]:
    """`(line number, argument)` for every file-append opener in `text`."""
    found: list[tuple[int, str]] = []
    for match in APPEND_CALL.finditer(text):
        # A doc comment that NAMES the pattern (the helper's own header does) is not an opener.
        line_start = text.rfind("\n", 0, match.start()) + 1
        if text[line_start : match.start()].lstrip().startswith("//"):
            continue
        line = text[: match.start()].count("\n") + 1
        found.append((line, match.group(1).strip()))
    return found


def defines_helper(text: str) -> bool:
    """Whether `text` is the module that defines the sanctioned helpers."""
    return all(re.search(rf"fn\s+{name}\b", text) for name in HELPER_FUNCTIONS)


def check(root: Path, sources: list[Path] | None = None) -> list[str]:
    """Return a list of failures; empty means every log in the tree is fresh per run."""
    failures: list[str] = []
    exempt_hits: dict[str, int] = {name: 0 for name in EXEMPT}

    for reason in EXEMPT.values():
        if not reason.strip():
            failures.append(
                "EXEMPT carries an entry with an empty reason -- an unexplained exemption "
                "cannot be told apart from an oversight"
            )

    for path in rust_files(root) if sources is None else sources:
        relative = path.relative_to(root).as_posix()
        openers = append_openers(path.read_text(encoding="utf-8", errors="replace"))
        if not openers:
            continue
        if relative in EXEMPT:
            exempt_hits[relative] += len(openers)
            continue
        for line, argument in openers:
            failures.append(
                f"{relative}:{line}: opens a log for append (`.append({argument})`) without the "
                f"one-shot truncation. Logs must describe ONE run: use "
                f"`ds2_game_base::log::open_fresh_run_append(path)` (or `append_line` / "
                f"`begin_fresh_run`), or add this file to EXEMPT with a reason."
            )

    # A stale exemption is worse than none: it reads as considered while covering nothing, and the
    # day that file grows a real appender the gate stays green.
    for name in sorted(EXEMPT):
        if not (root / name).exists():
            failures.append(f"{name}: EXEMPT here but the file does not exist -- stale exemption")
        elif exempt_hits[name] == 0:
            failures.append(
                f"{name}: EXEMPT here but it opens nothing for append -- stale exemption, drop it "
                f"so the next real appender in this file is caught"
            )

    # The helpers must actually exist, or every caller is routing through nothing.
    helper = root / HELPER
    if not helper.exists() or not defines_helper(
        helper.read_text(encoding="utf-8", errors="replace")
    ):
        failures.append(
            f"{HELPER.as_posix()} no longer defines {' + '.join(HELPER_FUNCTIONS)} -- the rule "
            f"has nothing to route through"
        )
    return failures


def selftest() -> int:
    """Prove the checks fire, on synthetic inputs, in both directions."""
    import tempfile

    failures = 0

    def case(name: str, condition: bool) -> None:
        nonlocal failures
        if not condition:
            print(f"selftest FAIL: {name}", file=sys.stderr)
            failures += 1

    helper_source = (
        "//! build on any `.append(...)` opener outside this module.\n"
        "pub fn begin_fresh_run(path: &Path) {}\n"
        "pub fn open_fresh_run_append(path: &Path) -> Option<File> {\n"
        "    begin_fresh_run(path);\n"
        "    OpenOptions::new().create(true).append(true).open(path).ok()\n"
        "}\n"
    )
    helper_without_opener = (
        "pub fn begin_fresh_run(path: &Path) {}\n"
        "pub fn open_fresh_run_append(path: &Path) -> Option<File> { None }\n"
    )
    loader_source = "fn detach_log_line() { OpenOptions::new().append(true).open(p); }\n"

    def tree(tmp: str, sources: dict[str, str], helper: str = helper_source) -> Path:
        root = Path(tmp)
        files = {HELPER.as_posix(): helper, "crates/ds2-loader/src/lib.rs": loader_source}
        files.update(sources)
        for name, body in files.items():
            target = root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(body)
        return root

    # Negative DIRECTION: a violating snippet must fail.
    with tempfile.TemporaryDirectory() as tmp:
        root = tree(
            tmp,
            {
                "crates/bad-dll/src/lib.rs": (
                    "fn log(args: Arguments<'_>) {\n"
                    "    if let Ok(mut f) = OpenOptions::new()\n"
                    "        .create(true)\n"
                    "        .append(true)\n"
                    "        .open(LOG_PATH)\n"
                    "    {\n"
                    "    }\n"
                    "}\n"
                )
            },
        )
        problems = check(root)
        case(
            "a plain append(true) opener fails",
            any("crates/bad-dll/src/lib.rs:4" in p and ".append(true)" in p for p in problems),
        )

    # The variable-argument bypass: `.append(flag)` is the same hole spelled differently.
    with tempfile.TemporaryDirectory() as tmp:
        root = tree(
            tmp, {"crates/sneaky-dll/src/lib.rs": "let f = File::options().append(keep).open(p);\n"}
        )
        case(
            "append(<variable>) fails too",
            any("crates/sneaky-dll/src/lib.rs:1" in p for p in check(root)),
        )

    # Positive DIRECTION: compliant sources, Vec::append, truncation and a doc comment naming the
    # pattern must all pass.
    with tempfile.TemporaryDirectory() as tmp:
        root = tree(
            tmp,
            {
                "crates/good-dll/src/lib.rs": (
                    "if let Some(mut f) = ds2_game_base::log::open_fresh_run_append(&path()) {}\n"
                ),
                "crates/good-dll/src/other.rs": "keep.append(&mut local);\n",
                "crates/good-dll/src/third.rs": (
                    "let f = OpenOptions::new().write(true).truncate(true).open(p);\n"
                ),
                "crates/good-dll/src/fourth.rs": "/// never `.append(true)` a log directly\n",
            },
        )
        problems = check(root)
        case("a compliant tree passes", problems == [])

    # A stale exemption must be reported rather than quietly covering nothing.
    with tempfile.TemporaryDirectory() as tmp:
        root = tree(tmp, {}, helper=helper_without_opener)
        case(
            "an exemption covering no appender fails",
            any("stale exemption" in p for p in check(root)),
        )

    # Losing the helper must fail loudly: callers would be routing through nothing.
    with tempfile.TemporaryDirectory() as tmp:
        root = tree(tmp, {}, helper="OpenOptions::new().append(true);\n")
        case(
            "a gutted helper module fails",
            any("nothing to route through" in p for p in check(root)),
        )

    if failures:
        print(f"selftest: {failures} case(s) failed", file=sys.stderr)
        return 1
    print("[check-fresh-run-logs] selftest ok (6 cases)")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--selftest", action="store_true")
    parser.add_argument("--root", type=Path, default=REPO_ROOT)
    args = parser.parse_args()
    if args.selftest:
        return selftest()

    root = args.root.resolve()
    sources = rust_files(root)
    failures = check(root, sources)
    if failures:
        print("[check-fresh-run-logs] FAIL:", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1
    print(
        f"[check-fresh-run-logs] ok -- {len(sources)} Rust files scanned, every log "
        f"opener routes through the one-shot truncation, {len(EXEMPT)} file(s) exempt with reasons"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
