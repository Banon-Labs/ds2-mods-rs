# Face-only character creator (DS2 SOTFS, build 9527516)

Goal: reuse the game's character creator in the world to change only the player's face.

Everything here was read from `darksoulsii-deobf.bin` and the Ghidra project, with no game running.
**[static]** means read straight from the code. **[inferred]** means deduced from that code but not
proven. None of the hook sites below is Arxan-redirected (`scripts/ds2-arxan-chain.py` on
0x1400de610, 0x14004c910, 0x14004cc90 and 0x140340590 finds each function's own prologue).

## The objects

- `openCharaMakerWindow` 0x1401986c0 leads to the frontend body 0x1404fffd0, which stores
  `FeOperatorTestCharaMaking+0x28 = 0`. The operator's update 0x1400e3060 reads `+0x28` only as zero
  or not zero: zero pushes the full creator (0x1400e35e0), anything else pushes name entry
  (0x1400e3620). No third mode exists. [static]
- The `FeSceneTestCharaMaking` constructor 0x1400e27f0 builds an `FeCharaMakingWarehouse`
  (constructor 0x1400e3340, vtable 0x1410bb520) at scene+0xb8. The warehouse holds:
  - the group set (0x1400e3180) at warehouse+0x140: `FeGroupCharaMakingTop` at +0x178 (the group the
    full creator pushes, constructor 0x1400eaf50, vtable 0x1410bb138); `FeGroupCreateNameEntry` at
    +0x488 (the group name entry pushes); and the detail tab, large-slot, palette and other groups.
    [static]
  - the class/gift selection at warehouse+0x3b28 (constructor 0x1400dacc0), with the class row id
    at +0x108 and the gift row id at +0x10c. [static: these are the fields 0x1400de610 reads]
  - the "chara data" block at warehouse+0x3e20 (constructor 0x14004c780): eight u32 character
    handles at +0x00..+0x1f (slot 0 is the doll you edit, slots 2-7 are extra previews, slot 1 is not
    spawned), followed at +0x20 by 97 u32 values (0x184 long). 0x14004ce50 fills those from the
    player's FaceGen parameters when the creator opens. [static]
- `FeCharaMakingEditParam` (vtable 0x1410bb0a8) is one slider or option. +0x10 points at the chara
  data block, +0x18 is the parameter id and +0x1c the slot. 0x14004d880 writes a face block into
  preview slots (slot -2 = all eight, -1 = slots 1-7) through each one's face part import. [static]

## 1. What "Finish creation" commits

There is no separate confirm step. `FeGroupCharaMakingTop` has an enter handler, 0x1400ebea0 (vtable
+0xc8), and a leave handler, 0x1400ec020 (vtable +0xe8). The leave handler is the commit. The
top-level key guide (charamaking.fmg 10030100) offers "Finish creation" and has no Back, so leaving
always commits. [static for the handlers; inferred that no path pops Top without committing]

**Enter, 0x14004c910** (called from 0x1400ebea0 with warehouse+0x3e20): [static]
- Sets FaceGenManager (`[GameManagerImp+0x80]`) requested mode `+0x104 = 2`. The FaceGen state machine
  0x1404f2d80/0x1404f2d10 then moves `+0x100` toward it and flushes caches (0x1404f27e0). This is the
  `+0x100 == 2` early return in the player-ready check 0x14037f5b0.
- `0x140415c60(EnemyGeneratorManager, mapId("m10_02_00_00"), 0x10, MapMan+0x160)`. What this does is
  unknown; commit undoes it with 0x140415f10. [inferred: a creator-scene setup]
- On the player (`PlayerCtrl->vtbl[0x1403126b0]` = the player's ChrAsm): it sets the model's
  (`ChrAsm->vtbl+0x80`) `+0xe89 = 0` and snapshots the FaceGen parameters into chara data +0x20
  (0x14004ce50). Then it **strips the player's visible equipment**: on `ChrAsm->vtbl+0x60` it calls
  0x1403476d0(..., -1, 1) for armour kinds 0-5 and 0x140347370(..., 6..9, -1).
- Spawns the doll in slot 0 (CharacterManager 0x140356bf0 type 0) and previews in slots 2-7 (type 1).

**Leave, 0x1400ec020**, in order: [static]
1. **0x14004cc90(warehouse+0x3e20) commits the face.**
   - Zeroes a stack face block (length 0xa2), takes the slot-0 doll's face part (`chr->vtbl+0x120`
     ChrAsm, `+0x80` model, `0x140339cd0(model)`) and exports it into the block (part `vtbl+0x180` =
     0x14033f8f0).
   - Deletes all eight preview characters (CharacterManager 0x140359890) and clears their handles.
   - Sets bit 7 of block+0x95 (the "custom face" flag; SetFaceData then sets model+0x40 = -1, meaning
     "no preset").
   - If the player's model has a FaceGen face part, calls the player model's `SetFaceData`
     (`ChrAsmModel vtbl+0x60` = 0x14033a5b0) with the block and sets model `+0xe89 = 1`.
   - Sets FaceGenManager `+0x104 = 1` (back to in-game FaceGen mode) and calls
     0x140415f10(EnemyGeneratorManager).
2. **0x1400de610(warehouse+0x3b28) commits class and gift.** It is the rest of the character:
   - Class row = `0x140358e80(CharacterManager, +0x108)` (a param lookup through CharacterManager+0x730;
     [inferred] PlayerStatusParam). Stores the id at CharacterManager+0x840. Then `addSoul(row+8)`,
     `assignAttributes(row+6, +0xc, +0xe, +0x10, +0x12, +0x16, +0x1a, +0x1e)` (the class's starting
     stats and level), recalculation 0x14038bc70, and 0x140202f10/0x140203e10 with row+0x124/+0x125
     (purpose unknown).
   - Gift row = `0x140358e80(..., +0x10c)`. Unequips all 0x29 equipment slots (0x1401ac510 on
     `GameDataManager+0x10`), then equips the row's item list (0x140155b90 builds it, 0x1401ac3a0
     equips each item).
   - 0x140346d90(`ChrAsm->vtbl+0x60`) re-applies the **class row's** equipment list to the visible
     equipment and to GameDataManager, then 0x1401ac4f0 and 0x14037f9d0(player), which sets
     `[ChrAsm->vtbl+0x70]+0x3f = 1`.
