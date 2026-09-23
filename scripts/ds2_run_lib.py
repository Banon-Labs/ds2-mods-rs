"""Shared primitives for the branch-launch pipeline: sleepless waits and run state.

Two rules shape everything here
-------------------------------
1. **No sleeps as synchronization** (`scripts/check-no-timeouts.py`). Readiness is an
   *event*; a timeout is only a safety backstop. So waiting for a log line blocks on inotify
   and waiting for a process to exit blocks on a pidfd -- in both cases `select()` returns
   the instant the thing happens, and the timeout exists solely so a wedged run cannot hang
   a caller forever.
2. **Every agent-facing shell op is capped at 30s.** Callers pass bounds under that; the
   waits below are written so a caller can loop over short bounded waits and stay responsive
   rather than asking for one long one.

This module exists because seven scripts in this directory each grew their own copy of the
same ctypes inotify block. New code in this pipeline uses this one instead of making it
eight.
"""

from __future__ import annotations

import ctypes
import ctypes.util
import errno
import json
import os
import select
from dataclasses import dataclass, field
from pathlib import Path

# <sys/inotify.h>
IN_MODIFY = 0x0000_0002
IN_CREATE = 0x0000_0100
IN_MOVED_TO = 0x0000_0080
IN_CLOSE_WRITE = 0x0000_0008
DEFAULT_MASK = IN_MODIFY | IN_CREATE | IN_MOVED_TO | IN_CLOSE_WRITE

RUN_STATE_ROOT = Path(
    os.environ.get("ER_RUN_STATE_DIR", Path.home() / ".cache" / "er-me3-runs")
)


class DirectoryWatch:
    """Block until something in `directory` changes, without polling.

    Watches the directory rather than a file: the DLL rotates its logs at startup
    (`<name>.log` -> `<name>.log.prev`), so a watch pinned to an inode would go deaf at
    exactly the moment the interesting run begins.

    Degrades honestly: if inotify is unavailable, `available` is False and `wait()` returns
    immediately, so a caller's own bounded re-check loop still makes progress instead of the
    wait silently blocking forever.
    """

    def __init__(self, directory: Path, mask: int = DEFAULT_MASK) -> None:
        self.directory = Path(directory)
        self.fd = -1
        self._libc = None
        try:
            self._libc = ctypes.CDLL(ctypes.util.find_library("c"), use_errno=True)
            self._libc.inotify_init1.argtypes = [ctypes.c_int]
            self._libc.inotify_init1.restype = ctypes.c_int
            self._libc.inotify_add_watch.argtypes = [
                ctypes.c_int,
                ctypes.c_char_p,
                ctypes.c_uint32,
            ]
            self._libc.inotify_add_watch.restype = ctypes.c_int
            fd = self._libc.inotify_init1(os.O_NONBLOCK | os.O_CLOEXEC)
            if fd >= 0:
                if self._libc.inotify_add_watch(fd, os.fsencode(self.directory), mask) < 0:
                    os.close(fd)
                else:
                    self.fd = fd
        except OSError:
            self.fd = -1

    @property
    def available(self) -> bool:
        return self.fd >= 0

    def wait(self, timeout: float) -> bool:
        """Return True if an event arrived, False on timeout. Never sleeps."""
        if self.fd < 0:
            return False
        try:
            ready, _, _ = select.select([self.fd], [], [], max(0.0, timeout))
        except OSError as err:
            if err.errno == errno.EINTR:
                return False
            raise
        if not ready:
            return False
        try:
            os.read(self.fd, 65536)  # drain; callers re-read their own state anyway
        except OSError:
            pass
        return True

    def close(self) -> None:
        if self.fd >= 0:
            os.close(self.fd)
            self.fd = -1

    def __enter__(self) -> DirectoryWatch:
        return self

    def __exit__(self, *_exc) -> None:
        self.close()


