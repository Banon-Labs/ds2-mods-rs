#!/usr/bin/env python3
"""Read DARK SOULS II's `*Ebl.bhd`/`*Ebl.bdt` archives, with no external tools.

    python3 scripts/ds2-ebl.py info
    python3 scripts/ds2-ebl.py hash /menu/42.febnd.dcx
    python3 scripts/ds2-ebl.py extract /menu/42.febnd.dcx --out /tmp/menu42

Everything this needs ships with the game. `GameDataKeyCode.pem` beside the archive is an RSA
PUBLIC key, and the header is decrypted by raw-exponentiating each 256-byte block with it -- the
header was "signed" with the private key, so the public key is the decryption key. No key had to
be found, extracted or guessed.

WHY THIS EXISTS rather than a note saying "use BinderTool". BinderTool and UXM are .NET tools for
Windows, and this repo's evidence rule is that a claim about the game's data has to be
reproducible here, from the artifact, by a command printed beside the claim. The whole chain is
about 150 lines of standard library, so vendoring a runtime to run someone else's implementation
would cost more than writing it.

THE CHAIN, and every step of it is checked rather than assumed:

  GameDataEbl.bhd    2206 blocks x 256 bytes, RSA-2048
    -> raw RSA with the shipped public key, 255 plaintext bytes per block
  BHD5               magic checked; 1931 buckets, 11699 entries
    -> every entry's `hash % bucketCount == its bucket index` is VERIFIED, not trusted. That is
       what proves the record layout below is right: a wrong stride or a wrong field offset
       scrambles the hashes and the check fails on essentially every entry.
  path -> hash       h = h * 37 + byte, over the LOWERCASED path, u32 wrapping
    -> the classic FromSoftware path hash. Confirmed against paths read out of the executable's
       own string table (`/material/AllMaterialBnd.bnd`, `/sound/frpg2_main.bnd`), not against a
       filename list from somewhere else.
  GameDataEbl.bdt    seek to the entry's offset, read its size
  DCX / DFLT         zlib at a FIXED offset from the header, not at the first `78` byte found
  BND4               a small file table; entries carry a name

WHAT IS NOT IMPLEMENTED, deliberately, and named rather than left as a surprise: 1960 of the
11699 entries carry an AES key record (a nonzero key offset at +0x18). Those are encrypted in
ranges and this does not decrypt them. It REFUSES such an entry instead of handing back
ciphertext that looks like a corrupt file. Nothing this repo has needed so far is one of them --
`/menu/42.febnd.dcx`, the pause menu's layout, is not.

The BDT is 13 GB. Nothing here reads more of it than one entry.
"""

from __future__ import annotations

import argparse
import base64
import struct
import sys
import zlib
from pathlib import Path

GAME_DIR = (
    Path.home()
    / ".local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game"
)

#: The archive this defaults to. There are five `*Ebl` pairs; the others take `--archive`.
DEFAULT_ARCHIVE = "GameDataEbl"

#: One RSA block in, this many plaintext bytes out. The decrypted 256-byte big-endian integer
#: always has a zero top byte -- it must, being smaller than the modulus -- and the payload is
#: everything after it. Checked rather than assumed: `BHD5` lands at plaintext offset 0 of block 0
#: only with this value, and the concatenated stream's length then covers the header's own
#: self-reported size.
RSA_PLAINTEXT_BYTES = 255

BHD5_MAGIC = b"BHD5"
#: `struct BHD5Entry { u32 pathHash; u32 size; u64 dataOffset; i64 saltedHashOffset; i64 aesKeyOffset; }`
ENTRY_FORMAT = "<IIQqq"
ENTRY_SIZE = struct.calcsize(ENTRY_FORMAT)

DCX_MAGIC = b"DCX\0"
#: Where the deflate stream starts in a DS2 `DCX`/`DFLT`. Fixed by the header shape, and taken as
#: a constant on purpose: searching for a zlib signature finds one 57 KB into the compressed data
#: of this very file and decompresses to garbage.
DCX_PAYLOAD_OFFSET = 0x4C

BND4_MAGIC = b"BND4"


def path_hash(path: str) -> int:
    """`h = h * 37 + byte` over the lowercased path, u32 wrapping."""
    h = 0
    for byte in path.lower().encode("utf-8"):
        h = (h * 37 + byte) & 0xFFFF_FFFF
    return h


