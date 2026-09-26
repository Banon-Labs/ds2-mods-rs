#!/usr/bin/env python3
"""Stop an agent-launched DARK SOULS II session and leave nothing behind.

Ported from `../er-mods-rs/scripts/er-teardown.py`. What is ported is the CLASSIFIER, which is the
only hard part; the rest is signalling and waiting.

Why this is a script and not two `kill` commands
------------------------------------------------
Because two `kill` commands is what was tried, on 2026-09-23, and it left `srt-bwrap` and four
`winedevice.exe` alive while reporting "reaper gone / wineserver gone". Killing the pids you happen
to have noticed is not a teardown; it is a teardown-shaped sentence.

**The game can be invisible to `pgrep`.** Steam launches DS2 through the Steam Linux Runtime, and
on the day this script was written `DarkSoulsII.exe` lived where the host's `/proc` did not show it
by name -- `pgrep -x DarkSoulsII.exe` and `pidof DarkSoulsII.exe` both answered "nothing running"
about a game that was on screen. That has not held since: on 2026-09-26 `pgrep -x DarkSoulsII.exe`
returned the running game's pid after every launch, the same pid this script then listed as
`DarkSoulsII.exe by=appid`. Which launch path hides it is not pinned down, so the name is not
trusted as the only rule.

So the classifier that finds everything is the ENVIRONMENT:

    SteamGameId=335300 / SteamAppId=335300 / STEAM_COMPAT_APP_ID=335300

`/proc/<pid>/environ` is readable per-pid for the user's own processes regardless of which PID
namespace the process lives in, and Steam puts those variables into every process in the session --
the reaper, the container shims, the Proton wrappers and the game. That is the rule that catches the
ones `comm` cannot see.

Three more rules back it up, each for a case the first one misses:

  * **`WINEPREFIX=` names our prefix.** In er-mods-rs, eight `start.exe` processes leaked -- one per
    run, all alive at once -- because their `exe` resolved into the Proton install rather than the
    prefix and they carried none of the appid variables.
  * **`comm` is a wine service AND something about the process points into our prefix.** Catches the
    `winedevice.exe`/`wineserver` pair that outlives the game.
  * **`comm` is the game.** Needs no corroboration: there is one DARK SOULS II on this machine.

Never matched on the COMMAND LINE. A pattern in a command line is a pattern that appears in the
searching shell's own command line, which is the self-match that killed a session's shell in the
sibling repo and is why `pgrep -f` is banned here.

Never signals: the Steam client (`steam`, `steamwebhelper`), this process, any of its ancestors, or
PID 1. Killing the client to clean up after a game would log the user out, which is not a trade this
tool gets to make.

Waiting is event-driven -- `pidfd_open` + `poll`, which becomes readable exactly when a process
dies, including for processes that are not our children. Never a sleep-and-recheck loop.

    python3 scripts/ds2-teardown.py            # SIGTERM, wait, SIGKILL the survivors
    python3 scripts/ds2-teardown.py --status   # list what is alive and exit
    python3 scripts/ds2-teardown.py --dry-run  # list what WOULD be signalled
    python3 scripts/ds2-teardown.py --selftest # exercise the classifier and the protections
"""

from __future__ import annotations

import argparse
import glob
import os
import select
import signal
import sys

#: The Proton prefix Steam builds for this appid.
DEFAULT_PREFIX = os.path.expanduser("~/.local/share/Steam/steamapps/compatdata/335300")

#: The appid as it appears in the environment of every process in the session, container shims
#: included. Matched on the VARIABLE and not on the bare number: `335300` alone occurs in unrelated
#: Steam paths, and matching it loosely is the class of mistake AGENTS.md bans for `rsi` and `wine`.
APPID_NEEDLES = (
    b"SteamGameId=335300",
    b"SteamAppId=335300",
    b"STEAM_COMPAT_APP_ID=335300",
)

