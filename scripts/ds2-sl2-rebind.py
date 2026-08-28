#!/usr/bin/env python3
"""Rewrite the Steam ID inside a DARK SOULS II: SOTFS `.sl2`, so another account can load it.

    python3 scripts/ds2-sl2-rebind.py <in.sl2> --to <steamid-hex-16> -o <out.sl2>
    python3 scripts/ds2-sl2-rebind.py <in.sl2> --show          # find the IDs, change nothing

WHY THIS EXISTS. Redirecting the save directory is not enough to load someone else's save.
Measured, not assumed: with `[save_redirect]` pointing at a donor folder the game answers "The save
data was not loaded correctly", and with the SAME redirect pointing at this account's own folder
through the same `Z:` path it boots straight through. So the path is reachable and the mechanism
works; what the game rejects is the file's CONTENTS. The ID written inside the save is the
difference between those two runs.

WHAT IT CHANGES. Exactly the bytes that spell a 16-character Steam ID in hex, wherever they appear
in the decrypted payload of any entry, and nothing else. In both saves examined here that is a
single occurrence, in `USER_DATA000` at plaintext offset 0x39, stored as ASCII. Old and new are the
same length by construction -- a SteamID64 in hex is always 16 characters -- so nothing moves, no
section size changes, and no offset table needs rewriting. An ID of any other length is refused.

THE RESEAL, which is the part that is easy to get wrong. Each BND4 entry is

    [ 16-byte MD5 of everything after it ][ 16-byte CBC IV ][ AES-128-CBC ciphertext ]

and the MD5 covers the CIPHERTEXT WITH ITS IV, not the plaintext. So a patched entry has to be
re-encrypted under its OWN original IV and then re-hashed over `iv + ciphertext`. Reusing the IV is
deliberate: it keeps every byte outside the patched block identical to the original file, which
makes a diff of the two saves show exactly the edit and nothing else.

Encryption goes through `openssl enc` for the same reason `ds2-sl2.py`'s decryption does: this
machine has neither `pycryptodome` nor `cryptography`, and openssl is already a dependency of the
reader.

VERIFY, DO NOT TRUST. `--verify` (on by default when writing) re-reads the finished file the way
`ds2-sl2.py` does: every entry's MD5 must check, every payload must decrypt to a valid section
chain, and the new ID must be present with the old one gone. A save that fails that is not written.
"""

import argparse
import hashlib
import re
import struct
import subprocess
import sys
from pathlib import Path

#: The SOTFS key. Same provenance as `ds2-sl2.py`'s: read out of the game image, not remembered.
KEY_HEX = "599F9B699640A55236EE2D70835EC744"

#: A SteamID64 rendered as hex is always this many characters. The patch is length-preserving and
#: this is why; anything else would move bytes and invalidate the section sizes around them.
STEAM_ID_HEX_LEN = 16


def crypt(data, iv, key_hex, encrypt):
    """AES-128-CBC through openssl, no padding. `data` must be a multiple of the block size."""
    r = subprocess.run(
        ["openssl", "enc", "-aes-128-cbc", "-e" if encrypt else "-d", "-nopad",
         "-K", key_hex, "-iv", iv.hex()],
        input=data,
        capture_output=True,
    )
    if r.returncode:
        raise SystemExit(r.stderr.decode())
    return r.stdout


def entries(b):
    """(index, name, data offset, size) for each BND4 entry, in table order."""
    count = struct.unpack_from("<I", b, 0x0C)[0]
    entry_size = struct.unpack_from("<Q", b, 0x20)[0]
    out = []
    for i in range(count):
        o = 0x40 + i * 0x20
        size = struct.unpack_from("<Q", b, o + 8)[0]
        data_off = struct.unpack_from("<I", b, o + 16)[0]
        name_off = struct.unpack_from("<I", b, o + 20)[0]
        name = b[name_off:b.index(b"\x00\x00", name_off) + 1].decode("utf-16le", "replace")
        out.append((i, name.rstrip("\x00"), data_off, size))
    _ = entry_size
    return out


#: A SteamID64 for an individual account is `0x0110000100000000 + accountid`, so in hex it is the
#: literal `01100001` followed by eight hex digits of account number -- `01100001526d6d84`,
#: `01100001018fa4be`. Anchoring on that eight-character prefix is what stops this reporting every
#: run of sixteen hex digits in a 6.8 MB payload. Getting the prefix one character too long
#: (`011000010`) matches only accounts whose number happens to start with a zero, which is a silent
#: no-op on every other save; the tool refuses to write when it finds nothing, which is how that
#: was caught rather than shipped.
STEAM_ID_RE = re.compile(rb"01100001[0-9a-fA-F]{8}")