def rsa_public_key(pem_path: Path) -> tuple[int, int]:
    """`(modulus, exponent)` out of a PKCS#1 `RSA PUBLIC KEY` PEM.

    Hand-parsed DER rather than a dependency: it is two INTEGERs inside a SEQUENCE, and adding a
    `cryptography` dependency to this repo to read them would be the tail wagging the dog. It also
    would not help -- no mainstream binding exposes the RAW modexp this format needs, only padded
    encrypt/decrypt, which this is not.
    """
    body = "".join(line for line in pem_path.read_text().splitlines() if "-----" not in line)
    der = base64.b64decode(body)

    def read(buf: bytes, index: int) -> tuple[bytes, int]:
        index += 1  # tag
        length = buf[index]
        index += 1
        if length & 0x80:
            count = length & 0x7F
            length = int.from_bytes(buf[index : index + count], "big")
            index += count
        return buf[index : index + length], index + length

    sequence, _ = read(der, 0)
    modulus, index = read(sequence, 0)
    exponent, _ = read(sequence, index)
    return int.from_bytes(modulus, "big"), int.from_bytes(exponent, "big")


def decrypt_bhd(bhd_path: Path, pem_path: Path) -> bytes:
    modulus, exponent = rsa_public_key(pem_path)
    blob = bhd_path.read_bytes()
    if len(blob) % 256:
        raise SystemExit(f"{bhd_path} is not a whole number of 256-byte RSA blocks")
    out = bytearray()
    for offset in range(0, len(blob), 256):
        block = int.from_bytes(blob[offset : offset + 256], "big")
        out += pow(block, exponent, modulus).to_bytes(256, "big")[256 - RSA_PLAINTEXT_BYTES :]
    if bytes(out[:4]) != BHD5_MAGIC:
        raise SystemExit(
            f"decrypted header does not start with {BHD5_MAGIC!r} -- got {bytes(out[:4])!r}. "
            "Either the .pem does not belong to this .bhd, or the block layout differs on this build."
        )
    return bytes(out)


class Bhd5:
    """A parsed BHD5 header, and the check that says the parse is right."""

    def __init__(self, blob: bytes) -> None:
        self.blob = blob
        (self.declared_size, self.bucket_count, self.bucket_offset, salt_length) = (
            struct.unpack_from("<IIII", blob, 0x0C)
        )
        self.salt = blob[0x1C : 0x1C + salt_length]
        self.entries: dict[int, tuple[int, int, int, int]] = {}
        self.bucket_violations = 0
        for bucket in range(self.bucket_count):
            count, offset = struct.unpack_from("<II", blob, self.bucket_offset + bucket * 8)
            for index in range(count):
                record = struct.unpack_from(ENTRY_FORMAT, blob, offset + index * ENTRY_SIZE)
                path_hash_, size, data_offset, _salted, aes_key = record
                if path_hash_ % self.bucket_count != bucket:
                    self.bucket_violations += 1
                self.entries[path_hash_] = (size, data_offset, aes_key, bucket)

    def lookup(self, virtual_path: str) -> tuple[int, int, int, int]:
        wanted = path_hash(virtual_path)
        entry = self.entries.get(wanted)
        if entry is None:
            raise SystemExit(
                f"no entry for {virtual_path!r} (hash 0x{wanted:08x}).\n"
                "    The path is the one INSIDE the archive, so `menu:/42.febnd.dcx` in the\n"
                "    executable is `/menu/42.febnd.dcx` here."
            )
        return entry


def read_entry(bdt_path: Path, size: int, offset: int, aes_key: int, what: str) -> bytes:
    if aes_key:
        raise SystemExit(
            f"{what} is one of the AES-encrypted entries (key record at {aes_key:#x}).\n"
            "    This tool does not decrypt those, and will not hand back ciphertext dressed\n"
            "    up as a file. See the module docstring."
        )
    with bdt_path.open("rb") as handle:
        handle.seek(offset)
        data = handle.read(size)
    if len(data) != size:
        raise SystemExit(f"{what}: wanted {size} bytes at {offset:#x}, got {len(data)}")
    return data


def dcx_decompress(data: bytes) -> bytes:
    """Unwrap a DS2 `DCX`/`DFLT`. Returns the input unchanged if it is not a DCX."""
    if data[:4] != DCX_MAGIC:
        return data
    uncompressed_size, compressed_size = struct.unpack_from(">II", data, 0x1C)
    out = zlib.decompress(data[DCX_PAYLOAD_OFFSET:])
    if len(out) != uncompressed_size:
        raise SystemExit(
            f"DCX says {uncompressed_size} bytes uncompressed, got {len(out)}. "
            f"(compressed size field: {compressed_size})"
        )
    return out


