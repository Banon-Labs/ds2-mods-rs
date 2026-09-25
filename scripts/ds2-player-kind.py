#!/usr/bin/env python3
"""Name every `PlayerCtrl` in the live roster, and print the bytes that say what it is.

    python3 scripts/ds2-player-kind.py
    python3 scripts/ds2-player-kind.py --selftest

WHY
---
`ds2-invasion-path` decides who to draw a route to by reading one byte, and a count in its log
cannot be checked against what is on the screen. `roster: players=3 remotes=2 phantoms=0` is
equally consistent with "two people are in your session" and with "two recordings walked past and
the test missed them", and those send you to different places.

This prints, for every object in the roster whose vtable is `PlayerCtrl`'s, the three things that
separate those two readings:

  name             `CharacterCtrl+0x118`, an MSVC `std::wstring`. The factory's own label, and one
                   of `Player_%06u` (local, `0x140357920`), `NetworkPlayer_%06u` or
                   `GhostPlayer_%06u` (both `0x1403572e0`).
  spawn kind       `CharacterCtrl+0x54`, written by `0x1403161b0` from the spawn request's `+0x29`.
  phantom param    `*(CharacterCtrl+0xb0) + 0x3c`. `0x14016f740` calls the character a replay when
                   this is `0x12` or `0x13`, and the replay spawner `0x1401a0d20` writes exactly
                   that pair.
  team type        `*(CharacterCtrl+0xb0) + 0x3d`, from the same params struct's `+0xaf`. Printed
                   because it is the other byte that init assigns and nothing here has read it yet.

Read-only: it opens the game's memory for reading and writes nothing.

The constants are the ones `crates/ds2-rva` publishes, quoted with their names so a drift between
this file and the crate is visible rather than silent.
"""

from __future__ import annotations

import argparse
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import ds2_run_lib

#: `ds2_rva::GAME_MANAGER_IMP`
GAME_MANAGER_IMP = 0x016148F0
#: `ds2_rva::PLAYER_CTRL_OFFSET`
PLAYER_CTRL_OFFSET = 0xD0
#: `ds2_rva::GAME_MANAGER_CHARACTER_MANAGER_OFFSET`
CHARACTER_MANAGER_OFFSET = 0x18
#: `ds2_rva::CHARACTER_MANAGER_ENTITY_BEGIN_OFFSET` / `..._END_OFFSET`. A begin/end POINTER PAIR.
ROSTER_BEGIN_OFFSET = 0x10
ROSTER_END_OFFSET = 0x18
#: `ds2_rva::PLAYER_CTRL_VTABLE`, as an RVA.
PLAYER_CTRL_VTABLE = 0x010E4BB8
#: `ds2_rva::MAX_ROSTER` -- the same refusal the crate makes on a torn begin/end pair.
MAX_ROSTER = 8192

#: `ds2_rva::CHARACTER_CTRL_NAME_OFFSET`
NAME_OFFSET = 0x118
#: `ds2_rva::WSTRING_LEN_OFFSET` / `..._CAPACITY_OFFSET` / `..._SSO_MAX`
WSTRING_LEN_OFFSET = 0x10
WSTRING_CAPACITY_OFFSET = 0x18
WSTRING_SSO_MAX = 7
#: Longest name this reads, matching `census::NAME_LIMIT`.
NAME_LIMIT = 64

#: `ds2_rva::CHARACTER_CTRL_PHANTOM_BLOCK_OFFSET`
PHANTOM_BLOCK_OFFSET = 0xB0
#: `ds2_rva::PHANTOM_BLOCK_PHANTOM_PARAM_OFFSET`
PHANTOM_PARAM_OFFSET = 0x3C
#: The team type, one byte past it. Not yet published by the crate; see the module docstring.
TEAM_TYPE_OFFSET = 0x3D
#: `ds2_rva::REPLAY_PHANTOM_PARAM_IDS`
REPLAY_PHANTOM_PARAM_IDS = (0x12, 0x13)
#: The spawn kind, `ds2_rva` does not publish this one yet; `0x1403161b0` writes it.
SPAWN_KIND_OFFSET = 0x54


def read_bytes(pid: int, address: int, count: int) -> bytes | None:
    """`count` bytes at `address`, or None if the range is not readable."""
    try:
        with open(f"/proc/{pid}/mem", "rb", buffering=0) as handle:
            handle.seek(address)
            raw = handle.read(count)
    except OSError:
        return None
    return raw if len(raw) == count else None


def read_u8(pid: int, address: int) -> int | None:
    raw = read_bytes(pid, address, 1)
    return None if raw is None else raw[0]


def read_wstring(pid: int, address: int) -> str | None:
    """An MSVC `std::wstring` at `address`, inline or heap, as the game's own readers decide.

    A length past the capacity is a torn read of two fields the game was writing, not a name, and
    is refused rather than printed.
    """
    length = ds2_run_lib.read_qword(pid, address + WSTRING_LEN_OFFSET)
    capacity = ds2_run_lib.read_qword(pid, address + WSTRING_CAPACITY_OFFSET)
    if length is None or capacity is None or length == 0 or length > capacity:
        return None
    if capacity > WSTRING_SSO_MAX:
        characters = ds2_run_lib.read_qword(pid, address)
        if not characters:
            return None
    else:
        characters = address
    raw = read_bytes(pid, characters, min(length, NAME_LIMIT) * 2)
    if raw is None:
        return None
    return raw.decode("utf-16-le", errors="replace")


