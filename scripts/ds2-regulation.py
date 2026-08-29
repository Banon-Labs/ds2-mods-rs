#!/usr/bin/env python3
"""Decrypt DARK SOULS II: SOTFS's `enc_regulation.bnd.dcx` and read the param tables inside.

    python3 scripts/ds2-regulation.py list
    python3 scripts/ds2-regulation.py extract --out /tmp/ds2reg
    python3 scripts/ds2-regulation.py param LevelUpStatusCalcParam.param
    python3 scripts/ds2-regulation.py souls --max-level 200
    python3 scripts/ds2-regulation.py items --grep 'Ring of Binding|Dark Weapon'

Everything below is either read out of the game (binary, or the file itself) or checked against
it. Where a constant could not be recovered from the executable that is said out loud, here and
at runtime.

# THE CRYPTO, and how each piece of it was pinned down

`enc_regulation.bnd.dcx` is **AES-128-CTR**, not CBC or ECB. The file is 836803 bytes, which is
not a multiple of 16 and cannot be a block-cipher ciphertext -- that single arithmetic fact is
what ruled out the block modes before a single key was tried. The layout is

    [ 0x00 .. 0x20 )   32-byte header; bytes 0..11 of it are the CTR nonce material
    [ 0x20 .. EOF  )   keystream-XORed payload, a plain DS2 `DCX`/`DFLT`

    counter block = 0x80 || file[0:11] || 00 00 00 01

The nonce shape and the data offset were CONFIRMED by decryption, not assumed: the plaintext's
first sixteen bytes come out as the exact DS2 DCX header
`44 43 58 00 00 01 00 00 00 00 00 18 00 00 00 24`, taken from a real DCX pulled out of
`GameDataEbl` with `scripts/ds2-ebl.py`, and the payload then inflates to a BND4 of exactly the
size the DCX header declares. A wrong nonce or a wrong offset gives noise at the first byte.

THE KEY IS THE ONE THING NOT DERIVED FROM THE BINARY, and this is the honest statement of it.
`REGULATION_KEY_HEX` below is the published `ds2RegulationKey` constant (SoulsFormats
`SFUtil.cs`; the same bytes are `RegulationFileDs2` in BinderTool's `DecryptionKeys.cs`). Unlike
the save key, which `scripts/ds2-sl2.py --key-from-image` re-derives from a UTF-16 string in
`.rdata`, this one is NOT recoverable by inspection. Two searches, both run after the key was already
known and confirmed, say so: (1) the 16 key bytes do not occur anywhere in the 30892032 bytes of
`darksoulsii-deobf.bin` or the 28200992 of `DarkSoulsII.exe`, and neither does the 32-character
hex text in ASCII or UTF-16 -- which is how the SAVE key is stored, so that storage form was
specifically looked for; (2) treating every one of those ~59 million byte offsets as a candidate
AES-128 key and testing it against the known DCX-header plaintext under CBC (IV = file[0:16])
and ECB produced no hit either. The key is assembled at runtime, behind Arxan.
So: PROVENANCE IS INFERRED, but CORRECTNESS IS VERIFIED -- a wrong key cannot produce a valid DCX
header, a valid zlib stream, the declared inflate size, and 228 sanely-named BND4 members. That
check runs on every invocation and this refuses to hand back plaintext that fails it.

# THE CONTAINER

Stock BND4, `02000200`, 228 members, 0x40-byte header, 24-byte file entries
`{ u32 flags; i32 -1; u32 size; u32 0; u32 dataOffset; u32 nameOffset }`, names NUL-terminated
ASCII. The `-1` and the zero pad are asserted, so a build with a different entry stride fails
here instead of yielding shifted garbage.

# THE PARAM FORMAT, and the proof that this parse is the game's

    +0x00  u32  offset of the (always empty) string table == the param's own length
    +0x08  u16  unknown, varies 0..51 across the 205 params
    +0x0A  u16  ROW COUNT
    +0x0C  char[0x20]  param type name, NUL-padded  ("CHR_LEVEL_UP_SOULS_PARAM", ...)
    +0x2C  u32  0x00070400 in all 205 params
    +0x30  u64  offset of the first row's data
    +0x38  u64  0
    +0x40  the row index: `{ u64 id; u64 dataOffset; u64 nameOffset }` x rowCount
           then the row data, at a stride this tool MEASURES from the index rather than assuming

This is not guesswork dressed up. `FUN_140358b90` in the executable -- the CharacterManager's
lookup for `PlayerLevelUpSoulsParam` (index 0x27 of the name array in
`getCharacterManagerParamNameFromIndex` at 0x14048b620) -- reads the row count as
`*(ushort *)(base + 10)` and the row's data as `base + *(u64 *)(base + (index + 3) * 0x18)`.
`(index + 3) * 0x18` is `0x40 + index * 24 + 8`: the same index table, the same 24-byte stride,
the same offset-at-+8. The game agrees with this parse instruction for instruction.

Every param is checked on load: `dataStart == 0x40 + rowCount * 24`, the index offsets strictly
increasing at ONE uniform stride, the first equal to `dataStart`, the last row inside the
declared length. All 205 params in the shipped file satisfy all four. Row NAMES do not exist in
DS2 params -- every `nameOffset` is either 0 or points at the empty string at the file's end --
so item names come from an FMG, not from here. See `scripts/ds2-fmg.py`.

# THE TWO TABLES THIS WAS BUILT FOR

`PlayerLevelUpSoulsParam.param` -- 852 rows, 12-byte stride, `{ u16 level; u16 pad; i32 gradient;
i32 souls }`. Row ids 0..850 plus a sentinel 999. The souls value on the row whose id is L is
**the cost to go from soul level L to L+1**, which is a direction that is easy to get backwards
by one level and so was settled from the code rather than from a plausible-looking table:
`FUN_1401fb970`, the level-up menu's increment, does `souls -= pending; level += 1; pending =
lookup(level)` -- it pays the value stored for the level it is leaving, then re-reads at the new
level. `FUN_14003ebe0`, the menu's display update, likewise passes the CURRENT soul level to the
cost lookup to render "souls required". So the soul memory a level L implies is the running sum
of rows 1..L-1, which is what `souls` prints.

The `gradient` field is 0 in every shipped row; the lookup would interpolate
`(wanted - rowLevel) * gradient + souls` for a level with no exact row. This refuses to report a
plain per-level cost if a future build makes it nonzero, because then the table is not one.

`LevelUpStatusCalcParam.param` -- 9 rows, 8-byte stride, and NOT the soul table despite the name.
It is a menu-side param (the executable names it beside `FeTimeSetting`, `FeColorPalette` and
the rest of the level-up screen's params at VA 0x1410f2da0) and its nine rows carry one packed
u32 each. No paramdef for it is available here, so `param` prints the raw words and this refuses
to invent field names for them.

`ItemParam.param` -- 1275 rows, 84-byte stride. THE ROW ID IS THE ITEM ID AND IS ALSO THE TEXT ID
IN `itemname.fmg`: the mapping is the identity, verified over the whole table (1234 of the 1275
row ids resolve to a name; 106 FMG ids have no ItemParam row). `items --fmg` does that join.
"""