3. A tail call to 0x1400e3bf0(warehouse+0x12b8) runs only a return-to-title check when
   warehouse+0x12b8+0x1e6 is set (UI only).

**Not written by the full creator's commit:** the name at PlayerGameData+0x24 (that is name entry's
job; neither 0x14004cc90 nor 0x1400de610 touches PlayerGameData), and the player's sex flag. Sex lives
on `ChrAsm->vtbl+0x70` `+0x2c`. SetFaceData does not write it. Only the face part's import (below)
does. [static]

### The face block (length 0xa2)

It lives at `ChrAsmModel+0xef0`. `SetFaceData` 0x14033a5b0 writes it; the getter
`ChrAsmModel vtbl+0x68` = 0x140339d90 refreshes +0x92/+0x95/+0x99 from the live objects, then returns
it. The live face is a separate object, the FaceGen face part `ChrAsmModel+0xd08` (only used in mode
0x08; reached through 0x140339cd0), which holds a FaceGenParameter at `[part+0x18]+0x20`. The layout
below comes from the part's export 0x14033f8f0 and import 0x140340590: [static for the reads and
writes; field meanings inferred]

| Offset | Source on export | Note |
| --- | --- | --- |
| +0x00..+0x0f | floats cast to u8, from FaceGenParameter +0x70/+0x74/+0x78, +0xa0/+0xa4/+0xa8, +0x80/+0x84/+0x88, +0x90/+0x94/+0x98, +0xb0..+0xbc | colours (hair, eyes and so on) [inferred] |
| +0x10..+0x91 | from 0x1404f4a70 (import 0x1404f5680) | FaceGen shape and texture coefficients |
| +0x92 | `ChrAsm->vtbl+0x70` `+0x2c` (flipped when 0x14033a230(model) is true) | sex |
| +0x93 | `vtbl+0x70` `+0x2d` | body option ("Build" or "Physique") [inferred] |
| +0x94 | low 7 bits from 0x1403415c0(part, param+0x40, sex) | hair or preset index [inferred] |
| +0x95 | bit 7 = custom-face flag (model+0xf85); bits 0-3 = param+0x48; bits 4-6 = `vtbl+0x70` `+0x20` | SetFaceData writes bits 4-6 back to `vtbl+0x70` `+0x20` and marks it dirty |
| +0x96, +0x97, +0x98 | param +0x44, +0x4c, +0x50 | |
| +0x99 | `ChrAsm->vtbl+0x50` `+8` | SetFaceData passes it to 0x140345d90; a body or voice setting [inferred] |
| +0x9a..+0xa1 | four shorts = param +0x54..+0x60 x scale | |