class WatchSet:
    """Watch several directories at once, so a caller can tail logs that do not share a parent.

    Lives here rather than in one launcher because more than one tool needs it: a run's crash
    records and the DLL's own logs land in different trees, and a waiter that watches only one of
    them goes deaf to the other.
    """

    def __init__(self, directories) -> None:
        seen: list[Path] = []
        for directory in directories:
            directory = Path(directory)
            if directory not in seen:
                seen.append(directory)
        self.watches = [DirectoryWatch(directory) for directory in seen]

    @property
    def available(self) -> bool:
        return any(watch.available for watch in self.watches)

    def wait(self, timeout: float) -> bool:
        """Return on the first event from any watched directory, or on the timeout.

        One `select` over every inotify fd, not a loop of per-directory waits: waiting on each in
        turn would spend the whole budget on the first quiet directory and never look at the second.
        """
        fds = [watch.fd for watch in self.watches if watch.fd >= 0]
        if not fds:
            return False
        try:
            ready, _, _ = select.select(fds, [], [], max(0.0, timeout))
        except OSError:
            return False
        for fd in ready:
            try:
                os.read(fd, 65536)
            except OSError:
                pass
        return bool(ready)

    def close(self) -> None:
        for watch in self.watches:
            watch.close()

    def __enter__(self) -> "WatchSet":
        return self

    def __exit__(self, *_exc) -> None:
        self.close()


def game_dir() -> Path:
    """The `ELDEN RING/Game` directory: where the game lives and where every loaded DLL logs.

    One owner, because the path is machine-shaped: this repo now runs a native Linux Steam
    install, the retired WSL2 layout put it under `C:\\SteamLibrary`, and a script carrying its
    own copy of either silently resolves to nothing rather than erroring -- which reads as "the
    file is missing" instead of "you looked in the wrong place". `ME3_STEAM_DIR` overrides it,
    the same variable `~/Elden/launch.sh` derives from.
    """
    steam = Path(os.environ.get("ME3_STEAM_DIR", Path.home() / ".local/share/Steam"))
    return steam / "steamapps/common/ELDEN RING/Game"


def process_alive(pid: int) -> bool:
    """True if `pid` exists and is not a zombie. Reads /proc rather than shelling out.

    `pgrep` is deliberately avoided: it embeds the target name in its own command line, so it
    matches itself and trips this repo's guard.
    """
    if pid <= 0:
        return False
    try:
        status = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8", errors="replace")
    except OSError:
        return False
    # "pid (comm) state ..." -- comm can contain spaces and parens, so split on the last ')'.
    try:
        return status[status.rindex(")") + 1 :].split()[0] != "Z"
    except (ValueError, IndexError):
        return False


GAME_PROCESS_NAMES = ("DarkSoulsII.exe",)


def find_game_pids(names: tuple[str, ...] = GAME_PROCESS_NAMES) -> list[int]:
    """PIDs of the running game, found by reading /proc directly.

    Deliberately not `pgrep`: `pgrep -f DarkSoulsII.exe` puts the pattern in its own command
    line and so matches itself, and this repo's guard blocks the bare form outright. Reading
    /proc has neither problem and costs nothing.

    Matches `comm` (truncated to 15 bytes by the kernel, so `DarkSoulsII.exe` fits) and, for
    Proton's wrapper processes, the full command line.
    """
    found = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            comm = (entry / "comm").read_text(encoding="utf-8", errors="replace").strip()
            if any(comm.lower() == name.lower() for name in names):
                found.append(int(entry.name))
                continue
            cmdline = (entry / "cmdline").read_bytes().replace(b"\0", b" ").decode(
                "utf-8", errors="replace"
            )
        except (OSError, ValueError):
            continue
        lowered = cmdline.lower()
        if any(name.lower() in lowered for name in names):
            found.append(int(entry.name))
    return found


def game_pid() -> int | None:
    """The Linux pid of `DarkSoulsII.exe`, or `None`.

    Stricter than [`find_game_pids`] on purpose. That one also matches a full command line, which
    catches me3's launcher because its argv names the game; this one matches `comm` exactly, so the
    pid it returns is the game process itself. Anything that will read the game's memory needs this
    one, because a read against the launcher resolves to rubble rather than failing.

    Wine reports the Windows executable name in `comm`; the `exe` symlink points at
    wine64-preloader for every Windows process in the prefix, so `comm` is the discriminator.
    """
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if (entry / "comm").read_text(encoding="utf-8").strip() == "DarkSoulsII.exe":
                return int(entry.name)
        except OSError:
            continue
    return None


