#!/usr/bin/env python3
"""Read a DARK SOULS II: Scholar of the First Sin `.sl2` save: container, crypto, contents.

    python3 scripts/ds2-sl2.py <save.sl2>              # list the BND4 entries
    python3 scripts/ds2-sl2.py <save.sl2> -x <outdir>  # also decrypt each entry to a file

THE CONTAINER is a stock BND4: header, a table of 0x20-byte file headers, a UTF-16LE name
table, then payloads. 23 entries named USER_DATA000..USER_DATA022.

THE CRYPTO is AES-128-CBC, one independent IV per entry:

    entry = [ 16-byte MD5 of everything after it ][ 16-byte CBC IV ][ AES-128-CBC ciphertext ]

The MD5 is taken over the CIPHERTEXT (IV included), not the plaintext, so it can be checked
before decrypting anything. Verified against all 23 entries of a real save.

Note the IV is the SECOND sixteen bytes, not the first. Decrypting from offset 16 using the
leading hash as the IV also yields correct plaintext from the second block onward, because CBC
needs only C[i-1] to recover P[i]. That is a comfortable way to be subtly wrong about the
layout: the payload looks right while the first plaintext block is noise.

THE DECRYPTED PAYLOAD is a chain of sections:

    [ u32 totalSize ] then repeating [ u32 id ][ u32 version ][ u32 size ][ size bytes ]

Section id=4 (size 0x1370) is the per-character array: a 16-byte prologue followed by exactly
ten records of 0x1F0 bytes, the game's ten character slots. It appears in both global entries.

THE IVs ARE NOT RANDOM, which is the most useful accident in the file. Entries written by one
save operation share IV bytes 0, 1, 4, 5, 8, 9, 12 and 13. That partitions the 23 entries into
write batches WITHOUT DECRYPTING ANYTHING, and so identifies the character slot the most recent
save touched. See --batches.

THE KEY IS NOT TYPED IN HERE FROM MEMORY. It is read out of the game's own config table in
`.rdata`, where it sits as a UTF-16 hex string immediately BEFORE the property name that
selects it (the table is laid out value-then-key):

    L"599F9B699640A55236EE2D70835EC744"    <- value
    L"SaveLoad2.Title.EncryptionKey"       <- key      (VA 0x1410da888 in darksoulsii-deobf.bin)

That storage form is why a scan of the image for a raw 16-byte AES key finds nothing: the key
only exists as text until the game parses it. `--key-from-image` re-derives it from the binary
rather than trusting the constant below, which is the check to run if a patch ever moves it.
"""

import argparse
import hashlib
import re
import struct
import subprocess
import sys
from pathlib import Path

#: Read out of the game image, not remembered. See module docstring and --key-from-image.
KEY_HEX = "599F9B699640A55236EE2D70835EC744"
KEY_PROPERTY = "SaveLoad2.Title.EncryptionKey"

#: VANILLA DARK SOULS II (`DARKSII0000.sl2`, Steam app 236430) uses a DIFFERENT key, and this one
#: cannot be re-derived with `--key-from-image` because that image is a different executable which
#: is not checked out here. Provenance, since it is not read from a binary: it is `ds2SaveKey` in
#: SoulsFormats `SoulsFormats/Util/SFUtil.cs`, which carries both keys side by side and labels this
#: one "original DS2 save files on PC"; the same bytes appear as `UserDataKeyDs2` in BinderTool's
#: `DecryptionKeys.cs` and in SoulsAssetPipeline's `SL2Decryptor.cs`.
#:
#: VERIFIED HERE rather than trusted: both vanilla saves in `/home/banon/DS2` decrypt under it into
#: valid section chains -- sane ids/versions/sizes and readable UTF-16 character names -- while
#: they decrypt to high-entropy noise under `KEY_HEX` above. That round trip is the check to redo
#: if this constant is ever doubted; a wrong key here produces noise, not a plausible save.
VANILLA_KEY_HEX = "B7FD463E4A9C1102DF1739E5F3B2A50F"


