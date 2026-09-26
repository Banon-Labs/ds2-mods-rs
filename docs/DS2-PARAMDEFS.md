# DARK SOULS II paramdefs for fromsoftware-rs's param-generator

Where a Scholar of the First Sin paramdef set can come from, whether `tools/param-generator` can
read it, what licence it carries, and how to check it against the game's own rows.

Researched 2026-09-25 by reading the upstream repositories through the GitHub API. Nothing was
downloaded into this repo or into fromsoftware-rs, and the game install was not touched.

## Answer

A usable set exists. Smithbox ships SotFS paramdefs as Paramdex-format XML at
[`src/Smithbox.Data/Assets/PARAM/DS2S/Defs`](https://github.com/vawser/Smithbox/tree/main/src/Smithbox.Data/Assets/PARAM/DS2S/Defs),
one file per param type, under the MIT licence. It is the same source fromsoftware-rs already
vendors for Elden Ring, DS3, Sekiro and Nightreign, and a static comparison against the generator's
parser finds nothing it would reject. Last change to that directory at the time of writing:
Smithbox commit [`65879741cd3e`](https://github.com/vawser/Smithbox/commit/65879741cd3e)
("DS2 Defs", 2026-05-16).

The earlier conclusion that the binary contributes nothing still holds, but the grep it rested on
looked for the wrong thing: DS2's param types do not end in `_ST`. They are named `WEAPON_PARAM`,
`ARMOR_PARAM`, `ITEM_LOT_PARAM2` and so on; only a handful carry `_ST`.

## Sources compared

| Source | DS2S defs | Format | Licence | State |
| --- | --- | --- | --- | --- |
| [vawser/Smithbox](https://github.com/vawser/Smithbox) `src/Smithbox.Data/Assets/PARAM/DS2S/Defs` | yes | Paramdex XML | [MIT](https://github.com/vawser/Smithbox/blob/main/LICENSE) (Vawser 2025; Katalash, Meowmaritus 2018) | active |
| [soulsmods/Paramdex](https://github.com/soulsmods/Paramdex) `DS2S/Defs` | yes, minus `PLAYER_ACTION_PARAM` and `PLAYER_MOVEMENT_PARAM` | Paramdex XML, `XmlVersion="1"` | none declared | last push 2026-03 |
| [soulsmods/DSMapStudio](https://github.com/soulsmods/DSMapStudio) `src/StudioCore/Assets/Paramdex/DS2S` | yes | Paramdex XML | MIT | last push 2025-05; superseded by Smithbox |
| [JKAnderson/Yapped](https://github.com/JKAnderson/Yapped) | no DS2 support found | -- | GPL-3.0 | archived |
| [soulsmods/SoulsFormatsNEXT](https://github.com/soulsmods/SoulsFormatsNEXT), [JKAnderson/SoulsFormats](https://github.com/JKAnderson/SoulsFormats) | no defs; the reader and the regulation decryptor | C# | GPL-3.0 | NEXT active, original archived |

Paramdex's README states the DS2S defs were reverse-engineered by the community (only DeS, DS1 and
BB shipped official defs), so field names and some layouts are community guesses, not From's.

## Licence notes

- Smithbox's MIT grant is the only explicit licence on any copy of the DS2S defs. Vendoring them
  means keeping the MIT copyright and permission notice beside the files, for example a
  `NOTICE` or `LICENSE-SMITHBOX` in `tools/param-generator/params/darksouls2/`.
- The defs originate in Paramdex, which declares no licence at all. Vawser is a listed Paramdex
  contributor and relicenses the copy inside Smithbox; the other contributors have not granted
  anything themselves. fromsoftware-rs already accepts this position for its other four games
  (`param_def.rs` describes its input as "the Smithbox XML", and its Elden Ring history reads
  "Sync ER params with the latest Smithbox defs"), so a DS2 set adds no new exposure. Take the
  files from Smithbox, not from Paramdex.
- Do not port SoulsFormats code. It is GPL-3.0, and Smithbox's embedded copy under
  `src/Andre/SoulsFormats` inherits that. The regulation algorithm below is a fact that can be
  reimplemented; the C# is not something to translate line for line into an MIT/Apache crate.

## Schema fit

The generator reads only three things (`tools/param-generator/src/param_def.rs`):
`<ParamType>`, an optional `<Index>`, and each `<Field Def="...">` plus an optional
`RemovedVersion` attribute. Everything else in the XML is ignored.

A DS3 file (`params/darksouls3/ActionButtonParam.xml`) and a Smithbox DS2S file
(`WEAPON_PARAM.xml`) share that shape. The headers differ -- DS2S carries `DataVersion`,
`FormatVersion` 104/105 and mostly `BigEndian true` where DS3 carries `Version` 201 and `Index` --
but the generator reads none of those.

Checked across every DS2S file, streamed and grepped, not saved:

- **Field types**: only `u8 s8 u16 s16 u32 s32 f32 angle32 dummy8 fixstr`, all of which
  `FieldType::alignment_and_size` handles. No `f64`, which Paramdex allows and the generator would
  panic on.
- **Bitfields**: all on `u8` or `dummy8`. The generator lays every bitfield out in single-byte
  groups, so this matters; DS3 has `u32` bitfields and still works, but DS2 does not exercise that
  path. No bitfield also declares an array length (that combination is `unimplemented!()`).
- **Names**: every field name is a plain identifier, no duplicates inside a param, and no two
  collapse to the same name once lower-cased with underscores removed (a proxy for
  `normalize_name` collisions).
- **`RemovedVersion`, `FirstVersion`, `Index`**: none present. With no `Index` anywhere, the
  generator's all-or-none rule is satisfied and no `ParamDef::INDEX` is emitted. That is correct
  for DS2: DS3's index is a position in `CSRegulationManager`'s sorted list, and DS2 has no such
  list to index.
- **Param type names**: unique across the set, so no two structs would collide.

Not checked: whether the generated file compiles and whether `rustfmt` accepts it. Those need the
files on disk, which this research deliberately avoided.

## Several files per type

DS2 keeps many `.param` files that share one type (per-map `GENERATOR_PARAM`,
`GENERATOR_LOCATION_PARAM`, `ITEM_LOT_PARAM2`, and so on), plus loose params under `Param\` in the
game directory. Smithbox resolves file name to type through
[`Param Type Info.json`](https://github.com/vawser/Smithbox/blob/main/src/Smithbox.Data/Assets/PARAM/DS2S/Param%20Type%20Info.json)
(`"CameraGrapplerParam": "CAMERA_GPAPPLER_PARAM"`, `"CameraNoiseParam": "NOISE_PARAM"`). The
generator emits one struct per type, which is what a binding wants; the file-to-type map is only
needed by whatever reads rows at runtime, and that is a question about DS2's in-memory param
manager, not about the defs.

## Decrypting the regulation to validate row sizes

From Smithbox's SoulsFormats copy,
[`SFUtil.DecryptDS2Regulation`](https://github.com/vawser/Smithbox/blob/main/src/Andre/SoulsFormats/SoulsFormats/Util/SFUtil.cs)
with the key in
[`Keys.cs`](https://github.com/vawser/Smithbox/blob/main/src/Andre/SoulsFormats/SoulsFormats/Util/Keys.cs),
and its use in `ParamBank.LoadParameters_DS2`:

- File: `enc_regulation.bnd.dcx` in the game directory. If it already reads as a BND4 it is used
  as is (modded installs ship it decrypted).
- Cipher: AES-128-CTR. Key `40 17 81 30 DF 0A 94 54 33 09 E1 71 EC BF 25 4C`.
- IV: `0x80`, followed by the file's header from offset `0x00` up to `0x0A` inclusive, then
  `00 00 00 01`.
- Ciphertext starts at offset `0x20`; the plaintext is a BND4 of `.param` files.

A validation tool therefore needs AES-CTR, a BND4 reader and a PARAM header reader. Row size is
not stored in a PARAM header; it is the stride between consecutive rows' data offsets (what
SoulsFormats calls the detected size). Comparing that stride with `size_of` of each generated
struct, for every `.param` in the BND4 and every loose `Param\*.param`, proves the defs against the
shipping data. The PARAM header's paramdef data version can also be compared with each XML's
`DataVersion`.

## Recommended path

1. In the fromsoftware-rs fork (`chozandrias76/fromsoftware-rs`), copy
   `src/Smithbox.Data/Assets/PARAM/DS2S/Defs/*.xml` from a pinned Smithbox commit into
   `tools/param-generator/params/darksouls2/`, add Smithbox's MIT notice beside them, and record
   the commit hash in the README line that documents the rebuild command.
2. Run `param-generator --input tools/param-generator/params/darksouls2 --output
   crates/darksouls2/src/param/generated.rs` unchanged, then `rustfmt`. The static check above
   predicts no parser change is needed.
3. Before anything reads a row through those structs, build the host-side validator described
   above as a small tool in this repo, reading `enc_regulation.bnd.dcx` read-only, and fail on any
   stride that disagrees with `size_of`. Community-reversed defs are the likeliest place for a
   wrong size, and a wrong size silently corrupts every field after it.
4. Leave indices off. They would have to come from DS2's runtime param table, which nobody has
   mapped yet.
