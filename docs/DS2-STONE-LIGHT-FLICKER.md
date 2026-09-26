# Prism Stone: a flickering glow that casts light

Static RE and offline data reads, 2026-09-25. No game run. This turns `docs/ffx-actions.md`
section 7 (curve types, the 11001 point light) into the exact bytes a derived copy of the stone
effect needs, for `crates/ds2-invasion-path/src/stone_effect.rs`.

Tags: **verified in binary** (an address in `DarkSoulsII.exe`, flat image
`darksoulsii-deobf.bin`, base `0x140000000`), **verified in data** (read out of
`Game/sfx/sfx9999.ffxbnd.dcx`, offsets as `scripts/ds2-ffx.py tree` prints them), **inferred**.

## 1. What the stone copy is today

`strip_sparkles` finds the one `Param type 37` naming `2032` (833: object at `0x0c30`, length
`0xce6`) and writes `0` over the id, so action 14 inside the glow's `2101` spawns nothing in that
slot. The glow itself is the action-59 billboard at `0x0468` (**verified in data**).

## 2. The light: put 181's light child where the sparkles were

Effect 181 is a light and nothing else. Its root action 14, slot 2, is a `Param type 37` naming
`2101` (object `0x0414`, length `0x37a`) whose appearance arg is action `11001`
(`SfxFxDrawEntityHostPointLight`) (**verified in data**). Action 14 spawns every non-empty child
slot (`0x140f51e90`), and 11001 dispatches through `SfxFxActionHandler` `0x140bef150` ->
`0x140bef460`, by action id, with no lookup in the effect's ResourceSet (**verified in binary**).

So the stone copy replaces the whole 2032 object with 181's 2101 object, byte for byte:

- 833..839 and 181 have the **same class table** (the same names in the same order), so the
  copied object's class indices mean the same thing in the stone file (**verified in data**, every
  stone colour and 181). The code must assert this, not assume it.
- In 833..838 the 2032 object sits at `0x0c30` with length `0xce6` (**verified in data**). In 839
  it is `0xd1e`: the colour's extra bytes are inside the sparkle object itself, which the
  implementation's host tests found. Locate it by pattern (as `strip_sparkles` already does), not
  by offset or length, and take the enclosing lengths' delta from the object actually found.
- Every enclosing object's i32 length (at object `+6`) grows by `0x37a` minus the sparkle object's length (`-0x96c` in 833..838, `-0x9a4` in 839). In 833
  those are `0x019e` Effect, `0x038c` StateMap, `0x039a` State, `0x03ac` Action 79, `0x03ba`
  ParamList, `0x03cc` Param 37 (2101 glow), `0x03de` ParamList, `0x0be8` Param 38 (action 14),
  `0x0bfa` ParamList (**verified in data**). No count changes: a ParamList's two ints count params,
  not bytes. The file header before the class table holds no total size (**verified in data**).
- The light is a child of the glow, so it lives as long as the glow and sits at its origin
  (transform action 35 in the copied args is all zeros) (**inferred**).

Light values to write inside the copied object. Offsets are relative to the start of the copied
Param 37 (`0x0414` in 181); the value sits at primitive object `+10`:

| 11001 param | primitive at | 181 value | candle value |
| --- | --- | --- | --- |
| p0 colour A (FXColorRGBA) | `+0x0e0` | 1, .627, .251, .251 | stone glow rgb, alpha .251 |
| p1 colour B (FXColorRGBA) | `+0x11a` | 1, .627, .251, .059 | stone glow rgb, alpha .059 |
| p2 radius (float key) | `+0x154` | 10.0 | 4.0 |
| p6 flicker period min (int, 1/30 s ticks) | `+0x1c4` | 5 | 3 |
| p7 flicker period max (int) | `+0x1e0` | 10 | 9 |
| p8 flicker floor (float) | `+0x1fc` | 0.8 | 0.6 |

The update `0x140c0ba70` reads p6/p7/p8 from `this+0x74/+0x78/+0x7c`, picks each period with a
xorshift over `[p6, p7]`, and multiplies a triangle wave between 1 and p8 onto both colours; the
light is handed to the renderer (adapter `vt+0xa0`) only while the radius curve is `> 0`
(**verified in binary**). The xorshift state is seeded with a constant in the ctor, so stones
spawned on the same frame flicker in step (**verified in binary** for the seed, the lock-step is
**inferred**). The candle numbers are a starting point, not measured (**inferred**).

ResourceSet: the stone's action vector does not list 11001. Dispatch does not read it (above), and
shipped sets list ids their tree never uses (833 lists 105, 28, 15001; 181 lists 1, 59, 20002),
so it reads as an authoring summary (**inferred**). What, if anything, loads from that vector was
not traced. If a run shows no light, the first same-size fallback is to overwrite the unused
`15001` in that vector with `11001` (833: dword at file `0x1af8`).

## 3. The glow flicker: make the billboard colour a looping curve