How the block is persisted [static except where marked]:
- Each save-slot header entry (stride 0x1f0, ten entries) holds the block at entry+0x00. 0x14019c190
  writes it from the live part (`vtbl+0x180`).
- `SaveDataPlayer` (vtable 0x1410da260) reads its version-0x6e section through 0x1402e5cd0 and
  0x14019eb10, which copies section+0xc8 (one face block) to `[GameDataManager+0xd0]+0xd0`.
- Its writer 0x1402e5dc0 builds the version-0x6f section from PlayerCtrl through an Arxan-wrapped
  builder (0x141b3d2e4) that was not read. [inferred: it exports the face from the live part, as every
  other writer does]

## 2. Is there an appearance-only mode or path?

- **In the creator: no.** The `+0x28` byte has two meanings: 0 is the full creator, anything else is
  name entry. The full creator's top tabs (charamaking.fmg 10010100-10010130) are "Class & gift",
  "Body" (Gender / Build / Physique, 12010100-12010120), "Face" and "Advanced settings". All of them
  sit under one `FeGroupCharaMakingTop`, and its single leave handler commits face, class and gift
  together. A sub-page cannot be pushed on its own and still be useful: the doll is spawned in Top's
  enter and the commit runs in Top's leave. [static]
- **In vanilla, in the world: none found.** The talk-script cases around 130451/130452
  (0x1404631b9..0x1404631f6) call 0x140198720, 0x14019ace0, 0x1401986c0, 0x140198f70, 0x140198920 and
  0x140199540. Only the two known ones reach the creator. Apart from the creator, the only code that
  calls `ChrAsmModel` SetFaceData on a character outside spawn and load is the demo-character copy
  in 0x1403552c0 ("DemoEquipIdCopied"), which copies the **player's** face onto a cutscene double.
  [static; scan limited to those cases and to every place that copies a whole face block]

## 3. How a new face reaches the live model

- `SetFaceData` (0x14033a5b0) copies the block into +0xef0, sets +0x40 = -1 when +0x95 bit 7 is set,
  and pushes +0x95 bits 4-6 and +0x99 into their equipment objects. **It does not touch the live face
  part.** [static]
- The face part is built only once: 0x14033d3d0 creates it from +0xef0 when `+0xd08 == 0`.
  0x14033d120 and the readiness check 0x140152370 call it. While a part exists, a new +0xef0 is
  ignored until that part is destroyed (0x14033bf70) and rebuilt. [static]
