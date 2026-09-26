# `sfxcommon.ffxbnd` holds no effects

`Game/sfx/sfxcommon.ffxbnd` is the one bundle under `Game/sfx/` whose file entries
`scripts/ds2-ffx.py` used to refuse, so its FFX action ids were never checked against the id ->
class table in `docs/ffx-actions.md` (section 3). This closes that gap.

## Format

It is a bare `BND4` (no `DCX` wrapper). The header's stored format byte is `0x70`. SoulsFormats
`Binder.ReadFormat` keeps the byte as stored only when its low bit is set and its high bit clear, or
when the header's bit-big-endian byte is set; otherwise it reverses the bit order. `0x70` reverses to
`0x0e`: ids, both name flags, and no compression flag. Without the compression flag an entry carries
no uncompressed-size field, which is why its entries are shorter than the ones in
`sfx9999.ffxbnd.dcx` (stored `0x74`, reversed `0x2e`, compression set). Its unicode byte is clear, so
names are single-byte strings rather than UTF-16.

The reader now builds the entry fields from those flags and refuses a header whose declared entry
size disagrees with what the flags imply. `python3 scripts/ds2-ffx.py --selftest` builds both
layouts and the mismatch case.

## Contents

```
$ python3 scripts/ds2-ffx.py list --bundle ".../Game/sfx/sfxcommon.ffxbnd"
       0         0    C:\Documents and Settings\wataa\My Documents\dummy.dmy
```

The only member is a zero-length placeholder with id `0`, not a `DLsE` file. There are no FFX
action ids to scan, and the table in `docs/ffx-actions.md` needs no entry from this bundle.