from __future__ import annotations

import argparse
import re
import struct
import subprocess
import sys
import zlib
from pathlib import Path

GAME_DIR = (
    Path.home()
    / ".local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game"
)
DEFAULT_REGULATION = GAME_DIR / "enc_regulation.bnd.dcx"
#: Where `scripts/ds2-ebl.py extract /menu/text/english/itemname.fmg` puts the name table.
DEFAULT_FMG = None

#: NOT read from the executable -- see the module docstring. Overridable with `--key`.
REGULATION_KEY_HEX = "40178130DF0A9454330 9E171ECBF254C".replace(" ", "")
#: Ciphertext starts here; the bytes before it are the nonce material.
PAYLOAD_OFFSET = 0x20
NONCE_BYTES = 11

#: A DS2 `DCX`/`DFLT` header's first sixteen bytes, byte for byte. Read off a real DCX
#: (`/menu/42.febnd.dcx` out of GameDataEbl), not recalled. This is the decryption's proof.
DCX_HEADER_16 = bytes.fromhex("44435800000100000000001800000024")
#: Where the deflate stream begins in a DS2 DCX. Same constant `scripts/ds2-ebl.py` uses, and
#: taken as a constant for the same reason: a zlib signature search finds a false one first.
DCX_PAYLOAD_OFFSET = 0x4C

BND4_MAGIC = b"BND4"
BND4_HEADER_SIZE = 0x40
BND4_ENTRY_SIZE = 24

