#!/usr/bin/env python3
"""Stage `dinput8.dll` into DARK SOULS II and launch it, gated on the DLL's own log line.

WHY A TOOL AND NOT AN AGENT'S SENTENCE
--------------------------------------
"I launched it and it worked" is a claim a tool can make and a person can check. This script
prints its success block ONLY after reading the loader DLL's `ds2-loader: arxan ...` line out of
the log file the DLL itself wrote during THIS run. Nothing else -- not the process existing, not
Steam returning zero, not a window appearing -- is accepted as evidence, because a copy-pasted
block is a promise to whoever reads it. On timeout it says so plainly and exits non-zero.

Patterned on `../er-mods-rs/scripts/er-run-branch.py`, whose testimony discipline (and whose two
hard-won log-tailing bugs -- rotation and partial lines, both commented below) this reproduces.

THE PIPELINE
------------
  1. PREFLIGHT   -- the built DLL exists; the game directory exists and is writable; `steam` is
                    on PATH. Refuse rather than guess.
  2. STAGE       -- copy the built DLL over `<Game>/dinput8.dll`.
  3. FINGERPRINT -- print the SHA-256 of the STAGED file, read back off disk. A build that
                    failed, or that "succeeded" without recompiling, leaves the previous DLL in
                    place; a run against that produces evidence for code which is not the code
                    under test. The hash is what makes that visible instead of invisible.
  4. LAUNCH      -- `steam -applaunch 335300` with WINEDLLOVERRIDES="dinput8=n,b". Without that
                    override Wine's builtin dinput8 wins the load and our DLL never runs.
  5. TESTIMONY   -- poll `<Game>/ds2-loader.log` for the DLL's own line. Success block only if
                    it appears; a FAILED block, and a non-zero exit, otherwise.

Usage:
    python3 scripts/ds2-run.py --dry-run   # stage nothing, launch nothing, report what it would do
    python3 scripts/ds2-run.py --selftest  # exercise the log tailer against synthetic rotations
    python3 scripts/ds2-run.py             # the real run
"""

from __future__ import annotations

import argparse
import hashlib
import os
import shutil
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

GAME_DIR = (
    Path.home()
    / ".local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game"
)
APPID = "335300"

#: cargo emits this because `crates/ds2-loader/Cargo.toml` sets `[lib] name = "dinput8"`.
BUILT_DLL = REPO_ROOT / "target/x86_64-pc-windows-msvc/release/dinput8.dll"
STAGED_DLL_NAME = "dinput8.dll"

#: Mirrors `LOG_FILE_NAME` in `crates/ds2-loader/src/lib.rs`. Both halves of this contract have
#: to change together: rename it on one side only and every run reports a false "did not load".
LOG_NAME = "ds2-loader.log"

#: The strong witness. Written from dearxan's callback, so it proves the DLL loaded AND that
#: `neuter_arxan` reached its callback -- which is the whole point of this build.
ARXAN_LINE_PREFIX = "ds2-loader: arxan"

#: The weak witness. Written at `DLL_PROCESS_ATTACH`. It proves only that the DLL was loaded.
#: Kept separate from the strong one on purpose: "loaded, and dearxan never reported" is a
#: completely different failure from "never loaded", and conflating them sends you hunting a
#: WINEDLLOVERRIDES problem that does not exist.
ATTACH_LINE_PREFIX = "ds2-loader: attach"

#: Native first, then builtin. Wine prefers its own `dinput8` without this and the proxy is
#: simply never mapped.
DLL_OVERRIDE = "dinput8=n,b"

#: How long to wait for the DLL to speak. DS2 boots through Proton and dearxan analyses 48 Arxan
#: stubs single-threaded before its callback runs; neither has been timed on this machine, so
#: this is a deliberately loose bound, not a measurement.
TESTIMONY_BUDGET_SECONDS = 240.0
POLL_SECONDS = 0.5