def key_from_image(image):
    """Re-derive the key from the game image by finding the string before the property name."""
    blob = Path(image).read_bytes()
    name = KEY_PROPERTY.encode("utf-16-le")
    for m in re.finditer(re.escape(name), blob):
        # Values are NUL-padded to 8 bytes; walk back over the padding, then over the string.
        end = m.start()
        while end >= 2 and blob[end - 2 : end] == b"\0\0":
            end -= 2
        start = end
        while start >= 2 and blob[start - 2 : start] != b"\0\0":
            start -= 2
        text = blob[start:end].decode("utf-16-le", "replace")
        if re.fullmatch(r"[0-9A-Fa-f]{32}", text):
            return text
    raise SystemExit(f"no 32-hex-digit value found before {KEY_PROPERTY!r} in {image}")


def decrypt(ct, iv, key_hex):
    r = subprocess.run(
        ["openssl", "enc", "-aes-128-cbc", "-d", "-nopad", "-K", key_hex, "-iv", iv.hex()],
        input=ct,
        capture_output=True,
    )
    if r.returncode:
        raise SystemExit(r.stderr.decode())
    return r.stdout


#: The character-list section: id 4, always this size. 16-byte prologue then ten records.
#: It appears in BOTH "global" entries (USER_DATA000 and USER_DATA022); they agree, and the first
#: one found is used.
SLOT_SECTION_ID = 4
SLOT_SECTION_SIZE = 0x1370
SLOT_PROLOGUE = 16
#: One slot record. Same stride the live list group uses (`ds2_rva::SAVE_SLOT_STRIDE`).
SLOT_STRIDE = 0x1F0
SLOT_COUNT = 10

#: Offsets WITHIN a record, established by inspection of two real saves this session:
#: the eleven `short`s at 0x188 are the nine DS2 stats plus two, and the UTF-16 name begins
#: immediately after them at 0x188 + 11*2 = 0x19E.
SLOT_ATTRS_OFFSET = 0x188
SLOT_ATTRS_COUNT = 11
SLOT_NAME_OFFSET = 0x19E

#: DO NOT use the record's flags byte at +0x1D9 to decide occupancy. It is zero in every record of
#: every real `.sl2` -- the game derives it when it loads the per-character entries -- so a cold
#: read of a save file must judge occupancy from record CONTENT. See `ds2_rva::SAVE_SLOT_FLAGS_OFFSET`,
#: which documents the same trap from the runtime side. An all-zero record is an empty slot; the
#: game fills stats, name and the ownership word for a real one.


#: A slot whose nine stats are ALL ONE holds a character the game will still load. It looks like a
#: blank -- no name, stats below any DS2 starting value -- and this once classified it as "not a
#: character" on exactly that reasoning. THE GAME DISAGREED, and the game is the authority: with a
#: downloaded "mule" staged and slot 9 requested, the runtime logged
#: `autoload slot=9 refused=false ... occupied=true` with its own flags byte reading `0x0d`, bit 0
#: set. So the all-ones shape is reported for what it is -- a blank character, ready to be named --
#: and is COUNTED AS OCCUPIED, because refusing to pick it produced a warning about a slot that
#: then loaded perfectly well.
BLANK_ATTRS = 1


def _classify(attrs):
    """`occupied`, `blank` or `empty` for one slot's nine stats. The first two are both loadable.

    OCCUPANCY IS JUDGED FROM STAT CONTENT, and it has to be: the record's own flags byte at +0x1D9
    is zero in every real `.sl2` because the game derives it when it loads the per-character
    entries (see `ds2_rva::SAVE_SLOT_FLAGS_OFFSET`). Nothing here is a claim about what the game
    will agree to load -- the runtime applies an ownership check this cannot see, and the mod's
    continue flow refuses a slot the game refuses and says so in the log.
    """
    if not any(attrs):
        return "empty"
    if all(a == BLANK_ATTRS for a in attrs):
        return "blank"
    return "occupied"