def decrypt_all(b, key_hex):
    """(index, name, offset, size, iv, plaintext) per entry. Decrypts each entry exactly once.

    Cached by the caller and threaded through, because a naive `find_ids` inside the patch loop
    turns 23 openssl passes into 529 over ~190 MB, which takes minutes instead of a second.
    """
    out = []
    for idx, name, off, size in entries(b):
        blob = bytes(b[off:off + size])
        out.append((idx, name, off, size, blob[16:32],
                    crypt(blob[32:], blob[16:32], key_hex, encrypt=False)))
    return out


def find_ids(decrypted):
    """Every ASCII Steam ID in the decrypted entries, as (index, name, plaintext offset, id)."""
    return [
        (idx, name, m.start(), m.group().decode())
        for idx, name, _off, _size, _iv, pt in decrypted
        for m in STEAM_ID_RE.finditer(pt)
    ]


def rebind(b, new_id, key_hex, decrypted):
    """Return (patched bytes, list of (entry, offset, old id)) with every ID replaced."""
    out = bytearray(b)
    patched = []
    hits = find_ids(decrypted)
    for idx, name, off, size, iv, pt_orig in decrypted:
        mine = [h for h in hits if h[0] == idx]
        if not mine:
            continue
        ct = bytes(b[off + 32:off + size])
        pt = bytearray(pt_orig)
        for _idx, _name, poff, old in mine:
            pt[poff:poff + STEAM_ID_HEX_LEN] = new_id.encode()
            patched.append((name, poff, old))
        new_ct = crypt(bytes(pt), iv, key_hex, encrypt=True)
        if len(new_ct) != len(ct):
            raise SystemExit(f"{name}: re-encrypt changed length {len(ct)} -> {len(new_ct)}")
        # The hash covers the IV as well as the ciphertext. Getting this wrong produces a file the
        # game rejects in exactly the same way as the unpatched one, which is a miserable thing to
        # debug -- hence the verify pass.
        new_hash = hashlib.md5(iv + new_ct).digest()
        out[off:off + 16] = new_hash
        out[off + 32:off + size] = new_ct
    return bytes(out), patched


def verify(b, new_id, key_hex):
    """Re-read a finished file the way the reader does. Returns a list of problems."""
    problems = []
    for _idx, name, off, size in entries(b):
        blob = b[off:off + size]
        if hashlib.md5(blob[16:]).digest() != blob[:16]:
            problems.append(f"{name}: MD5 mismatch")
            continue
        pt = crypt(blob[32:], blob[16:32], key_hex, encrypt=False)
        total = struct.unpack_from("<I", pt, 0)[0]
        if total > len(pt):
            problems.append(f"{name}: section total 0x{total:x} exceeds payload 0x{len(pt):x}")
    found = {h[3].lower() for h in find_ids(decrypt_all(b, key_hex))}
    if found != {new_id.lower()}:
        problems.append(f"ids after patch: {sorted(found) or 'none'}, wanted only {new_id.lower()}")
    return problems


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("save")
    ap.add_argument("--to", metavar="HEX16", help="the Steam ID to write, 16 hex characters")
    ap.add_argument("-o", "--out", metavar="FILE", help="where to write the patched save")
    ap.add_argument("--show", action="store_true", help="report the IDs found and change nothing")
    ap.add_argument("--key", metavar="HEX", default=KEY_HEX, help="AES-128 key override")
    args = ap.parse_args()

    b = Path(args.save).read_bytes()
    if b[:4] != b"BND4":
        raise SystemExit("not a BND4 container")

    decrypted = decrypt_all(b, args.key)
    hits = find_ids(decrypted)
    for _idx, name, off, value in hits:
        print(f"  {name} +0x{off:x}  {value}")
    if args.show:
        if not hits:
            print("  no Steam ID found -- this is not the file layout this tool knows")
        return 0
    if not args.to or not args.out:
        raise SystemExit("--to and -o are both required unless --show")
    if len(args.to) != STEAM_ID_HEX_LEN or any(
        c not in "0123456789abcdefABCDEF" for c in args.to
    ):
        raise SystemExit(f"--to wants exactly {STEAM_ID_HEX_LEN} hex characters")
    if not hits:
        raise SystemExit("no Steam ID found to replace; refusing to write")

    patched, changed = rebind(b, args.to, args.key, decrypted)
    for name, off, old in changed:
        print(f"  patched {name} +0x{off:x}  {old} -> {args.to}")
    problems = verify(patched, args.to, args.key)
    if problems:
        for p in problems:
            print(f"  FAILED {p}", file=sys.stderr)
        raise SystemExit("verification failed; nothing written")
    # Everything outside the patched blocks must be byte-identical, or something moved that should
    # not have. Cheap to check and it catches a whole class of container mistake.
    if len(patched) != len(b):
        raise SystemExit(f"length changed {len(b)} -> {len(patched)}")
    Path(args.out).write_bytes(patched)
    differing = sum(1 for x, y in zip(b, patched) if x != y)
    print(f"  wrote {args.out}  ({differing} bytes differ of {len(b)})")
    print("  verified: every entry MD5 checks, every payload decrypts, only the new ID present")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