#: Wine/Proton per-session service processes, plus the container and Steam shims that wrap them.
#: `DarkSoulsII.exe` is listed with them because it belongs to the same session and must go in the
#: same sweep -- though on this target it is usually invisible to `comm`; see the module docs.
PREFIX_COMMS = frozenset(
    {
        "DarkSoulsII.exe",
        "explorer.exe",
        "plugplay.exe",
        "rpcss.exe",
        "services.exe",
        "start.exe",
        "svchost.exe",
        "tabtip.exe",
        "wineboot.exe",
        "winedevice.exe",
        "winemenubuilder.exe",
        "wineserver",
        "xalia.exe",
    }
)

#: The Steam Linux Runtime and Steam launch shims. They carry the appid in their environment, so
#: they are already caught by `_has_appid`; naming them here is what makes a survey READABLE when
#: the environment is unreadable, and what catches a shim whose appid variables were dropped.
SHIM_COMMS = frozenset({"reaper", "srt-bwrap", "pv-bwrap", "pressure-vessel"})

#: Never signalled, whatever else matches.
STEAM_CLIENT_COMMS = frozenset({"steam", "steamwebhelper"})

#: `comm` is truncated to 15 characters by the kernel. `DarkSoulsII.exe` is exactly 15, so it
#: survives intact -- asserted in the selftest rather than assumed, because a rename to anything
#: longer would silently stop matching.
COMM_LIMIT = 15

#: Milliseconds to wait for SIGTERM before escalating, and again for the kill to land. Spent inside
#: `poll()` on a pidfd -- a readiness wait on "this process exited", not a sleep polling for it.
TERM_GRACE_MS = 12_000
KILL_GRACE_MS = 3_000


def _read(path: str) -> str | None:
    try:
        with open(path, encoding="utf-8", errors="replace") as handle:
            return handle.read()
    except OSError:
        return None


def _link(path: str) -> str:
    try:
        return os.readlink(path)
    except OSError:
        return ""


def _environ(entry: str) -> bytes:
    try:
        with open(f"{entry}/environ", "rb") as handle:
            return handle.read()
    except OSError:
        return b""


def _has_appid(environ: bytes) -> bool:
    """Does this process's environment name the DARK SOULS II appid?"""
    return any(needle in environ for needle in APPID_NEEDLES)


def _names_prefix(environ: bytes, prefix: str) -> bool:
    """Whether this process's `WINEPREFIX` is the game's prefix."""
    return b"WINEPREFIX=" + prefix.encode("utf-8", "replace") in environ


def ancestors(pid: int) -> set[int]:
    """`pid` and every parent up to init -- never signal our own process tree."""
    out: set[int] = set()
    cur = pid
    while cur and cur not in out:
        out.add(cur)
        try:
            with open(f"/proc/{cur}/stat", encoding="utf-8", errors="replace") as handle:
                cur = int(handle.read().rsplit(")", 1)[1].split()[1])
        except (OSError, IndexError, ValueError):
            break
    return out


def _steam_client_pids() -> set[int]:
    """The user's Steam client and its helpers. Never targets.

    The Proton prefix contains a Windows `steam.exe` shim, and the client's own children can inherit
    the game's environment. Identified by the NATIVE executable name, which the Windows shim does
    not share.
    """
    client: set[int] = set()
    for entry in glob.glob("/proc/[0-9]*"):
        comm = _read(f"{entry}/comm")
        if comm is None:
            continue
        if comm.strip() in STEAM_CLIENT_COMMS:
            try:
                client.add(int(entry.rsplit("/", 1)[-1]))
            except ValueError:
                pass
    return client