PARAM_HEADER_SIZE = 0x40
PARAM_INDEX_ENTRY_SIZE = 24
PARAM_NAME_OFFSET = 0x0C
PARAM_NAME_LENGTH = 0x20
#: The one value at +0x2C across all 205 shipped params. Reported, not enforced.
PARAM_FORMAT_WORD = 0x00070400

#: PlayerLevelUpSoulsParam row: `{ u16 level; u16 pad; i32 gradient; i32 souls }`.
SOULS_STRIDE = 12
SOULS_SENTINEL_ID = 999


class Fail(SystemExit):
    """Every consistency failure raises this. Nothing here returns a best-effort parse."""


# --------------------------------------------------------------------------- crypto / container


def decrypt(blob: bytes, key_hex: str) -> bytes:
    """AES-128-CTR the regulation file, and refuse anything that is not a DS2 DCX."""
    if len(blob) <= PAYLOAD_OFFSET:
        raise Fail(f"regulation file is {len(blob)} bytes; too short to hold a payload")
    counter = bytes([0x80]) + blob[:NONCE_BYTES] + b"\0\0\0" + bytes([1])
    if len(counter) != 16:
        raise Fail(f"counter block is {len(counter)} bytes, not 16")
    run = subprocess.run(
        ["openssl", "enc", "-aes-128-ctr", "-d", "-K", key_hex, "-iv", counter.hex()],
        input=blob[PAYLOAD_OFFSET:],
        capture_output=True,
    )
    if run.returncode:
        raise Fail(run.stderr.decode().strip() or "openssl failed")
    plain = run.stdout
    if plain[:16] != DCX_HEADER_16:
        raise Fail(
            f"decrypted head is {plain[:16].hex(' ')}, expected the DS2 DCX header "
            f"{DCX_HEADER_16.hex(' ')}.\n"
            "    The key is wrong for this build, or the container layout changed. This will not\n"
            "    hand back plaintext it cannot vouch for -- see the module docstring."
        )
    return plain


def dcx_inflate(data: bytes) -> bytes:
    uncompressed, compressed = struct.unpack_from(">II", data, 0x1C)
    if compressed != len(data) - DCX_PAYLOAD_OFFSET:
        raise Fail(
            f"DCX declares {compressed} compressed bytes, {len(data) - DCX_PAYLOAD_OFFSET} follow"
        )
    out = zlib.decompress(data[DCX_PAYLOAD_OFFSET:])
    if len(out) != uncompressed:
        raise Fail(f"DCX declares {uncompressed} inflated bytes, got {len(out)}")
    return out


def bnd4_members(data: bytes) -> list[tuple[str, int, int]]:
    """`[(name, dataOffset, size)]`, with the entry layout asserted rather than trusted."""
    if data[:4] != BND4_MAGIC:
        raise Fail(f"expected {BND4_MAGIC!r} after inflating, got {data[:4]!r}")
    count = struct.unpack_from("<I", data, 0x0C)[0]
    header_size = struct.unpack_from("<Q", data, 0x10)[0]
    entry_size = struct.unpack_from("<Q", data, 0x20)[0]
    if header_size != BND4_HEADER_SIZE or entry_size != BND4_ENTRY_SIZE:
        raise Fail(
            f"BND4 header/entry sizes are {header_size:#x}/{entry_size}; this build is not the "
            f"one this parses ({BND4_HEADER_SIZE:#x}/{BND4_ENTRY_SIZE})"
        )
    out = []
    for index in range(count):
        base = header_size + index * entry_size
        _flags, sentinel, size, pad, offset, name_at = struct.unpack_from("<IiIIII", data, base)
        if sentinel != -1 or pad != 0:
            raise Fail(f"BND4 entry {index}: sentinel {sentinel} pad {pad}; layout is not 24-byte")
        if offset + size > len(data) or name_at >= len(data):
            raise Fail(f"BND4 entry {index} points outside the file")
        end = data.index(b"\0", name_at)
        out.append((data[name_at:end].decode("shift_jis", "replace"), offset, size))
    return out


def load(path: Path, key_hex: str) -> dict[str, bytes]:
    blob = path.read_bytes()
    inflated = dcx_inflate(decrypt(blob, key_hex))
    return {n: inflated[o : o + s] for n, o, s in bnd4_members(inflated)}