#: /proc/<pid>/comm is truncated to 15 characters and "DarkSoulsII.exe" is exactly 15, so the
#: exact-comm match works. `pgrep -f 'DarkSoulsII.exe'` does NOT: it matches this script's own
#: command line, which contains the string, and reports the game running before it has started.
GAME_COMM = "DarkSoulsII.exe"

EXIT_OK = 0
EXIT_NO_TESTIMONY = 2
EXIT_ERROR = 3


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def pgrep_exact(comm: str) -> list[int]:
    """PIDs whose exact comm is `comm`. Empty on no match; `pgrep` exits 1 for that."""
    proc = subprocess.run(
        ["pgrep", "-x", comm], capture_output=True, text=True, timeout=10
    )
    return [int(line) for line in proc.stdout.split() if line.isdigit()]


def steam_running() -> bool:
    """Is the Steam client up? Native Linux only -- this repo's game runs through Proton."""
    return bool(pgrep_exact("steam") or pgrep_exact("steamwebhelper"))


class LogTail:
    """Read only what THIS run appended, surviving the DLL's startup log rotation.

    `ds2_game_base::log` renames `<log>` to `<log>.prev` and truncates on the DLL's first write,
    so a plain offset would either miss the new file or replay the old one. Remembering the
    inode as well makes "everything since launch" exact. Both rotation shapes are handled, and
    missing the second one cost `../er-mods-rs` a live run:

      * REPLACED           -- new inode, so read the whole new file.
      * TRUNCATED IN PLACE -- same inode, size dropped below our offset. Seeking to the old
                              offset lands past EOF and reads NOTHING, so the DLL's startup
                              lines are invisible and a perfectly healthy run is reported silent.
    """

    def __init__(self, path: Path) -> None:
        self.path = path
        try:
            stat = path.stat()
            self.inode: int | None = stat.st_ino
            self.offset = stat.st_size
        except OSError:
            self.inode, self.offset = None, 0

    def new_text(self) -> str:
        try:
            stat = self.path.stat()
        except OSError:
            return ""
        rotated = stat.st_ino != self.inode or stat.st_size < self.offset
        start = 0 if rotated else self.offset
        try:
            with self.path.open("rb") as handle:
                handle.seek(min(start, stat.st_size))
                data = handle.read()
        except OSError:
            return ""
        # A LINE IS EVIDENCE ONLY ONCE IT IS TERMINATED. A read landing mid-write returns a
        # PREFIX, and a prefix of "ds2-loader: arxan status=ok detected=true ..." is
        # indistinguishable from a DLL that reported less than it did. Hand back whole lines
        # only; the remainder arrives complete on the next poll, milliseconds later.
        end = data.rfind(b"\n")
        if end < 0:
            return ""
        consumed = (0 if rotated else start) + end + 1
        self.inode, self.offset = stat.st_ino, consumed
        return data[: end + 1].decode("utf-8", errors="replace")


def await_testimony(tail: LogTail) -> dict:
    """Block until the DLL reports its Arxan result, or the budget runs out.

    Verdicts:
      confirmed        -- the `ds2-loader: arxan` line is on disk. This run is what it says.
      attached-silent  -- the DLL loaded and said so, but dearxan's callback never reported.
                          The proxy and the override WORKED; the Arxan step is what did not.
      silent           -- no line at all. Most likely the DLL was never mapped.
    """
    deadline = time.monotonic() + TESTIMONY_BUDGET_SECONDS
    attach_line: str | None = None
    game_seen = False
    while True:
        for line in tail.new_text().splitlines():
            stripped = line.strip()
            if stripped.startswith(ARXAN_LINE_PREFIX):
                return {
                    "status": "confirmed",
                    "line": stripped,
                    "attach_line": attach_line,
                    "waited": TESTIMONY_BUDGET_SECONDS - (deadline - time.monotonic()),
                }
            if stripped.startswith(ATTACH_LINE_PREFIX):
                attach_line = stripped

        alive = bool(pgrep_exact(GAME_COMM))
        game_seen = game_seen or alive
        if game_seen and not alive:
            # The game came up and went away. Waiting longer cannot produce a line, because
            # there is no longer a process to write one. Re-read once first: the exit path and
            # the last write race, and losing that race would report silence over real evidence.
            for line in tail.new_text().splitlines():
                stripped = line.strip()
                if stripped.startswith(ARXAN_LINE_PREFIX):
                    return {
                        "status": "confirmed",
                        "line": stripped,
                        "attach_line": attach_line,
                        "waited": TESTIMONY_BUDGET_SECONDS - (deadline - time.monotonic()),
                    }
                if stripped.startswith(ATTACH_LINE_PREFIX):
                    attach_line = stripped
            return {
                "status": "attached-silent" if attach_line else "silent",
                "attach_line": attach_line,
                "game_exited": True,
                "game_seen": True,
            }

        if time.monotonic() >= deadline:
            return {
                "status": "attached-silent" if attach_line else "silent",
                "attach_line": attach_line,
                "game_exited": False,
                "game_seen": game_seen,
            }
        time.sleep(POLL_SECONDS)


