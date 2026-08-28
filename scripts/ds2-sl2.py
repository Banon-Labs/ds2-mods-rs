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


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("save")
    ap.add_argument("-x", "--extract", metavar="DIR", help="write decrypted payloads here")
    ap.add_argument("--key-from-image", metavar="BIN", help="re-derive the key from the game image")
    ap.add_argument("--batches", action="store_true",
                    help="group entries by IV write batch (needs no key)")
    args = ap.parse_args()

    key_hex = key_from_image(args.key_from_image) if args.key_from_image else KEY_HEX
    if args.key_from_image:
        print(f"key from {args.key_from_image}: {key_hex}"
              f"{'  (matches built-in)' if key_hex.upper() == KEY_HEX else '  ** DIFFERS **'}\n")

    b = Path(args.save).read_bytes()
    assert b[:4] == b"BND4", "not a BND4 container"
    out = Path(args.extract) if args.extract else None
    if out:
        out.mkdir(parents=True, exist_ok=True)

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