# ------------------------------------------------------------------------------------- the param


class Param:
    """One `*.param`, with the four structural checks that make a wrong stride impossible."""

    def __init__(self, name: str, blob: bytes) -> None:
        self.name = name
        self.blob = blob
        if len(blob) < PARAM_HEADER_SIZE:
            raise Fail(f"{name}: {len(blob)} bytes, too short for a param header")
        self.declared_length = struct.unpack_from("<I", blob, 0)[0]
        self.unknown08 = struct.unpack_from("<H", blob, 8)[0]
        self.row_count = struct.unpack_from("<H", blob, 0x0A)[0]
        raw_name = blob[PARAM_NAME_OFFSET : PARAM_NAME_OFFSET + PARAM_NAME_LENGTH]
        self.type_name = raw_name.split(b"\0")[0].decode("ascii", "replace")
        self.format_word = struct.unpack_from("<I", blob, 0x2C)[0]
        self.data_start = struct.unpack_from("<Q", blob, 0x30)[0]

        expected = PARAM_HEADER_SIZE + self.row_count * PARAM_INDEX_ENTRY_SIZE
        if self.data_start != expected:
            raise Fail(
                f"{name}: dataStart {self.data_start:#x} != 0x40 + {self.row_count} * 24 "
                f"({expected:#x}). The row index is not the shape this parses."
            )
        if self.declared_length > len(blob):
            raise Fail(f"{name}: header declares {self.declared_length} bytes, member is {len(blob)}")

        self.ids: list[int] = []
        offsets: list[int] = []
        for k in range(self.row_count):
            at = PARAM_HEADER_SIZE + k * PARAM_INDEX_ENTRY_SIZE
            row_id, data_at, name_at = struct.unpack_from("<QQQ", blob, at)
            if name_at not in (0, self.declared_length):
                raise Fail(
                    f"{name}: row {k} has a name offset {name_at:#x}. DS2 params carry no row "
                    "names; this build does, and nothing here knows how to read them."
                )
            self.ids.append(row_id)
            offsets.append(data_at)
        self.offsets = offsets

        if self.row_count == 0:
            self.stride = 0
            return
        if offsets[0] != self.data_start:
            raise Fail(f"{name}: first row is at {offsets[0]:#x}, header says {self.data_start:#x}")
        strides = {b - a for a, b in zip(offsets, offsets[1:])}
        if len(strides) > 1:
            raise Fail(f"{name}: row stride is not uniform -- saw {sorted(strides)}")
        self.stride = strides.pop() if strides else self.declared_length - self.data_start
        if self.stride <= 0:
            raise Fail(f"{name}: measured stride {self.stride}")
        if offsets[-1] + self.stride > self.declared_length:
            raise Fail(
                f"{name}: last row {offsets[-1]:#x}+{self.stride} overruns the declared "
                f"length {self.declared_length:#x}"
            )

    def row(self, index: int) -> bytes:
        at = self.offsets[index]
        return self.blob[at : at + self.stride]

    def by_id(self) -> dict[int, bytes]:
        return {rid: self.row(k) for k, rid in enumerate(self.ids)}

    def describe(self) -> str:
        return (
            f"{self.name}\n"
            f"  type name    {self.type_name}\n"
            f"  rows         {self.row_count}\n"
            f"  stride       {self.stride} bytes  (measured from the row index)\n"
            f"  data start   {self.data_start:#x}  == 0x40 + {self.row_count} * 24\n"
            f"  length       {self.declared_length}  (member is {len(self.blob)} bytes)\n"
            f"  +0x08 u16    {self.unknown08}\n"
            f"  +0x2C u32    {self.format_word:#010x}"
            + ("" if self.format_word == PARAM_FORMAT_WORD else "  <- not the usual 0x00070400")
        )


# ------------------------------------------------------------------------------------ subcommands


def cmd_list(members: dict[str, bytes], args) -> int:
    params = 0
    for name, blob in sorted(members.items()):
        note = ""
        if name.endswith(".param"):
            params += 1
            p = Param(name, blob)
            note = f"  {p.type_name}  rows={p.row_count} stride={p.stride}"
        print(f"{len(blob):9d}  {name}{note}")
    print(f"\n{len(members)} members, {params} of them `.param` (all parsed without complaint)")
    return 0