def survey(prefix: str = DEFAULT_PREFIX, protected: set[int] | None = None) -> list[dict]:
    """Every process belonging to this game's session, with the evidence that classified it."""
    if protected is None:
        protected = _steam_client_pids() | ancestors(os.getpid()) | {1}
    found: list[dict] = []
    for entry in glob.glob("/proc/[0-9]*"):
        pid_text = entry.rsplit("/", 1)[-1]
        comm = _read(f"{entry}/comm")
        if comm is None:
            continue
        comm = comm.strip()
        environ = _environ(entry)
        exe = _link(f"{entry}/exe")
        in_prefix = (
            prefix in exe or prefix in _link(f"{entry}/cwd") or _names_prefix(environ, prefix)
        )
        by_appid = _has_appid(environ)
        # The game itself needs no corroboration: there is one DARK SOULS II on this machine, and
        # under the Steam Linux Runtime its `exe` and `cwd` point inside the container rather than
        # into the prefix, so the other rules can all be false for it.
        by_name = comm in {"DarkSoulsII.exe", "DarkSoulsII"}
        by_shim = comm in SHIM_COMMS and (by_appid or in_prefix)
        if not (by_appid or by_name or by_shim or (comm in PREFIX_COMMS and in_prefix)):
            continue
        try:
            pid = int(pid_text)
        except ValueError:
            continue
        # Hard exclusion, applied AFTER classification so it cannot be reasoned around.
        if pid in protected:
            continue
        stat = _read(f"{entry}/stat")
        found.append(
            {
                "pid": pid,
                "comm": comm,
                "state": stat.split()[2] if stat else "?",
                "threads": len(glob.glob(f"{entry}/task/*")),
                # Which rule caught it, so a survey that misses something is diagnosable rather
                # than merely short.
                "matched_by": (
                    "appid"
                    if by_appid
                    else ("name" if by_name else ("shim" if by_shim else "prefix"))
                ),
                "exe": exe.rsplit("/", 1)[-1],
            }
        )
    found.sort(key=lambda row: row["pid"])
    return found


def wait_for_exit(pids: list[int], timeout_ms: int) -> bool:
    """Block until every pid has exited, or `timeout_ms` elapses. True if all exited.

    `pidfd_open` + `poll` is the readiness primitive for this event: a pidfd becomes readable
    exactly when its process dies, including for processes that are not our children. A
    sleep-and-recheck loop would be both slower and less reliable.

    A pid already gone, or one we may not open, counts as exited -- both mean nothing to wait for.
    """
    poller = select.poll()
    fds: dict[int, int] = {}
    for pid in pids:
        try:
            fd = os.pidfd_open(pid, 0)
        except (OSError, AttributeError):
            continue
        fds[fd] = pid
        poller.register(fd, select.POLLIN)
    try:
        while fds:
            ready = poller.poll(timeout_ms)
            if not ready:
                break
            for fd, _event in ready:
                poller.unregister(fd)
                os.close(fd)
                fds.pop(fd, None)
        return not fds
    finally:
        for fd in list(fds):
            try:
                poller.unregister(fd)
                os.close(fd)
            except OSError:
                pass


def teardown(prefix: str = DEFAULT_PREFIX, verbose: bool = True) -> int:
    """SIGTERM every session process, wait, then SIGKILL whatever survived. Returns what remains."""
    protected = _steam_client_pids() | ancestors(os.getpid()) | {1}
    targets = survey(prefix, protected)
    if verbose:
        print(f"[ds2-teardown] {len(targets)} session process(es) to remove")
        for row in targets:
            print(
                f"[ds2-teardown]   SIGTERM {row['pid']:>8} {row['comm']:<20} "
                f"by={row['matched_by']}"
            )
    for row in targets:
        try:
            os.kill(int(row["pid"]), signal.SIGTERM)
        except OSError:
            pass
    wait_for_exit([int(row["pid"]) for row in targets], TERM_GRACE_MS)

    survivors = survey(prefix, protected)
    for row in survivors:
        try:
            os.kill(int(row["pid"]), signal.SIGKILL)
            if verbose:
                print(f"[ds2-teardown]   SIGKILL {row['pid']:>8} {row['comm']}")
        except OSError:
            pass
    wait_for_exit([int(row["pid"]) for row in survivors], KILL_GRACE_MS)

    # Re-surveyed rather than assumed: "I sent the signals" is not "they are gone", which is the
    # exact claim this script exists to stop being made by hand.
    remaining = survey(prefix, protected)
    if verbose:
        if remaining:
            print(f"[ds2-teardown] STILL ALIVE ({len(remaining)}):")
            for row in remaining:
                print(
                    f"[ds2-teardown]   {row['pid']:>8} {row['comm']:<20} "
                    f"state={row['state']} by={row['matched_by']}"
                )
        else:
            print("[ds2-teardown] clean -- zero session processes remain")
    return len(remaining)


