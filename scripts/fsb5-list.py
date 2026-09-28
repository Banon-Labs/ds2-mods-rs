#!/usr/bin/env python3
"""List the subsounds of FMOD FSB5 banks: name, length in seconds, loop points (samples).

Usage: fsb5-list.py BANK.fsb [...]   (read-only; parses the header/name table only)
"""
import os
import struct
import sys

RATES = {1: 8000, 2: 11000, 3: 11025, 4: 16000, 5: 22050, 6: 24000, 7: 32000, 8: 44100, 9: 48000}

for p in sys.argv[1:]:
    b = open(p, "rb").read(1 << 20)
    n, sh, ns, ds, mode = struct.unpack_from("<IIIII", b, 8)
    base = 0x3C + sh
    offs = struct.unpack_from("<%dI" % n, b, base)
    names = [b[base + o : b.index(b"\0", base + o)].decode("latin1") for o in offs]
    pos, out = 0x3C, []
    for i in range(n):
        v = struct.unpack_from("<Q", b, pos)[0]
        pos += 8
        nxt = v & 1
        r = RATES.get((v >> 1) & 0xF, 0)
        samples = v >> 34
        loop = None
        while nxt:
            h = struct.unpack_from("<I", b, pos)[0]
            pos += 4
            nxt, sz, typ = h & 1, (h >> 1) & 0xFFFFFF, (h >> 25) & 0x7F
            if typ == 2:
                r = struct.unpack_from("<I", b, pos)[0]
            if typ == 3:
                loop = struct.unpack_from("<II", b, pos)
            pos += sz
        out.append((names[i], round(samples / r, 1) if r else 0, loop))
    print(os.path.basename(p), n, "mode=%#x" % mode, [o for o in out if o[0][:1] in "mo" or o[0].startswith(("bgm","boss"))] or ("no-m", len(out), out[:3]))