def cmd_extract(members: dict[str, bytes], args) -> int:
    args.out.mkdir(parents=True, exist_ok=True)
    for name, blob in sorted(members.items()):
        (args.out / name).write_bytes(blob)
    print(f"wrote {len(members)} members to {args.out}")
    return 0


def cmd_param(members: dict[str, bytes], args) -> int:
    name = args.name if args.name in members else args.name + ".param"
    if name not in members:
        raise Fail(f"no member named {args.name!r}; try `list`")
    p = Param(name, members[name])
    print(p.describe())
    if p.stride % 4 == 0:
        words = p.stride // 4
        fmt = f"<{words}I" if args.unsigned else f"<{words}i"
    else:
        fmt = None
    print()
    limit = args.rows if args.rows > 0 else p.row_count
    for k in range(min(limit, p.row_count)):
        raw = p.row(k)
        if args.hex or fmt is None:
            print(f"  id={p.ids[k]:<12d} {raw.hex(' ')}")
        else:
            print(f"  id={p.ids[k]:<12d} {list(struct.unpack(fmt, raw))}")
    if limit < p.row_count:
        print(f"  ... {p.row_count - limit} more rows (--rows 0 for all)")
    return 0


def souls_table(members: dict[str, bytes]) -> tuple[Param, dict[int, int]]:
    """`{level: souls to reach level+1}`, with the gradient field asserted zero."""
    p = Param("PlayerLevelUpSoulsParam.param", members["PlayerLevelUpSoulsParam.param"])
    if p.stride != SOULS_STRIDE:
        raise Fail(f"PlayerLevelUpSoulsParam stride is {p.stride}, expected {SOULS_STRIDE}")
    table: dict[int, int] = {}
    for k, row_id in enumerate(p.ids):
        level, pad, gradient, souls = struct.unpack("<HHii", p.row(k))
        if level != row_id or pad != 0:
            raise Fail(
                f"PlayerLevelUpSoulsParam row {k}: level field {level} != row id {row_id} "
                f"(pad {pad}). The row layout is not `u16 level; u16 pad; i32 grad; i32 souls`."
            )
        if gradient != 0:
            raise Fail(
                f"PlayerLevelUpSoulsParam row id {row_id} has gradient {gradient}. The game "
                "interpolates `(wanted - level) * gradient + souls` for levels with no exact "
                "row, so with a nonzero gradient this is no longer a per-level table and this "
                "tool will not print one."
            )
        table[row_id] = souls
    return p, table


def cmd_souls(members: dict[str, bytes], args) -> int:
    p, table = souls_table(members)
    levels = sorted(k for k in table if k != SOULS_SENTINEL_ID)
    if levels != list(range(levels[0], levels[-1] + 1)):
        raise Fail("PlayerLevelUpSoulsParam row ids are not contiguous; refusing to sum them")
    top = min(args.max_level, levels[-1])
    # With --csv stdout must be nothing but CSV, so the provenance block goes to stderr where a
    # pipeline ignores it and a human still sees it.
    note = sys.stderr if args.csv else sys.stdout
    print(p.describe(), file=note)
    print(
        "\n  row id L holds the souls to go from soul level L to L+1 (verified against the "
        "game's own level-up increment, FUN_1401fb970).",
        file=note,
    )
    print(
        f"  ids {levels[0]}..{levels[-1]} contiguous, plus sentinel id {SOULS_SENTINEL_ID} "
        f"= {table[SOULS_SENTINEL_ID]}\n",
        file=note,
    )
    if args.csv:
        print("level,cost_to_next,cumulative_at_level")
    else:
        print(f"  {'level':>5}  {'cost L->L+1':>13}  {'cumulative (soul memory at L)':>30}")
    running = 0
    for level in range(1, top + 1):
        if args.csv:
            print(f"{level},{table[level]},{running}")
        else:
            print(f"  {level:>5}  {table[level]:>13,}  {running:>30,}")
        running += table[level]
    if not args.csv:
        print(f"\n  cumulative at level {top}: {running:,}")
        total = 0
        for level in range(1, levels[-1] + 1):
            total += table[level]
        print(f"  cumulative at level {levels[-1] + 1} (last row + 1): {total:,}")
    return 0