#: A slot's own data lives in two entries, not in the character list: USER_DATA(slot+1) and
#: USER_DATA(slot+11). `FUN_1402e6230` loads load-content indices slot+8 and slot+0x12, and its
#: type-2 path loads index 7, which is USER_DATA000 -- so index k is USER_DATA(k-7). Measured on a
#: hung load (2026-09-26, ds2-mods-rs-t46i): slot 1's two streams were 111292 and
#: 501928 bytes, the length words of USER_DATA002 and USER_DATA012.
SLOT_DATA_ENTRY_OFFSETS = (1, 11)

#: `FUN_1402e47f0`'s header walk: 0x20-byte headers, u32 type at +0, u32 version at +4, u32 size
#: at +8, then a relative seek by size. It stops on type 0xD and bails out on a type missing from
#: the fifteen-entry table at 0x1410da1f0.
SECTION_HEADER = 0x20
SECTION_END = 0xD
SECTION_TYPES = frozenset(range(15))


def section_walk(stream):
    """What the game's header walk does with one slot entry's stream: `ends`, `bails` or `hangs`.

    Emulates `FUN_1402e47f0` over `DLMemoryInputStream` exactly where it matters. A read copies
    whatever is left and returns -1 AT the end without touching the header; a seek clamps to the
    end. So a walk that reaches the end without having read a 0xD header keeps the last header it
    read, seeks nowhere, fails the read again, and never leaves the loop. That is the hang: the
    game thread spins there with no timeout. An all-zero entry gets there in size/0x20 steps,
    because type 0 is a legal type and a size of 0 is a legal size.

    `bails` is the walk's own error exit (a type outside the table); the game survives that one.
    """
    if not stream:
        # The first header is then uninitialised stack. Nothing sane can be said except "unsafe".
        return "hangs"
    header = bytearray(SECTION_HEADER)
    cursor = 0
    while True:
        if cursor >= len(stream):
            # The read fails and the header is the one already acted on: the same step forever,
            # unless that header was the end marker, which would have left the loop already.
            return "hangs"
        chunk = stream[cursor : cursor + SECTION_HEADER]
        header[: len(chunk)] = chunk
        cursor += len(chunk)
        kind, _version, size = struct.unpack_from("<III", header, 0)
        if kind == SECTION_END:
            return "ends"
        if kind not in SECTION_TYPES:
            return "bails"
        cursor = min(cursor + size, len(stream))


def slot_hangs(b, table, slot, key_hex):
    """Whether loading `slot` would spin the game thread forever in `FUN_1402e47f0`.

    A missing entry counts as hanging: the game would stream a null buffer, whose first header is
    uninitialised stack.
    """
    for delta in SLOT_DATA_ENTRY_OFFSETS:
        index = slot + delta
        if index >= len(table):
            return True
        _i, _name, off, size = table[index]
        payload = decrypt(b[off + 32 : off + size], b[off + 16 : off + 32], key_hex)
        if section_walk(entry_stream(payload)) == "hangs":
            return True
    return False


def entry_stream(payload):
    """The bytes the game streams from one decrypted entry: past the u32 length, that long."""
    if len(payload) < 4:
        return b""
    length = struct.unpack_from("<I", payload, 0)[0]
    return payload[4 : 4 + length]


def _wide_name(record, offset):
    """Decode a UTF-16LE name, terminating on an ALIGNED pair of zero bytes.

    Splitting on `b"\x00\x00"` is the obvious version and it is wrong: that pattern matches at an
    odd offset inside ordinary ASCII-in-UTF-16 (`...6c 00 66 00 00 00` matches starting at the byte
    before the terminator), which truncates the last character and leaves half a code unit to
    decode. Stepping two bytes at a time is the whole fix -- it cost "Elden Wolf" its f.
    """
    chunk = record[offset : offset + 64]
    for i in range(0, len(chunk) - 1, 2):
        if chunk[i] == 0 and chunk[i + 1] == 0:
            chunk = chunk[:i]
            break
    return chunk.decode("utf-16le", "replace")