# The base the game image is mapped at. DARK SOULS II's preferred base is honoured under Proton --
# every `/proc/<pid>/maps` for `DarkSoulsII.exe` on this target opens with a `140000000-` span, which
# is also the identity test `scripts/er-find-seamless-osm.py` uses to tell the game apart from the
# me3 launcher, whose argv names `DarkSoulsII.exe` too. It is read back out of the maps rather than
# assumed, so a build that ever did relocate answers "cannot tell" instead of reading rubble.
GAME_PREFERRED_IMAGE_BASE = 0x140000000

# `WorldChrMan`, the singleton the game builds when a world loads, as an rva into the installed
# 1.17.1 image.
#
# Two hops from the 1.16.2 constant `er_game_base::rva::WORLD_CHR_MAN_GLOBAL_RVA` (`0x3d65f88`),
# because an rva in that file is 1.16.2 by design and is translated at use rather than being stale:
#
#   1.16.2 -> 1.17.0   `docs/recon/rva-map-1162-to-1170.data.tsv`, row `WORLD_CHR_MAN_GLOBAL_RVA`:
#                      `0x3d65f88` -> `0x3d69ff8` on 2325 agreeing references out of 2330. The same
#                      pair is written out independently in
#                      `docs/recon/npc-possess-1170-address-table.md`.
#   1.17.0 -> 1.17.1   `python3 scripts/map-rvas-1170-to-1171.py 0x143d69ff8` answers `unchanged,
#                      .data did not move between the two builds`. That patch grew one function
#                      inside `.text` and left the section table alone, so no global moved.
#
# Then read back out of the images instead of trusted. In `DarkSoulsII-deobf-1.17.1.bin`, `0x1401bac8c`
# is `mov rax, qword ptr [rip + 0x3baf365]`, which resolves to `0x143d69ff8`, immediately followed by
# `mov r13, qword ptr [rax + 0x1e508]` and `test r13, r13` -- the game performing this exact walk and
# null-check. The same three instructions sit at the same address in `DarkSoulsII-deobf.bin` reading
# `0x143d65f88`, so the two builds are paired at one site rather than by two separate guesses. 2469
# instructions in 1.17.1 load this global, against 2464 loading the 1.16.2 one.
#
# It lives here rather than in either caller because two scripts gate on it. A constant of this
# provenance copied into a second file is a constant that goes stale in one of them silently, and
# the failure it produces -- an address that resolves to something that is not a world -- reads as
# an empty world forever rather than as a wrong address.
WORLD_CHR_MAN_GLOBAL_RVA = 0x3D69FF8

# `WorldChrMan::mainPlayerIns`, the local player inside that singleton. A struct offset, so nothing
# translates it and nothing would notice the day it moves: recorded unchanged from 1.16.2 through
# 1.17 in `docs/recon/npc-possess-1170-address-table.md`, and 410 instructions read `[reg+0x1e508]`
# in each 1.17 image against 411 in 1.16.2.
WORLD_CHR_MAN_MAIN_PLAYER_OFFSET = 0x1E508

# What a qword has to look like to be an object pointer at all. The canonical-address split puts
# every user-mode pointer on x86-64 below `1 << 47`, and nothing the game allocates lives in the
# first 64 KB. A value outside this range means the address resolution is wrong, which is a
# different answer from "no player exists" and has to stay distinguishable from it.
MIN_OBJECT_POINTER = 0x1_0000
MAX_OBJECT_POINTER = 1 << 47

# How long the world-read selftest's own child gets to die after it is killed. A backstop on a reap,
# not a budget: the child is a python interpreter blocked on `stdin` and it goes immediately.
SELFTEST_CHILD_REAP_SECONDS = 5