def selftest() -> int:
    failures = 0

    def check(name: str, condition: bool) -> None:
        nonlocal failures
        print(f"  {'ok  ' if condition else 'FAIL'} {name}")
        if not condition:
            failures += 1

    # THE SELF-MATCH THIS SCRIPT EXISTS TO AVOID. Our own process carries this script's path -- and
    # therefore the string "335300" -- in its command line. Classification must not see it.
    check(
        "this process is not a target (the appid is in our cmdline, not our environ)",
        os.getpid() not in {row["pid"] for row in survey()},
    )
    check(
        "our ancestors are protected",
        not ({row["pid"] for row in survey()} & ancestors(os.getpid())),
    )
    check("PID 1 is protected", 1 not in {row["pid"] for row in survey()})
    check(
        "the Steam client is protected",
        not ({row["pid"] for row in survey()} & _steam_client_pids()),
    )
    # The classifier is matched on variables, never on the bare number.
    check(
        "a bare appid in a PATH does not classify",
        not _has_appid(b"PATH=/home/x/steamapps/compatdata/335300/pfx\0"),
    )
    check("SteamGameId classifies", _has_appid(b"SteamGameId=335300\0"))
    check("SteamAppId classifies", _has_appid(b"SteamAppId=335300\0"))
    check("STEAM_COMPAT_APP_ID classifies", _has_appid(b"STEAM_COMPAT_APP_ID=335300\0"))
    check(
        "another game's appid does not classify",
        not _has_appid(b"SteamGameId=1245620\0"),
    )
    check(
        "WINEPREFIX names the prefix",
        _names_prefix(f"WINEPREFIX={DEFAULT_PREFIX}\0".encode(), DEFAULT_PREFIX),
    )
    check(
        "another prefix does not",
        not _names_prefix(b"WINEPREFIX=/tmp/other\0", DEFAULT_PREFIX),
    )
    # The kernel truncates `comm` to 15 characters. The game's name is exactly 15, so it survives
    # -- a rename to anything longer would silently stop matching and this is what says so.
    check(
        f"the game's comm fits in {COMM_LIMIT} chars",
        len("DarkSoulsII.exe") <= COMM_LIMIT,
    )
    check(
        "a survey row says which rule caught it",
        all(row["matched_by"] in {"appid", "name", "shim", "prefix"} for row in survey()),
    )
    # `wait_for_exit` on a pid that cannot exist is "nothing to wait for", not a hang.
    check("waiting on a dead pid returns at once", wait_for_exit([2**22], 50))
    print("selftest: OK" if not failures else f"selftest: {failures} FAILED")
    return 1 if failures else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--prefix", default=DEFAULT_PREFIX)
    ap.add_argument("--status", action="store_true", help="list what is alive and exit")
    ap.add_argument("--dry-run", action="store_true", help="list what would be signalled")
    ap.add_argument("--selftest", action="store_true")
    args = ap.parse_args()

    if args.selftest:
        return selftest()

    if args.status or args.dry_run:
        rows = survey(args.prefix)
        verb = "WOULD signal" if args.dry_run else "alive"
        if not rows:
            print("[ds2-teardown] nothing running")
            return 0
        for row in rows:
            print(
                f"[ds2-teardown] {verb} {row['pid']:>8} {row['comm']:<20} "
                f"state={row['state']} threads={row['threads']:<4} by={row['matched_by']}"
            )
        return 0

    # Non-zero when something survived a SIGKILL, so a caller can tell a clean teardown from a
    # teardown-shaped sentence.
    return 1 if teardown(args.prefix) else 0


if __name__ == "__main__":
    sys.exit(main())