def slot_records(b, key_hex):
    """(index, state, stats tuple, name) for each of the ten character slots.

    `hollow` is a slot the character list calls `blank` or `occupied` whose own data entries the
    game cannot load: its section walk runs off the end with no 0xD header, and the game thread
    then spins in `FUN_1402e47f0` forever (see `section_walk`). It is NOT loadable, and
    `ds2-run.py`'s autoload skips it because it only takes `occupied` and `blank`. Measured on
    `~/Downloads/DS2 Saves/new/DS2SOFS0000.sl2`, whose list names "Swornsword Ole" in slot 1 while
    USER_DATA002 and USER_DATA012 are all zeros; picking him hard-locked the game.

    Occupancy is "the record is not all zeros", which is what distinguishes a used slot from an
    unused one in every save examined. It is deliberately NOT a claim about what the game will
    agree to load: the runtime applies an ownership check too, and the mod's continue flow refuses
    a slot the game would refuse and says so in the log. This is a reading of the file, not a
    promise about the game.
    """
    table = list(entries(b))
    for _i, _name, off, size in table:
        payload = decrypt(b[off + 32 : off + size], b[off + 16 : off + 32], key_hex)
        o = 4
        while o + 12 <= len(payload):
            sid, _ver, ssize = struct.unpack_from("<III", payload, o)
            if sid == SLOT_SECTION_ID and ssize == SLOT_SECTION_SIZE:
                base = o + 12 + SLOT_PROLOGUE
                out = []
                for slot in range(SLOT_COUNT):
                    r = payload[base + slot * SLOT_STRIDE :][:SLOT_STRIDE]
                    if len(r) < SLOT_STRIDE:
                        break
                    attrs = struct.unpack_from(f"<{SLOT_ATTRS_COUNT}h", r, SLOT_ATTRS_OFFSET)
                    state = _classify(attrs[:9])
                    if state != "empty" and slot_hangs(b, table, slot, key_hex):
                        state = "hollow"
                    out.append((slot, state, attrs[:9], _wide_name(r, SLOT_NAME_OFFSET)))
                return out
            if ssize == 0:
                break
            o += 12 + ssize
    return []


def entries(b):
    count = struct.unpack_from("<I", b, 0x0C)[0]
    header_size = struct.unpack_from("<Q", b, 0x10)[0]
    entry_size = struct.unpack_from("<Q", b, 0x20)[0]
    for i in range(count):
        o = header_size + i * entry_size
        size = struct.unpack_from("<Q", b, o + 8)[0]
        data_off = struct.unpack_from("<I", b, o + 16)[0]
        name_off = struct.unpack_from("<I", b, o + 20)[0]
        end = name_off
        while b[end : end + 2] != b"\0\0":
            end += 2
        yield i, b[name_off:end].decode("utf-16-le"), data_off, size