def cmd_items(members: dict[str, bytes], args) -> int:
    p = Param("ItemParam.param", members["ItemParam.param"])
    names: dict[int, str] = {}
    if args.fmg:
        sys.path.insert(0, str(Path(__file__).resolve().parent))
        import importlib.util

        spec = importlib.util.spec_from_file_location("ds2fmg", Path(__file__).parent / "ds2-fmg.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        names = module.parse(args.fmg.read_bytes())

    pattern = re.compile(args.grep, re.I) if args.grep else None
    print(p.describe())
    print(
        "\n  the row id IS the item id and IS the text id in itemname.fmg (identity mapping).\n"
        "  fields, as `i32[21]`: [0] own id, [5] WeaponParam id, [8] RingParam id,\n"
        "  [9] SpellParam id -- those three land in their param for EVERY row that fills them.\n"
        "  [6] is the ArmorParam id for 439 of the 461 rows that fill it; the other 22 land in\n"
        "  WeaponParam or nowhere, so treat [6] as a lead, not a fact. [12]/[13] are buy/sell\n"
        "  price (sell == buy/10 in 1247 of 1275 rows). The rest of the 84 bytes has no\n"
        "  paramdef here and is printed raw.\n"
    )
    shown = 0
    for k, row_id in enumerate(p.ids):
        label = names.get(row_id)
        if args.ids and row_id not in args.ids:
            continue
        if pattern and not (label and pattern.search(label)):
            continue
        values = list(struct.unpack(f"<{p.stride // 4}i", p.row(k)))
        refs = {
            "weapon": values[5],
            "armor": values[6],
            "ring": values[8],
            "spell": values[9],
        }
        refs = {k2: v for k2, v in refs.items() if v != -1}
        print(f"  {row_id:>10}  {label if label is not None else '<no name in fmg>'}")
        print(f"              buy={values[12]} sell={values[13]} refs={refs}")
        shown += 1
        if args.limit and shown >= args.limit:
            print(f"  ... stopped at --limit {args.limit}")
            break
    if not shown:
        print("  (nothing matched)")
    if names:
        have = sum(1 for i in p.ids if i in names)
        print(f"\n  {have} of {p.row_count} ItemParam rows resolve to a name in that fmg; "
              f"{len(set(names) - set(p.ids))} fmg ids have no ItemParam row")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__.splitlines()[0],
        epilog="See the module docstring for provenance of every constant.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument("--file", type=Path, default=DEFAULT_REGULATION)
    ap.add_argument(
        "--key",
        default=REGULATION_KEY_HEX,
        help="32 hex digits. The default is a PUBLISHED constant, not one read out of the "
        "executable -- see the module docstring.",
    )
    sub = ap.add_subparsers(dest="command", required=True)

    sub.add_parser("list", help="every member, with each param's row count and stride")

    p_extract = sub.add_parser("extract", help="write every member to a directory")
    p_extract.add_argument("--out", type=Path, required=True)

    p_param = sub.add_parser("param", help="decode one param and print its rows")
    p_param.add_argument("name")
    p_param.add_argument("--rows", type=int, default=40, help="0 for all")
    p_param.add_argument("--hex", action="store_true", help="raw bytes instead of words")
    p_param.add_argument("--unsigned", action="store_true", help="u32 instead of i32")

    p_souls = sub.add_parser("souls", help="the level -> soul cost table and its running sum")
    p_souls.add_argument("--max-level", type=int, default=200)
    p_souls.add_argument("--csv", action="store_true")

    p_items = sub.add_parser("items", help="ItemParam rows, optionally joined to itemname.fmg")
    p_items.add_argument("--fmg", type=Path, help="path to an extracted itemname.fmg")
    p_items.add_argument("--grep", help="regex over the fmg name (needs --fmg)")
    p_items.add_argument(
        "--id", dest="ids", action="append", type=lambda s: int(s, 0), default=[]
    )
    p_items.add_argument("--limit", type=int, default=60, help="0 for no limit")

    args = ap.parse_args()
    if not re.fullmatch(r"[0-9A-Fa-f]{32}", args.key):
        raise Fail(f"--key must be 32 hex digits, got {args.key!r}")
    if not args.file.is_file():
        raise Fail(f"not found: {args.file}")

    members = load(args.file, args.key)
    return {
        "list": cmd_list,
        "extract": cmd_extract,
        "param": cmd_param,
        "souls": cmd_souls,
        "items": cmd_items,
    }[args.command](members, args)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