def bnd4_files(data: bytes) -> list[tuple[str, bytes]]:
    """`[(name, bytes)]` out of a BND4. Empty list if this is not one."""
    if data[:4] != BND4_MAGIC:
        return []
    count = struct.unpack_from("<I", data, 0x0C)[0]
    header_size = struct.unpack_from("<Q", data, 0x10)[0]
    entry_size = struct.unpack_from("<Q", data, 0x20)[0]
    out = []
    for index in range(count):
        base = header_size + index * entry_size
        # `flags, pad[3], -1, compressedSize, uncompressedSize, dataOffset, id, nameOffset`.
        # The two sizes are equal in every entry seen here -- these members are stored
        # uncompressed inside an archive that is itself DCX'd -- but both are read rather than
        # one assumed, so a compressed member would be visible instead of silently truncated.
        size, uncompressed, data_offset, _id, name_offset = struct.unpack_from(
            "<QQIiI", data, base + 0x08
        )
        if size != uncompressed:
            raise SystemExit(
                f"BND4 member {index} is compressed ({size} -> {uncompressed}); not handled"
            )
        end = data.index(b"\0", name_offset)
        name = data[name_offset:end].decode("utf-8", "replace")
        out.append((name, data[data_offset : data_offset + size]))
    return out


def archive_paths(directory: Path, archive: str) -> tuple[Path, Path, Path]:
    bhd = directory / f"{archive}.bhd"
    bdt = directory / f"{archive}.bdt"
    pem = directory / f"{archive.replace('Ebl', 'KeyCode')}.pem"
    for path in (bhd, bdt, pem):
        if not path.is_file():
            raise SystemExit(f"not found: {path}")
    return bhd, bdt, pem


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--game-dir", type=Path, default=GAME_DIR)
    parser.add_argument("--archive", default=DEFAULT_ARCHIVE)
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("info", help="decrypt the header and report what is in it")

    p_hash = sub.add_parser("hash", help="the archive hash of a virtual path")
    p_hash.add_argument("path")

    p_extract = sub.add_parser("extract", help="pull one file out, unwrapping DCX and BND4")
    p_extract.add_argument("path")
    p_extract.add_argument("--out", type=Path, required=True, help="directory to write into")
    p_extract.add_argument(
        "--raw", action="store_true", help="write the stored bytes without unwrapping anything"
    )

    args = parser.parse_args()

    if args.command == "hash":
        print(f"0x{path_hash(args.path):08x}  {args.path}")
        return 0

    bhd_path, bdt_path, pem_path = archive_paths(args.game_dir, args.archive)
    header = Bhd5(decrypt_bhd(bhd_path, pem_path))

    if args.command == "info":
        encrypted = sum(1 for size, off, key, b in header.entries.values() if key)
        print(f"archive       {args.archive}")
        print(f"salt          {header.salt.decode('ascii', 'replace')}")
        print(f"buckets       {header.bucket_count}")
        print(f"entries       {len(header.entries)}")
        print(f"AES-encrypted {encrypted}")
        # THE PARSE'S OWN PROOF. Printed every run rather than asserted once in a comment: if a
        # future build changes the record stride this is the line that says so, instead of the
        # tool quietly returning entries that decode to nothing.
        print(
            f"bucket check  {header.bucket_violations} of {len(header.entries)} entries violate "
            f"hash % {header.bucket_count} == bucket"
        )
        if header.bucket_violations:
            print("  ^ NONZERO. The record layout is wrong for this build; do not trust extracts.")
            return 1
        return 0

    size, offset, aes_key, _bucket = header.lookup(args.path)
    data = read_entry(bdt_path, size, offset, aes_key, args.path)
    args.out.mkdir(parents=True, exist_ok=True)
    stem = args.path.rsplit("/", 1)[-1]
    print(f"{args.path}  hash=0x{path_hash(args.path):08x} size={size} offset={offset:#x}")

    if args.raw:
        target = args.out / stem
        target.write_bytes(data)
        print(f"  wrote {target} ({len(data)} bytes, as stored)")
        return 0

    unwrapped = dcx_decompress(data)
    if unwrapped is not data:
        print(f"  DCX -> {len(unwrapped)} bytes")
    members = bnd4_files(unwrapped)
    if not members:
        target = args.out / stem.removesuffix(".dcx")
        target.write_bytes(unwrapped)
        print(f"  wrote {target} ({len(unwrapped)} bytes)")
        return 0
    print(f"  BND4 -> {len(members)} file(s)")
    for name, blob in members:
        target = args.out / name.rsplit("\\", 1)[-1]
        target.write_bytes(blob)
        print(f"    wrote {target} ({len(blob)} bytes) magic={blob[:4]!r}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