def game_image_base(pid: int) -> int | None:
    """Where `DarkSoulsII.exe` is mapped in `pid`, or `None` when the image is not there.

    Wine gives a PE mapping no backing filename, so the span is recognised by its address: the game
    image is the one that starts at its preferred base. Answering `None` when that span is absent
    keeps a read off a process whose layout this script cannot account for.
    """
    try:
        maps = Path(f"/proc/{pid}/maps").read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None
    for line in maps.splitlines():
        span = line.split(maxsplit=1)[0]
        low, _, high = span.partition("-")
        try:
            start = int(low, 16)
            end = int(high, 16)
        except ValueError:
            continue
        if start == GAME_PREFERRED_IMAGE_BASE and end > start:
            return start
    return None


def read_qword(pid: int, address: int) -> int | None:
    """One 8-byte read out of a live process, or `None` when the address is not mapped.

    The mechanism `scripts/er-live-fields.py` documents: `/proc/<pid>/mem` opened read-only and
    seeked. Nothing is injected, no thread is suspended and no code runs inside the game, so this
    cannot disturb a session the way an attach can. Buffering is off because a buffered reader would
    read ahead past the requested qword into an unmapped neighbouring page and turn a good read into
    an error.
    """
    if not MIN_OBJECT_POINTER <= address < MAX_OBJECT_POINTER:
        return None
    try:
        with open(f"/proc/{pid}/mem", "rb", 0) as mem:
            mem.seek(address)
            data = mem.read(8)
    except (OSError, ValueError, OverflowError):
        return None
    if not data or len(data) != 8:
        return None
    return int.from_bytes(data, "little")


def player_in_a_world_at(pid: int, base: int) -> tuple[bool | None, str]:
    """Walk `WorldChrMan -> mainPlayerIns` in `pid`, given where the image sits.

    The verdict is deliberately three-valued. `True` and `False` both mean the walk completed;
    `None` means it could not be made -- an unmapped address, a refused read, or a qword that is not
    a pointer at all -- and that is a different refusal from "there is no player", because one says
    the game has no world yet and the other says this script cannot see.

    `base` is a parameter rather than a lookup so the selftest can point the same walk at a planted
    chain in a child process and exercise the arithmetic, the read and all three verdicts.
    """
    address = base + WORLD_CHR_MAN_GLOBAL_RVA
    world = read_qword(pid, address)
    if world is None:
        return None, f"WorldChrMan at 0x{address:x} could not be read in pid {pid}"
    if world == 0:
        return False, f"WorldChrMan at 0x{address:x} is null -- no world is loaded"
    if not MIN_OBJECT_POINTER <= world < MAX_OBJECT_POINTER:
        return None, (
            f"WorldChrMan at 0x{address:x} reads 0x{world:x}, which is not an object pointer -- "
            "the address resolution is wrong, not the world"
        )
    player_slot = world + WORLD_CHR_MAN_MAIN_PLAYER_OFFSET
    player = read_qword(pid, player_slot)
    if player is None:
        return None, (
            f"WorldChrMan is 0x{world:x} but its mainPlayerIns at 0x{player_slot:x} could not "
            "be read"
        )
    if player == 0:
        return False, f"WorldChrMan 0x{world:x} holds no mainPlayerIns -- no player in a world"
    if not MIN_OBJECT_POINTER <= player < MAX_OBJECT_POINTER:
        return None, (
            f"mainPlayerIns at 0x{player_slot:x} reads 0x{player:x}, which is not an object pointer"
        )
    return True, f"WorldChrMan 0x{world:x} -> mainPlayerIns 0x{player:x}"


def player_in_a_world(pid: int) -> tuple[bool | None, str]:
    """The same walk, against the image base found in `pid`'s own maps."""
    base = game_image_base(pid)
    if base is None:
        return None, (
            f"pid {pid} has no game image mapped at 0x{GAME_PREFERRED_IMAGE_BASE:x}, so no "
            "address in it can be resolved"
        )
    return player_in_a_world_at(pid, base)