def running_block(context: dict) -> str:
    lines = [
        "```",
        "============= DARK SOULS II IS RUNNING, WITH OUR DLL IN IT =============",
        f"  started        {context['started']}",
        f"  appid          {APPID}",
        f"  override       WINEDLLOVERRIDES={DLL_OVERRIDE}",
        "",
        f"  staged         {context['staged']}",
        f"  sha256         {context['sha256']}",
        f"  game pids      {context['game_pids'] or '<none right now>'}",
        "",
        "  PROVEN BY      the DLL's own log line, not by the process existing:",
        f"    {context['testimony']}",
    ]
    if context.get("attach_line"):
        lines.append(f"    {context['attach_line']}")
    lines += [
        f"  log            {context['log']}",
        f"  waited         {context['waited']:.1f}s",
        "",
        "  NOT claimed    window visible / menu reached / input working / Arxan actually",
        "                 defeated. This block says the proxy loaded and dearxan reported;",
        "                 read the line above for WHAT it reported.",
        "========================================================================",
        "```",
    ]
    return "\n".join(lines)


def failed_block(reason: str, detail: list[str]) -> str:
    lines = [
        "```",
        "================ DARK SOULS II: NO DLL TESTIMONY ================",
        f"  reason  {reason}",
    ]
    lines.extend(f"  {line}" for line in detail)
    lines += ["=================================================================", "```"]
    return "\n".join(lines)


def preflight(dry_run: bool) -> list[str]:
    """Return the problems that make a real run impossible. Empty list means go."""
    problems: list[str] = []
    if not BUILT_DLL.is_file():
        problems.append(
            f"built DLL not found: {BUILT_DLL}\n"
            "    build it: cargo xwin build --release --target x86_64-pc-windows-msvc -p ds2-loader"
        )
    if not GAME_DIR.is_dir():
        problems.append(f"game directory not found: {GAME_DIR}")
    elif not os.access(GAME_DIR, os.W_OK):
        problems.append(f"game directory is not writable: {GAME_DIR}")
    if not dry_run and shutil.which("steam") is None:
        problems.append("`steam` is not on PATH")
    return problems


def report_environment() -> None:
    """Print the two environment facts that decide whether the override reaches the game."""
    print(f"[env] game dir     {GAME_DIR}")
    print(f"[env] built DLL    {BUILT_DLL}")
    if steam_running():
        print(
            "[env] steam        ALREADY RUNNING -- `steam -applaunch` hands the request to the\n"
            "                   running client over IPC, so the WINEDLLOVERRIDES set by THIS\n"
            "                   process is not guaranteed to reach the game. If the run comes\n"
            "                   back with no testimony, that is the first thing to rule out:\n"
            "                   quit Steam and re-run so this invocation starts the client, or\n"
            "                   set the per-app launch options to\n"
            f"                     WINEDLLOVERRIDES=\"{DLL_OVERRIDE}\" %command%"
        )
    else:
        print(
            "[env] steam        not running -- this invocation starts the client, so the\n"
            "                   override is inherited by everything it launches."
        )