def selftest():
    """The walk's three outcomes, on streams built here rather than on anyone's save."""

    def header(kind, size):
        return struct.pack("<III", kind, 0, size).ljust(SECTION_HEADER, b"\0")

    good = header(0, 4) + b"abcd" + header(5, 0) + header(SECTION_END, 0)
    cases = [
        # Slot 1 of the save that hard-locked: 111292 zero bytes. Type 0, size 0, to the end.
        (bytes(111292), "hangs", "an all-zero entry"),
        (b"", "hangs", "an entry the game would stream as a null buffer"),
        (good, "ends", "a chain that reaches its 0xD header"),
        # A size that jumps past the end clamps there, and the next read fails for good.
        (header(0, 10_000) + header(SECTION_END, 0), "hangs", "a size past the end"),
        (header(0x20, 0), "bails", "a type outside the table -- the walk's own error exit"),
        # The terminator can arrive in a partial read, which copies what is left.
        (header(0, 0)[:SECTION_HEADER] + struct.pack("<I", SECTION_END), "ends",
         "a 0xD header in the last, short read"),
    ]
    failed = 0
    for stream, want, what in cases:
        got = section_walk(stream)
        mark = "ok  " if got == want else "FAIL"
        failed += got != want
        print(f"  {mark} {what}: {got}")
    payload = struct.pack("<I", len(good)) + good + b"\0\0\0\0"
    if entry_stream(payload) != good:
        print("  FAIL the stream starts past the u32 length and is that long")
        failed += 1
    print("ds2-sl2.py selftest:", "OK" if not failed else f"{failed} FAILED")
    return 1 if failed else 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("save", nargs="?")
    ap.add_argument("--selftest", action="store_true",
                    help="check the section-walk emulation against built streams (no save needed)")
    ap.add_argument("-x", "--extract", metavar="DIR", help="write decrypted payloads here")
    ap.add_argument("--key-from-image", metavar="BIN", help="re-derive the key from the game image")
    ap.add_argument("--batches", action="store_true",
                    help="group entries by IV write batch (needs no key)")
    ap.add_argument("--vanilla", action="store_true",
                    help="use the vanilla DARK SOULS II key instead of the SOTFS one "
                         "(for DARKSII0000.sl2 from Steam app 236430)")
    ap.add_argument("--slots", action="store_true",
                    help="list the ten character slots and which hold a character")
    ap.add_argument("--key", metavar="HEX",
                    help="use this AES-128 key (32 hex chars) instead of either built-in")
    args = ap.parse_args()
    if args.selftest:
        return selftest()
    if not args.save:
        ap.error("a save file is required unless --selftest is given")

    if args.key and args.vanilla:
        ap.error("--key and --vanilla both choose a key; pass one")
    if args.key and args.key_from_image:
        ap.error("--key and --key-from-image both choose a key; pass one")
    if args.vanilla and args.key_from_image:
        ap.error("--vanilla and --key-from-image both choose a key; pass one")

    if args.key:
        key_hex = args.key.strip().replace(" ", "")
        if len(key_hex) != 32 or any(c not in "0123456789abcdefABCDEF" for c in key_hex):
            ap.error("--key wants 32 hex characters (an AES-128 key)")
    elif args.vanilla:
        key_hex = VANILLA_KEY_HEX
    elif args.key_from_image:
        key_hex = key_from_image(args.key_from_image)
    else:
        key_hex = KEY_HEX
    if args.key_from_image:
        print(f"key from {args.key_from_image}: {key_hex}"
              f"{'  (matches built-in)' if key_hex.upper() == KEY_HEX else '  ** DIFFERS **'}\n")

    b = Path(args.save).read_bytes()
    assert b[:4] == b"BND4", "not a BND4 container"
    out = Path(args.extract) if args.extract else None
    if out:
        out.mkdir(parents=True, exist_ok=True)

    if args.slots:
        rows = slot_records(b, key_hex)
        if not rows:
            print("no character-list section found -- not a layout this tool knows")
            return 1
        for slot, state, attrs, name in rows:
            extra = f"  stats={sum(attrs):<4} name={name}" if state != "empty" else ""
            print(f"slot {slot} {state:<11}{extra}")
        return 0

    if args.batches:
        groups = {}
        for i, name, off, size in entries(b):
            iv = b[off + 16 : off + 32]
            groups.setdefault(bytes(iv[j] for j in (0, 1, 4, 5, 8, 9, 12, 13)), []).append(name)
        print("write batches -- entries sharing an IV seed were written by one save operation:")
        for seed, names in groups.items():
            print(f"  {seed.hex()}  {', '.join(names)}")
        print()

    print(f"{'#':>3}  {'name':<13} {'size':>9}  {'payload':>9}  md5  head")
    for i, name, off, size in entries(b):
        blob = b[off : off + size]
        ok = "ok " if hashlib.md5(blob[16:]).digest() == blob[:16] else "BAD"
        payload = decrypt(blob[32:], blob[16:32], key_hex)
        print(f"{i:3}  {name:<13} {size:9}  {len(payload):9}  {ok}  {payload[:16].hex(' ')}")
        if out:
            (out / f"{name}.bin").write_bytes(payload)
    if out:
        print(f"\nwrote {out}/USER_DATA*.bin")


if __name__ == "__main__":
    sys.exit(main())