def recorded_1162_to_1170_hop() -> tuple[bool | None, str]:
    """Re-read the first translation hop out of the map that produced it.

    `WORLD_CHR_MAN_GLOBAL_RVA` above is the far end of a two-hop carry, and the near end is a
    generated file that gets regenerated. If a refresh ever moves the row, this constant is wrong
    and nothing else here would notice, so the selftests read the row back. A missing map file is
    reported as unchecked rather than as a pass: these scripts also run from outside a checkout.
    """
    row_file = Path(__file__).resolve().parent.parent / "docs/recon/rva-map-1162-to-1170.data.tsv"
    if not row_file.is_file():
        return None, f"{row_file.name} is not here, so the 1.16.2 to 1.17.0 hop went unchecked"
    for line in row_file.read_text(encoding="utf-8", errors="replace").splitlines():
        fields = line.split("\t")
        if len(fields) >= 3 and fields[2].strip() == "WORLD_CHR_MAN_GLOBAL_RVA":
            mapped = int(fields[1].strip(), 16)
            if mapped == WORLD_CHR_MAN_GLOBAL_RVA:
                return True, f"{fields[0].strip()} -> 0x{mapped:x}, votes {fields[3].strip()}"
            return False, (
                f"the map now carries {fields[0].strip()} -> 0x{mapped:x}, not "
                f"0x{WORLD_CHR_MAN_GLOBAL_RVA:x}"
            )
    return False, "the map has no WORLD_CHR_MAN_GLOBAL_RVA row any more"


def world_read_selftest() -> list[tuple[str, bool]]:
    """Exercise the live-read witness end to end against a planted chain in a child process.

    Shared rather than duplicated: both `scripts/er-frida-up.py` and
    `scripts/er-frida-when-world.py` gate on this walk, and a second copy of the coverage is a
    second copy that can drift out of step with the constants above.

    A child is the target rather than this process, so the real cross-process read is what runs
    rather than a same-process shortcut that would pass for the wrong reason. The child plants three
    `WorldChrMan` slots -- one reaching a player, one reaching a world with no player, one null --
    and the walk is pointed at each by handing it a base that puts `WORLD_CHR_MAN_GLOBAL_RVA` on
    that slot. That covers the arithmetic, the read, and all three verdicts.

    What it cannot cover is the game: the offsets themselves are ground-truthed statically against
    the deobfuscated images, and whether they still name a live `WorldChrMan` is only settled by a
    read of a running `DarkSoulsII.exe`.
    """
    import subprocess
    import sys

    child_source = (
        "import ctypes, struct, sys\n"
        f"world_with = ctypes.create_string_buffer({WORLD_CHR_MAN_MAIN_PLAYER_OFFSET + 0x10})\n"
        f"world_without = ctypes.create_string_buffer({WORLD_CHR_MAN_MAIN_PLAYER_OFFSET + 0x10})\n"
        "player = ctypes.create_string_buffer(64)\n"
        f"struct.pack_into('<Q', world_with, {WORLD_CHR_MAN_MAIN_PLAYER_OFFSET},"
        " ctypes.addressof(player))\n"
        "slots = ctypes.create_string_buffer(24)\n"
        "struct.pack_into('<Q', slots, 0, ctypes.addressof(world_with))\n"
        "struct.pack_into('<Q', slots, 8, ctypes.addressof(world_without))\n"
        "struct.pack_into('<Q', slots, 16, 0)\n"
        "print(ctypes.addressof(slots), flush=True)\n"
        "sys.stdin.readline()\n"
    )
    child = subprocess.Popen(
        [sys.executable, "-c", child_source],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        text=True,
    )
    results: list[tuple[str, bool]] = []
    try:
        line = child.stdout.readline().strip() if child.stdout else ""
        if not line.isdigit():
            return [(f"the selftest child reported an address (got {line!r})", False)]
        slots = int(line)
        cases = [
            ("a planted mainPlayerIns reads as a player in a world", slots, True),
            ("a planted null mainPlayerIns reads as no player", slots + 8, False),
            ("a null WorldChrMan reads as no world", slots + 16, False),
            # Well above anything this child maps, so the read is refused rather than answered.
            ("an unmapped WorldChrMan answers `cannot tell`, not `no player`", 1 << 46, None),
        ]
        for label, slot, want in cases:
            verdict, detail = player_in_a_world_at(child.pid, slot - WORLD_CHR_MAN_GLOBAL_RVA)
            results.append((f"{label} -- {detail}", verdict is want))
    finally:
        child.kill()
        child.wait(timeout=SELFTEST_CHILD_REAP_SECONDS)
    return results