def stage() -> tuple[Path, str]:
    """Copy the built DLL into the game directory; return the staged path and ITS hash."""
    staged = GAME_DIR / STAGED_DLL_NAME
    shutil.copyfile(BUILT_DLL, staged)
    # Hash the STAGED file, read back off disk, not the source. The point of printing a hash is
    # to describe the bytes that will actually be loaded.
    return staged, sha256(staged)


def dry_run() -> int:
    print("[dry-run] staging nothing, launching nothing.")
    report_environment()
    problems = preflight(dry_run=True)
    for problem in problems:
        print(f"[dry-run] WOULD REFUSE: {problem}")

    staged = GAME_DIR / STAGED_DLL_NAME
    if BUILT_DLL.is_file():
        print(f"[dry-run] built    sha256 {sha256(BUILT_DLL)}  {BUILT_DLL}")
    if staged.is_file():
        current = sha256(staged)
        print(f"[dry-run] staged   sha256 {current}  {staged}")
        if BUILT_DLL.is_file() and current != sha256(BUILT_DLL):
            print("[dry-run] staged DLL DIFFERS from the built one; a real run would replace it.")
    else:
        print(f"[dry-run] staged   <absent>  {staged}")

    log_path = GAME_DIR / LOG_NAME
    print(f"[dry-run] would copy   {BUILT_DLL}")
    print(f"[dry-run]         to   {staged}")
    print(f"[dry-run] would launch env WINEDLLOVERRIDES={DLL_OVERRIDE} steam -applaunch {APPID}")
    print(f"[dry-run] would poll   {log_path}")
    print(f"[dry-run]         for  a line starting {ARXAN_LINE_PREFIX!r}")
    print(f"[dry-run]         upto {TESTIMONY_BUDGET_SECONDS:.0f}s, then FAIL with exit {EXIT_NO_TESTIMONY}")
    print(f"[dry-run] log present now: {log_path.is_file()}")
    print(f"[dry-run] {GAME_COMM} running now: {bool(pgrep_exact(GAME_COMM))}")
    return EXIT_ERROR if problems else EXIT_OK


def launch() -> int:
    report_environment()
    problems = preflight(dry_run=False)
    if problems:
        for problem in problems:
            print(f"REFUSING TO LAUNCH: {problem}", file=sys.stderr)
        return EXIT_ERROR

    staged, digest = stage()
    print(f"[stage] {staged}")
    print(f"[stage] sha256 {digest}")

    log_path = GAME_DIR / LOG_NAME
    # Take the tail's mark BEFORE launching. Everything it hands back afterwards is this run's.
    # Deliberately NOT deleting the log: the DLL rotates it to `.prev` itself on its first write,
    # and deleting here would destroy the previous run's evidence for no gain.
    tail = LogTail(log_path)

    started = datetime.now(timezone.utc).isoformat(timespec="seconds")
    subprocess.Popen(
        ["steam", "-applaunch", APPID],
        env={**os.environ, "WINEDLLOVERRIDES": DLL_OVERRIDE},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        stdin=subprocess.DEVNULL,
        start_new_session=True,  # survives this shell, and this agent turn
    )
    print(f"[launch] steam -applaunch {APPID}  (WINEDLLOVERRIDES={DLL_OVERRIDE})")
    print(f"[launch] waiting up to {TESTIMONY_BUDGET_SECONDS:.0f}s for {log_path}")

    verdict = await_testimony(tail)
    if verdict["status"] != "confirmed":
        if verdict["status"] == "attached-silent":
            reason = "the DLL LOADED but dearxan never reported"
            detail = [
                f"saw       {verdict['attach_line']}",
                f"missing   a line starting {ARXAN_LINE_PREFIX!r}",
                "",
                "The proxy and the WINEDLLOVERRIDES both worked -- this is not a loading",
                "failure. neuter_arxan's callback did not reach the log, so look at dearxan,",
                "not at the override.",
            ]
        else:
            reason = "no DLL testimony at all"
            detail = [
                f"waited    {TESTIMONY_BUDGET_SECONDS:.0f}s for any line in {log_path}",
                f"{GAME_COMM} was "
                + (
                    "seen and has since exited"
                    if verdict.get("game_exited")
                    else ("seen and is still running" if verdict.get("game_seen") else "never seen")
                ),
                "",
                "Most likely the DLL was never mapped: check WINEDLLOVERRIDES actually reached",
                "the game (see the [env] steam note above), and that the staged DLL is the one",
                "whose sha256 is printed above.",
            ]
        print(failed_block(reason, detail))
        return EXIT_NO_TESTIMONY

    print(
        running_block(
            {
                "started": started,
                "staged": staged,
                "sha256": digest,
                "log": log_path,
                "testimony": verdict["line"],
                "attach_line": verdict.get("attach_line"),
                "waited": verdict["waited"],
                "game_pids": pgrep_exact(GAME_COMM),
            }
        )
    )
    return EXIT_OK