def describe(pid: int, ctrl: int, local: int) -> str:
    name = read_wstring(pid, ctrl + NAME_OFFSET)
    spawn_kind = read_u8(pid, ctrl + SPAWN_KIND_OFFSET)
    block = ds2_run_lib.read_qword(pid, ctrl + PHANTOM_BLOCK_OFFSET)
    phantom_param = None if not block else read_u8(pid, block + PHANTOM_PARAM_OFFSET)
    team_type = None if not block else read_u8(pid, block + TEAM_TYPE_OFFSET)

    def byte(value: int | None) -> str:
        return "??" if value is None else f"0x{value:02x}"

    verdict = (
        "local"
        if ctrl == local
        else "REPLAY"
        if phantom_param in REPLAY_PHANTOM_PARAM_IDS
        else "person"
    )
    return (
        f"  0x{ctrl:012x}  {verdict:<6}  name={name or '<unreadable>'!r:<24} "
        f"kind={byte(spawn_kind)} phantom_param={byte(phantom_param)} team={byte(team_type)}"
    )


def walk(pid: int, base: int) -> int:
    manager = ds2_run_lib.read_qword(pid, base + GAME_MANAGER_IMP)
    if not manager:
        print("  GameManagerImp is null -- the game is still booting.")
        return 1
    characters = ds2_run_lib.read_qword(pid, manager + CHARACTER_MANAGER_OFFSET)
    local = ds2_run_lib.read_qword(pid, manager + PLAYER_CTRL_OFFSET)
    if not characters or not local:
        print("  no CharacterManager or no local player -- at a menu or still loading.")
        return 1
    begin = ds2_run_lib.read_qword(pid, characters + ROSTER_BEGIN_OFFSET)
    end = ds2_run_lib.read_qword(pid, characters + ROSTER_END_OFFSET)
    if not begin or end is None or end < begin or (end - begin) % 8:
        print(f"  roster span refused: begin=0x{begin or 0:x} end=0x{end or 0:x}")
        return 1
    count = (end - begin) // 8
    if count > MAX_ROSTER:
        print(f"  roster span refused: {count} entries is past MAX_ROSTER")
        return 1
    player_vtable = base + PLAYER_CTRL_VTABLE
    print(f"  roster                {count} entries, local player 0x{local:012x}")
    found = 0
    for index in range(count):
        ctrl = ds2_run_lib.read_qword(pid, begin + index * 8)
        if not ctrl:
            continue
        vtable = ds2_run_lib.read_qword(pid, ctrl)
        if vtable != player_vtable:
            continue
        found += 1
        print(describe(pid, ctrl, local))
    if found == 0:
        print("  no object in the roster carried PlayerCtrl's vtable.")
    return 0


def selftest() -> int:
    """Exercise the readers against this process, where the answers are known."""
    import ctypes
    import os

    failures = 0

    def check(name: str, ok: bool, detail: str = "") -> None:
        nonlocal failures
        if not ok:
            failures += 1
        print(("  ok   " if ok else "  FAIL ") + name + ("  " + detail if detail else ""))

    pid = os.getpid()

    known = (ctypes.c_uint8 * 4)(0x11, 0x12, 0x13, 0x14)
    address = ctypes.addressof(known)
    check("a byte is read back out of our own memory", read_u8(pid, address + 1) == 0x12)
    check("an unmapped byte answers None", read_u8(pid, 0x10) is None)

    # A heap-case wstring: pointer, length, capacity, laid out the way MSVC lays it out.
    #
    # The characters are built with an explicit `utf-16-le` encode rather than with
    # `ctypes.create_unicode_buffer`, which on Linux produces UCS-4 -- a `wchar_t` here is four
    # bytes and two on Windows. The first draft of this check used it and failed against its own
    # fixture, reading `'G\x00h\x00o\x00'`, which is what a UTF-16 reader correctly sees in UCS-4.
    text = "GhostPlayer_000042"
    encoded = text.encode("utf-16-le")
    buffer = ctypes.create_string_buffer(encoded, len(encoded))
    header = (ctypes.c_uint64 * 4)(ctypes.addressof(buffer), 0, len(text), len(text) + 8)
    check(
        "a heap-case wstring reads back",
        read_wstring(pid, ctypes.addressof(header)) == text,
        repr(read_wstring(pid, ctypes.addressof(header))),
    )

    # The inline case: characters at offset 0, capacity at or below the SSO maximum.
    inline = (ctypes.c_uint64 * 4)()
    short = "Rat"
    ctypes.memmove(inline, short.encode("utf-16-le"), len(short) * 2)
    inline[2] = len(short)
    inline[3] = WSTRING_SSO_MAX
    check("an inline wstring reads back", read_wstring(pid, ctypes.addressof(inline)) == short)

    torn = (ctypes.c_uint64 * 4)(0, 0, 9999, 4)
    check("a length past the capacity is refused", read_wstring(pid, ctypes.addressof(torn)) is None)

    check(
        "the replay ids are the pair 0x14016f740 tests",
        REPLAY_PHANTOM_PARAM_IDS == (0x12, 0x13),
    )
    return 1 if failures else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--selftest", action="store_true", help="check the readers and exit.")
    args = parser.parse_args()
    if args.selftest:
        return selftest()

    pid = ds2_run_lib.game_pid()
    if pid is None:
        print("DarkSoulsII.exe is not running.")
        return 1
    base = ds2_run_lib.game_image_base(pid)
    if base is None:
        print(f"could not find the image base of pid {pid}.")
        return 1
    print(f"pid {pid}, image base 0x{base:x}")
    return walk(pid, base)


if __name__ == "__main__":
    raise SystemExit(main())