Action-59 p8 is the billboard's RGBA curve (833: object `0x0586`, type 19 = vec4 linear,
`LoopCategoryNone`, one key `t=0`, stone colour) (**verified in data**). Type 20 is the same class
with `LoopCategoryCyclic`: the two deserializers `0x140f9baf0` (19) and `0x140f9b630` (20) have
identical bodies and differ only in the vtable they store, so the file layout is the same:
`i32 type; i32 n; n x (FXTick, FXColorRGBA)` (**verified in binary**).

The Billboard compile `0x141004620` reads p8 with `vt+0x30` and compiles each channel with
`vt+0x48(channel)`. For type 20 that is `0x140fb8ae0`: with 2+ keys it emits mode `0x10`
(`FUN_140fc73f0(0x10, ...)`); type 19's `0x140fb8b60` emits mode `4`; with one key both emit a
constant (`0x18`) or zero (`0x1c`). Table `0x14127d290` sends mode 4 to `0x140f56cd0` (linear,
clamped) and mode `0x10` to `0x140f57880` (**verified in binary**, table read); that
`0x140f57880` wraps time with `fmodf(t, t_last)` is from `docs/ffx-actions.md` 7.2 and was not
re-read here. So a looping colour **does** survive the Billboard compile, and it needs
at least two keys to loop at all.

Change for p8: keep the header's class u16 and version, write type `20`, and five keys whose last
repeats the first (colour = the stock key's four channels times the factor):

| key | time (s) | factor |
| --- | --- | --- |
| 0 | 0.0 | 1.00 |
| 1 | 0.1333 (4/30) | 0.80 |
| 2 | 0.2 (6/30) | 0.95 |
| 3 | 0.3333 (10/30) | 0.75 |
| 4 | 0.4667 (14/30) | 1.00 |

Each key is one FXTick object (14 bytes) and one FXColorRGBA object (26 bytes), copied from the
stock key's own headers, so the object goes from `0x3a` to `0xda` bytes (`+0xa0`). Its enclosing
lengths grow by the same amount: `0x047a` ParamList, `0x0468` Param 38 (action 59), then the same
chain as section 2 from `0x03de` up to `0x019e` (**verified in data**). Scaling all four channels
keeps the hue; whether alpha is opacity or intensity for this blend is **inferred**. Whether the
curve clock is the particle's age or the effect's is **inferred**; it loops either way.

A random range (types 81/82/84) does not flicker: the random factor is drawn once per particle
(`docs/ffx-actions.md` 7.2).

## 4. The change in `stone_effect.rs`

Offline dry run on 833, in memory, both edits: splice at `0x0c30` first, then `0x0586` (edit the
later offset first so the earlier one does not move), fix the enclosing lengths, and
`scripts/ds2-ffx.py`'s `Tree` re-parses the result end to end; the file goes from `0x1b17` to
`0x124b` bytes and the ResourceSet still parses at the tail (**verified in data**).

The code needs, beside `strip_sparkles`:

1. An object walker: the `{u16 class; i32 version; i32 length}` header, the Effect body skip
   (`0x16` bytes), `DLVector` as `{u16 class; i32 count; i32 x count}`, and "ancestors of offset
   X" = every object whose `[at, at + length)` strictly contains X.
2. `splice(ffx, at, old_len, new_bytes)`: replace, then add `new_len - old_len` to each ancestor's
   length. Refuse on a class-table mismatch between donor and stone.
3. Read member 181 from the same bundle (`member_from_bundle(dcx, 181)`), find its one
   `Param type 37` naming `2101` whose args hold `Param type 38` naming `11001`, patch the
   values in section 2 into that copy, and splice it over the 2032 object.
4. Find the glow's p8: the `Param type 38` naming `59` inside the glow's args, its ParamList's
   Param at index 8; require type 19 with one key; build the type-20 object of section 3 from that
   key's colour; splice it in.
5. Set the root id to the registered id (unchanged from today), and change the host tests: the
   "at most 8 bytes changed" and "same length" asserts no longer hold; assert instead that the
   result re-walks to its end, holds exactly one 11001, no 2032, and a type-20 p8 with five keys.

## 5. What a run has to show

Spawn the derived 25833 beside a stock 833 in a dark area, then:

- The light exists and is registered: `0x140c0b6e0` (point-light ctor) runs once per derived
  stone, `0x140c0ba70` runs every frame, and `this+0x74/+0x78/+0x7c` read 3 / 9 / 0.6. The
  adapter `vt+0xa0` is called with `1` once the radius is above 0.
- It flickers: the colour the update writes into the light object that `this+0x30` points
  to (its two colour float4s) changes between frames, between 1.0 and 0.6 of the base.
- The glow loops: `0x140fb8ae0` runs for the glow's p8 and writes mode `0x10` for all four
  channels, and the Billboard compile does not panic with `!!invalid parameter size!!`.
- The engine accepted the file: the spawn is counted in the `sys+0x2b8` spawn map
  (`docs/DS2-SFX-SPAWNED-IDS.md`).
- By eye (the user): the floor and nearby walls pick up the stone's colour, and the glow pulses.