def selftest() -> int:
    """Exercise the log tailer against the rotations it exists to survive.

    This is the only part of the script that can silently turn a good run into a reported
    failure, so it is the only part with a test.
    """
    import tempfile

    ok = True

    def check(condition: bool, label: str) -> None:
        nonlocal ok
        if not condition:
            ok = False
        print(("  ok   " if condition else "  FAIL ") + label)

    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / LOG_NAME

        path.write_text("STALE FROM AN EARLIER RUN\n", encoding="utf-8")
        tail = LogTail(path)
        check(tail.new_text() == "", "a pre-existing log contributes nothing to this run")

        # REPLACED: the DLL rotated the old file aside and made a new one.
        path.rename(path.with_suffix(path.suffix + ".prev"))
        path.write_text(f"{ATTACH_LINE_PREFIX} x\n", encoding="utf-8")
        check(
            ATTACH_LINE_PREFIX in tail.new_text(),
            "a replaced (new-inode) log is read from the top",
        )

        # TRUNCATED IN PLACE: same inode, size drops below our offset.
        with path.open("r+b") as handle:
            handle.truncate(0)
        with path.open("ab") as handle:
            handle.write(b"short\n")
        check("short" in tail.new_text(), "a truncated-in-place log is re-read from the top")

        # PARTIAL LINE: a read landing mid-write must yield nothing, then the whole line.
        with path.open("ab") as handle:
            handle.write(f"{ARXAN_LINE_PREFIX} status=ok detected=".encode())
        check(tail.new_text() == "", "an unterminated line is withheld until it is complete")
        with path.open("ab") as handle:
            handle.write(b"true blocking_entrypoint=true\n")
        text = tail.new_text()
        check(
            text.strip() == f"{ARXAN_LINE_PREFIX} status=ok detected=true blocking_entrypoint=true",
            "the completed line arrives whole on the next poll",
        )
        check(tail.new_text() == "", "a line already handed back is not handed back twice")

    check(
        f'name = "{BUILT_DLL.stem}"' in (REPO_ROOT / "crates/ds2-loader/Cargo.toml").read_text(),
        f"the crate really is named to emit {BUILT_DLL.name}",
    )
    loader_src = (REPO_ROOT / "crates/ds2-loader/src/lib.rs").read_text()
    check(f'"{LOG_NAME}"' in loader_src, f"the DLL writes the log this polls for ({LOG_NAME})")
    check(f'"{ARXAN_LINE_PREFIX}"' in loader_src, "the DLL writes the line this gates on")
    check(f'"{ATTACH_LINE_PREFIX}"' in loader_src, "the DLL writes the attach line")

    print("selftest: " + ("OK" if ok else "FAILED"))
    return EXIT_OK if ok else EXIT_ERROR


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="report what a run would do; stage nothing, launch nothing",
    )
    parser.add_argument(
        "--selftest", action="store_true", help="test the log tailer and the DLL/script contract"
    )
    args = parser.parse_args()

    if args.selftest:
        return selftest()
    if args.dry_run:
        return dry_run()
    return launch()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