- **The face part's own import, `ChrAsmModelFace vtbl+0x188` = 0x140340590, is the call that changes
  a live face.** It writes the FaceGenParameter fields and their dirty flags (+0x22, +0x28), the sex
  flag (`vtbl+0x70` `+0x2c`, marked dirty), calls the model's SetFaceData itself, and then either
  rebuilds right away (part `vtbl+0x1a8` and `[part+0x18]->vtbl+0x20(0)`) or, if the model is still
  loading, sets a deferred flag (`[model+0x30]+0x18 = 1`). This is how the creator updates the doll and
  previews (0x14004d880). [static]
- So the vanilla commit 0x14004cc90 calls only SetFaceData on the player. Whether the player's face
  changes on screen then depends on whether something rebuilds the part afterwards: the FaceGen
  manager switching from mode 2 back to 1 (0x1404f27e0 flushes its caches) or `+0xe89` going 0->1
  (0x14033b5a0 gates the parts' per-frame update on it). Neither shows a rebuild from +0xef0.
  [inferred: risky, needs one runtime check]
- Relation to the memories: 0x140339cd0 hands out the part only in mode 0x08, and the player-ready
  check 0x14037f5b0 returns early while FaceGenManager `+0x100 == 2`. That is the creator mode set by
  0x14004c910, so the world-load wait is skipped while the creator is open. [static]

## 4. Recommended design for a face-only change

Use the full creator UI unchanged and change only what its commit does. All sites are in plain
(non-Arxan) code.

1. **Open:** call `openCharaMakerWindow` 0x1401986c0 on the game thread, as
   `scripts/frida/open-chara-maker.js` already does. Set a "face-only" flag.
2. **Snapshot at enter:** hook the entry of 0x14004c910 (before it strips anything) and save:
   - the player's current face block: export from the player's face part (`vtbl+0x180` 0x14033f8f0
     into a 0xa2 buffer);
   - the player's visible equipment: `ChrAsm->vtbl+0x60` slots 0..0x33, read with
     0x140346940(equip, out, slot). This is the same 0x14-long record 0x140346d90 writes back with
     0x1403463d0.
   - Optional: after 0x14004c910 returns, import the player's block into the doll (slot 0 part
     `vtbl+0x188`) so editing starts from the current face. Whether the doll already starts from it
     is unproven.
3. **Skip class and gift:** detour 0x1400de610 to return at once while the flag is set. That skips
   addSoul, assignAttributes, the 0x29-slot unequip, the gift equip and the class-row re-equip
   (0x140346d90). Inventory, stats, souls and level stay untouched. [static for what it skips]
4. **Commit only the face:** let 0x14004cc90 run (it tears down the previews and restores the FaceGen
   mode and the enemy generator). Hook `SetFaceData` 0x14033a5b0 only when its return address is
   0x14004cdec (the call in 0x14004cc90). In the block at rdx, put back from the snapshot the bytes
   that are not face: +0x92 (sex), +0x93, +0x95 bits 4-6 and +0x99. Keep +0x95 bit 7 set. After
   0x14004cc90 returns, call the player's face part import `vtbl+0x188` (0x140340590) with that
   patched block, so the live face is rebuilt rather than only stored in +0xef0.
5. **Restore the visible equipment** the enter stripped: for each saved slot, call 0x1403463d0(equip,
   slot, &record), then 0x14037f9d0(PlayerCtrl). This copies what the skipped 0x1400de610 tail would
   have done, but with the player's own records instead of the class row's.
6. **Name:** the full creator does not write the name. If its finish flow also pushes name entry, the
   user must enter the same name again. [not checked]

**Still needs runtime proof (one run, measured without looking):**
- that the player's face part changes after step 4. Watch `[part+0x18]+0x20` (FaceGenParameter
  +0x22/+0x28 flags) and the export of the part before and after, against the doll's block;
- that the visible equipment comes back after step 5 (compare the 0x34 records), and that real
  equipment and stats in PlayerGameData stay unchanged (the existing PlayerGameData diff in
  open-chara-maker.js);