def wait_for_exit(pid: int, timeout: float) -> bool:
    """Block until `pid` exits or `timeout` elapses. Returns True if it exited.

    Uses a pidfd so the wait is edge-triggered on the actual exit, with no polling and no
    requirement that the caller be the process's parent -- which matters here, because the
    reaper deliberately is not.
    """
    if not process_alive(pid):
        return True
    try:
        pidfd = os.pidfd_open(pid, 0)
    except (OSError, AttributeError):
        # No pidfd: fall back to a liveness re-check, which the caller's loop drives.
        return not process_alive(pid)
    try:
        ready, _, _ = select.select([pidfd], [], [], max(0.0, timeout))
        return bool(ready)
    except OSError as err:
        if err.errno == errno.EINTR:
            return not process_alive(pid)
        raise
    finally:
        os.close(pidfd)


@dataclass
class RunState:
    """What a launched run left behind, and what has to be undone when it ends.

    Written before the launch and consumed by whoever gets there first -- the detached
    reaper on a clean exit, or the next invocation's garbage collection if the reaper never
    ran (SIGKILL, reboot, the stale-run sentinel tearing the game down from a hook). The
    cleanup is therefore idempotent by construction: it is a list of paths to remove, and
    removing an absent path is success.
    """

    run_id: str
    pid: int = 0
    profile: str = ""
    remove_paths: list[str] = field(default_factory=list)
    meta: dict = field(default_factory=dict)

    @property
    def directory(self) -> Path:
        return RUN_STATE_ROOT / self.run_id

    @property
    def state_file(self) -> Path:
        return self.directory / "run.json"

    def save(self) -> None:
        self.directory.mkdir(parents=True, exist_ok=True)
        payload = {
            "run_id": self.run_id,
            "pid": self.pid,
            "profile": self.profile,
            "remove_paths": self.remove_paths,
            "meta": self.meta,
        }
        tmp = self.state_file.with_suffix(".tmp")
        tmp.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
        tmp.replace(self.state_file)  # atomic: a torn state file is an uncleanable run

    @classmethod
    def load(cls, state_file: Path) -> RunState | None:
        try:
            payload = json.loads(state_file.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            return None
        return cls(
            run_id=payload.get("run_id", state_file.parent.name),
            pid=int(payload.get("pid", 0)),
            profile=payload.get("profile", ""),
            remove_paths=list(payload.get("remove_paths", [])),
            meta=dict(payload.get("meta", {})),
        )

    def cleanup(self) -> list[str]:
        """Remove everything this run staged. Returns what was actually removed.

        What the run wrote is not staged and is never removed. The same directory now holds the
        run's evidence -- me3's output, and every DLL log/telemetry file redirected here at launch
        so the next launch cannot overwrite it the way a game-directory log is overwritten. Only
        the explicit `remove_paths` (profile, sidecar, closure/save inputs) and `run.json` go.

        The `rmdir` below therefore removes the directory only in the one case where that is
        correct: a run that produced nothing at all. It fails harmlessly on a directory holding
        evidence, which is the normal outcome and the whole point.
        """
        removed: list[str] = []
        for raw in self.remove_paths:
            path = Path(raw)
            try:
                path.unlink()
                removed.append(raw)
            except FileNotFoundError:
                continue
            except OSError:
                continue
        try:
            self.state_file.unlink()
        except OSError:
            pass
        try:
            self.directory.rmdir()  # empty-only; a run with artifacts keeps its directory
        except OSError:
            pass
        return removed


def all_run_states(root: Path = RUN_STATE_ROOT) -> list[RunState]:
    if not root.is_dir():
        return []
    states = []
    for state_file in sorted(root.glob("*/run.json")):
        state = RunState.load(state_file)
        if state is not None:
            states.append(state)
    return states


def collect_dead_runs(root: Path = RUN_STATE_ROOT) -> list[tuple[str, list[str]]]:
    """Clean up after every run whose process is gone. Returns [(run_id, removed paths)].

    This -- not the reaper -- is what makes cleanup a guarantee. The reaper is the fast path
    and can be killed; this runs at the start of every launch, so a leftover survives at most
    until the next one.
    """
    collected = []
    for state in all_run_states(root):
        if state.pid and process_alive(state.pid):
            continue
        collected.append((state.run_id, state.cleanup()))
    return collected


def selftest() -> int:
    """Exercise every primitive, including that the waits return on the event not the timeout."""
    import subprocess
    import tempfile
    import time

    ok = True

    def check(condition: bool, label: str) -> None:
        nonlocal ok
        if not condition:
            ok = False
        print(("  ok   " if condition else "  FAIL ") + label)

    check(process_alive(os.getpid()), "process_alive sees our own pid")
    check(not process_alive(999_999_999), "process_alive rejects a nonexistent pid")

    with tempfile.TemporaryDirectory() as raw:
        directory = Path(raw)
        with DirectoryWatch(directory) as watch:
            check(watch.available, "inotify watch initialises")
            check(not watch.wait(0.05), "wait() times out cleanly when nothing happens")
            (directory / "evidence.log").write_text("x", encoding="utf-8")
            started = time.monotonic()
            check(watch.wait(5.0), "wait() returns on a real file event")
            check(
                time.monotonic() - started < 1.0,
                "the file wait returned on the EVENT, not by exhausting its cap",
            )

    child = subprocess.Popen(["/bin/true"])
    started = time.monotonic()
    check(wait_for_exit(child.pid, 5.0), "wait_for_exit returns when the child exits")
    check(
        time.monotonic() - started < 2.0,
        "the exit wait returned on the EVENT, not by exhausting its cap",
    )
    child.wait()

    with tempfile.TemporaryDirectory() as raw:
        root = Path(raw)
        staged = root / "staged.me3"
        staged.write_text("profileVersion = \"v1\"\n", encoding="utf-8")

        global RUN_STATE_ROOT
        previous_root, RUN_STATE_ROOT = RUN_STATE_ROOT, root
        try:
            state = RunState(
                run_id="selftest",
                pid=999_999_999,
                profile=str(staged),
                remove_paths=[str(staged)],
            )
            state.save()
            check(state.state_file.is_file(), "run state saves atomically")

            loaded = RunState.load(state.state_file)
            check(
                loaded is not None and loaded.remove_paths == [str(staged)],
                "run state round-trips through disk",
            )

            collected = collect_dead_runs(root)
            check(
                any(run_id == "selftest" for run_id, _ in collected),
                "GC collects a run whose process is gone",
            )
            check(not staged.exists(), "GC actually removed the staged file")
            check(collect_dead_runs(root) == [], "GC is idempotent -- a second pass finds nothing")

            live = RunState(
                run_id="live", pid=os.getpid(), profile="", remove_paths=[str(root / "keep")]
            )
            (root / "keep").write_text("x", encoding="utf-8")
            live.save()
            collect_dead_runs(root)
            check((root / "keep").exists(), "GC leaves a run whose process is still alive alone")

            # The evidence must survive cleanup. A run's artifacts are redirected into its state
            # directory at launch (er-run-branch.py's ARTIFACT_ENV), so a cleanup that removed the
            # directory would destroy exactly what the redirect exists to keep -- and would do it
            # to a finished run, at the moment someone came back to read it.
            evidence = RunState(
                run_id="finished",
                pid=999_999_998,
                profile=str(root / "finished.me3"),
                remove_paths=[str(root / "finished.me3")],
            )
            (root / "finished.me3").write_text("staged", encoding="utf-8")
            evidence.save()
            (evidence.directory / "er-quickload-continue-trace.log").write_text(
                "RUN EVIDENCE\n", encoding="utf-8"
            )
            evidence.cleanup()
            check(
                not (root / "finished.me3").exists(),
                "cleanup still removes what the run STAGED",
            )
            check(
                not evidence.state_file.exists(),
                "cleanup still removes run.json, so GC does not keep rediscovering the run",
            )
            check(
                (evidence.directory / "er-quickload-continue-trace.log").read_text(
                    encoding="utf-8"
                )
                == "RUN EVIDENCE\n",
                "cleanup does NOT remove what the run WROTE -- the artifacts outlive the run",
            )
        finally:
            RUN_STATE_ROOT = previous_root

    print("selftest:", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    import sys

    sys.exit(selftest())
