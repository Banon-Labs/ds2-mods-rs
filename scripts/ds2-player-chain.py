#!/usr/bin/env python3
"""Walk `GameManagerImp -> player_ctrl -> position` in the live game and print every link.

    python3 scripts/ds2-player-chain.py
    python3 scripts/ds2-player-chain.py --selftest

WHY
---
The Frida agent reported `CHARACTER UNREADABLE 33878 times` while the loader log's last line was
`ds2-continue: silence restored by=start-ingame`, and those two facts point opposite ways: the
game says it entered the in-game substate, the agent says there is no character. Exactly one of
three things is true -- the world has not finished loading, the pointer chain is wrong, or the
read itself is failing -- and they send you to three different places. Guessing which, from a
counter that only says "null", is how a session goes sideways.

This reads the same chain the agent does, out of `/proc/<pid>/mem`, and prints each link with the
value it held. A null at a named link is an answer; a chain that resolves to a plausible position
while the agent reports nothing is a different answer entirely. Read-only: it opens the game's
memory for reading and writes nothing.

The constants are the ones `crates/ds2-rva` publishes, quoted here with their names so a drift
between this file and the crate is visible rather than silent.
"""

from __future__ import annotations

import argparse
import pathlib
import struct
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import ds2_run_lib

#: `ds2_rva::GAME_MANAGER_IMP` -- the static that holds the pointer, as an RVA off the image base.
GAME_MANAGER_IMP = 0x016148F0
#: `ds2_rva::PLAYER_CTRL_OFFSET`
PLAYER_CTRL_OFFSET = 0xD0
#: `ds2_rva::GAME_MANAGER_CHARACTER_MANAGER_OFFSET`
CHARACTER_MANAGER_OFFSET = 0x18
#: `ds2_rva::CHARACTER_CTRL_POSITION_OFFSET` -- three floats.
POSITION_OFFSET = 0x90


def read_floats(pid: int, address: int, count: int) -> list[float] | None:
    try:
        with open(f"/proc/{pid}/mem", "rb", buffering=0) as handle:
            handle.seek(address)
            raw = handle.read(4 * count)
    except OSError:
        return None
    if len(raw) != 4 * count:
        return None
    return list(struct.unpack(f"<{count}f", raw))


def walk(pid: int, base: int) -> int:
    print(f"  image base            0x{base:x}")
    static = base + GAME_MANAGER_IMP
    print(f"  GameManagerImp static 0x{static:x}  (base + 0x{GAME_MANAGER_IMP:x})")

    manager = ds2_run_lib.read_qword(pid, static)
    if manager is None:
        print("  GameManagerImp        COULD NOT READ -- the static itself is not mapped")
        return 1
    print(f"  GameManagerImp        0x{manager:x}")
    if manager == 0:
        print("  -> the game has no GameManagerImp yet: it is still booting.")
        return 1

    characters = ds2_run_lib.read_qword(pid, manager + CHARACTER_MANAGER_OFFSET)
    print(
        f"  CharacterManager      "
        + ("COULD NOT READ" if characters is None else f"0x{characters:x}")
        + f"  (+0x{CHARACTER_MANAGER_OFFSET:x})"
    )

    player = ds2_run_lib.read_qword(pid, manager + PLAYER_CTRL_OFFSET)
    if player is None:
        print(f"  player_ctrl           COULD NOT READ  (+0x{PLAYER_CTRL_OFFSET:x})")
        return 1
    print(f"  player_ctrl           0x{player:x}  (+0x{PLAYER_CTRL_OFFSET:x})")
    if player == 0:
        print(
            "  -> NULL. The chain is fine and there is genuinely no character: the game is at a\n"
            "     menu, on a loading screen, or the save has not finished coming up. Nothing to\n"
            "     fix in the agent -- wait for the world."
        )
        return 1

    position = read_floats(pid, player + POSITION_OFFSET, 3)
    if position is None:
        print(f"  position              COULD NOT READ  (+0x{POSITION_OFFSET:x})")
        return 1
    print(
        f"  position              {position[0]:.2f}, {position[1]:.2f}, {position[2]:.2f}"
        f"  (+0x{POSITION_OFFSET:x})"
    )
    print("  -> a character is in the world and the agent can read it.")
    return 0


def selftest() -> int:
    """Exercise the reader against this process, where the answers are known."""
    failures = 0

    def check(name: str, ok: bool, detail: str = "") -> None:
        nonlocal failures
        if not ok:
            failures += 1
        print(("  ok   " if ok else "  FAIL ") + name + ("  " + detail if detail else ""))

    import ctypes

    known = (ctypes.c_float * 3)(1.5, -2.25, 400.0)
    address = ctypes.addressof(known)
    got = read_floats(ds2_run_lib.__dict__.get("_self_pid", None) or __import__("os").getpid(),
                      address, 3)
    check("three floats read back out of our own memory", got == [1.5, -2.25, 400.0], str(got))
    check(
        "an unmapped address reads as None",
        read_floats(__import__("os").getpid(), 0x400000000000, 3) is None,
    )
    print("\n" + ("selftest: OK" if failures == 0 else f"selftest: {failures} FAILED"))
    return 0 if failures == 0 else 1


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--selftest", action="store_true")
    args = parser.parse_args()
    if args.selftest:
        return selftest()

    pid = ds2_run_lib.game_pid()
    if pid is None:
        print("no DarkSoulsII.exe is running")
        return 1
    print(f"[player-chain] pid {pid}")
    return walk(pid, ds2_run_lib.GAME_PREFERRED_IMAGE_BASE)


if __name__ == "__main__":
    sys.exit(main())