- that the face persists through a save and reload. The SaveDataPlayer writer is behind Arxan
  (0x141b3d2e4), so read the save-slot header entry +0x00 after the next save;
- whether "Finish creation" also pushes name entry.

## 5. Why "Finish creation" demands a class and gift first

**The gate is 0x1400de340(selection, outJob)**, where selection = warehouse+0x3b28 (its first qword
points back at the warehouse). It builds the job that "Finish creation" runs. It is reached from both
routes: Top's handler 0x1400eb910 (call at 0x1400eba9e) and the "Finalize creation" entry of the
Class & gift list (data reference at 0x1400dd5c3). [static]

It refuses (it shows charamaking.fmg 80090130, "Select class and gift", and builds no confirm job) when
any of these holds: [static]
- u32 selection+0x108 (class) == 0;
- u32 selection+0x10c (gift) == 0;
- byte selection+0x131 != 0. 0x1400e06c0 and 0x1400e1140 set it to 1 when the class or gift list
  opens, and 0x1400e1a30 clears it when that list closes.

Otherwise it builds the "Is this your true self?" (id 212) confirm, whose yes path pops Top and so
runs the commit in section 1. Before both checks, 0x140500790 returning true makes it return no job
at all.

**Where the selection is set.** The constructor 0x1400dacc0 zeroes +0x108..+0x12f and the u16 at
+0x130. "Reset" (0x1400e1010) puts class, gift and indices 2, 7 and 8 back to 0. Nothing at enter
(0x1400ebea0 / 0x14004c910) fills them. The setter the lists use is **0x1400e2010(selection, index,
value)**: it stores `selection+0x108+index*4 = value`, then for index 0 (class) calls 0x1400e1ba0
and for index 1 (gift) calls 0x1400e1da0. Those update the tab label text and the stats panel
(warehouse+0x12b8), and while byte selection+0x130 == 0 they dress the doll in the class's gear
through 0x14004df20. [static]

Valid ids: [static, from the switches in 0x1400e1ba0 and 0x1400e1da0]
- class: 0x14 Warrior, 0x1e Knight, 0x32 Bandit, 0x46 Cleric, 0x50 Sorcerer, 0x5a Explorer,
  0x64 Swordsman, 0x6e Deprived;
- gift: 500 (0x1f4) Nothing, 0x1fe Life Ring, 0x208 Human Effigy, 0x212 Healing Wares,
  0x21c Homeward Bone, 0x226 Seed of a Tree of Giants, 0x230 Bonfire Ascetic, 0x23a Petrified
  Something.

0x1400de610 is skipped while the script is armed, so these ids only open the gate. They grant
nothing.

**Smallest change to `scripts/frida/appearance-only-creator.js`:** while armed, in the onLeave of
Top's enter 0x1400ebea0 (rcx at entry = the Top group; warehouse = `[top+0xc0]`), or of 0x14004c910
(rcx at entry = warehouse+0x3e20, so warehouse = rcx - 0x3e20):

```js
const sel = warehouse.add(0x3b28);
const setSelection = new NativeFunction(at(0xe2010), 'void', ['pointer', 'int', 'int']);
setSelection(sel, 0, 0x6e);   // class: Deprived (any id from the list above)
setSelection(sel, 1, 500);    // gift: Nothing
sel.add(0x131).writeU8(0);    // no class/gift list left open
```

This is the game's own setter, so the tab labels and stats panel follow. The side effect: the doll
wears the chosen class's gear while you edit (Deprived = almost none). If that is unwanted, write the
fields raw instead (`sel.add(0x108).writeU32(0x6e); sel.add(0x10c).writeU32(500);`). That passes the
gate the same way, but the Class & gift tab shows nothing selected. [inferred: the gate reads only these
fields]

Not used: CharacterManager+0x840. Only 0x1400de610 writes it, at character creation, and it is not
loaded from the save on its own [not checked]. After a load it may be 0, which would leave the gate
shut, so a fixed valid id is the safer choice.
