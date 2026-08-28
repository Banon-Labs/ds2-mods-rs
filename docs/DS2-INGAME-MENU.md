# The pause menu, its six tabs, and where a new row would come from

Everything in the first four sections was read statically from `darksoulsii-deobf.bin` (SOTFS build
9527516) with `scripts/ds2-rtti.py`, `scripts/ds2-xrefs.py`, `scripts/ds2-disasm.py`,
`scripts/ds2-arxan-chain.py` and the Ghidra MCP daemon. **No game was launched to establish any of
it.** The last two sections are the run that followed, and one claim below did not survive it: the
heading "the SELECTABLE row count is code-driven" says "selectable" because a run showed that the
drawn count is a different number from a different place.

Addresses here are VAs, the form the disassembly prints. Subtract `0x140000000` for the RVA that
`ds2-rva` records.

## The shape

```
FeGroupInGameTopSelect          six FeGroupInGameGroupSelect members, one per tab
  +- FeGroupInGameGroupSelect   a FexGridControl whose rows come from an item vector
       +- item vector           DLFixedVector<(u32 action, u32 gate)>, capacity 5
```

`FeGroupInGameTopSelect::ctor` at `0x1400a41b0` constructs all six, and for each one calls a
dedicated *item builder* that fills a stack descriptor, then `FUN_1400a40e0` to copy the descriptor
into the group. The six builders, and what their entries open:

| builder | entries `(action, gate)` | destination |
| --- | --- | --- |
| `0x1400a4990` | `(0,0)` | `FeGroupInGameMenuEquipTop` |
| `0x1400a4db0` | `(1,0)` | `FeGroupInGameMenuInventory2` |
| `0x1400a5620` | `(2,0) (3,0)` | `FeGroupInGameMenuStatusStatus`, `...StatusInfo` |
| `0x1400a4fc0` | `(4,1) (5,2) (6,3)` | `FeGroupIngameMessageWrite`, `...ReadHistory`, `...WriteHistory` |
| **`0x1400a5900`** | `(7,0) (8,0) (9,4)` | `...SystemSettingGame`, `...SystemSettingScreen`, **`FeGroupInGameReturnTitleCheck`** |
| `0x1400a5330` | `(0xb,0) (0xc,0)` | `...SystemSettingKeyboard`, `...SystemSettingGraphic` |

Every destination class was identified by walking the constructor's vtable write back to
`[vtable-8]`, reading the `RTTICompleteObjectLocator`, and reading the type descriptor's name. None
of them is inferred from a function name.

**`0x1400a5900` is the tab with the quit item.** Action `9` is the entry the player presses to
leave a game, and it resolves to `FeGroupInGameReturnTitleCheck` -- the dialog that offers to save
on the way to the title screen.

### The vector

The descriptor is a `DLKR::DLFixedVector`:

* elements start at `descriptor + (-(int)descriptor & 3)`, stride 8;
* the count is at `descriptor + 0x30`;
* capacity is **5**, spelled by the builders as `if (5 < newCount) panic("out of memory.")` against
  `DLFixedVector.inl:0x24c`, and independently by the copy at `0x1400a3ef0`, which panics unless the
  source count is `< 6`.

In the constructed group the same struct lands at `+0xf8` (elements) and `+0x128` (count), which is
`0xf8 + 0x30` -- corroborating the offset from the other side.

An entry is two `u32`s. The confirm path takes the action from `[entry]` and the gate from
`[entry+4]` (`lea rcx,[rax+4]` at `0x1400a4cce`).

## The dispatch

`FUN_1400a6090` at `0x1400a6090`, one `switch (action)`, reached from the tab's confirm handler at
`0x1400a6b10`. That handler reads the entry under the cursor, applies its gate, and passes either
the action or `-1`; `-1` selects a different sound id and falls out through the switch's `default`,
which is how a gated row refuses.

Two shapes of case:

* **a direct `FeInGameMenuWarehouse` member** -- actions 0, 2, 4, 5, 6, 9. Action 9 is
  `warehouse + 0x6f10`, and the warehouse's own constructor (`0x1400991e0`) builds
  `FeGroupInGameReturnTitleCheck` (ctor `0x14006e390`, vtable `0x1410b1768`) at member `+0xde2` --
  `0xde2 * 8 == 0x6f10`.
* **a `FexDynamicGroupExecJob` carrying a kind** -- actions 3, 7, 8, 0xb, 0xc, 0xd map to kinds 0,
  2, 3, 4, 5, 6. The job's exec (`0x14002d870`) reads the kind exactly once, to call the factory at
  `0x1400a67c0`.

Action `0xa` has no case at all.

### Action `0xd` is shipped, wired, and listed by nothing

The factory's branch for kind 6 shares a `case` label with kind 4:

```c
case 4:
case 6:
  lVar2 = heapAllocator(0xc68, 8, allocator);
  if (lVar2 != 0) FUN_1400803b0(lVar2);      // FeGroupInGameSystemSettingKeyboard
  break;
```

Same allocation, same constructor. The kind is not passed on to the group -- the created object is
initialised with `(*group->vtable[1])(group, allocator, 0)` and nothing else. So **executing action
`0xd` is byte-for-byte what executing the shipped Key Bindings row already does.** No tab lists it.

### The gates

Gate index `0` means no gate. Nonzero indices go through `0x1400a4e50`, which returns `1` for
*refused*: the confirm path turns that into `-1` and the availability pass greys the row.

The quit item carries gate `4`, which resolves the session object at `GameManagerImp + 0x22f0`
through `FUN_140513270` and asks `FUN_14025f690` about it. Neither callee is named in the project,
so **what that gate actually forbids is not recorded here** -- only that it is the gate the shipped
quit row uses.

## The SELECTABLE row count is code-driven

**This heading originally read "the row count is code-driven", and that was wrong.** Everything in
this section is accurate about what these functions do; the error was concluding that it was
sufficient to put a row on screen. It is not -- see "Measured, one run, 2026-08-28". The word
"visible" in the code comment below is preserved as it was written, so the mistake is legible.

The per-tab init
`FUN_1400a4d20` at `0x1400a4d20` is, in full:

```c
FUN_140026790(...); (*local_58[0])(local_58, 0x65, 1, 0);   // play a sequence
FexGridControl::FUN_1400216d0(tab, allocator, 0);
FUN_140021b30(tab, tab->itemCount);                          // <-- visible cells FROM the count
FeGroupInGameGroupSelect::FUN_1400a77c0(tab);                // <-- gate each row
```

`FUN_140021b30` is the same call `FeGroupInGameTopSelect`'s own init makes with a literal `6` for
its six tabs. The availability pass then walks `0..cellCount`, reads entry `i` with a bounds check
that falls back to a static `-1`, and greys what the gate refuses.

So the count the cursor is bounded by is the item vector's count. Appending an entry asks for a row.
What it does NOT do is create anything to draw: `FUN_140021b30` writes `+0xc8` and drives the
scrollbar, and the drawable cells were already fixed by the `0x1400216d0` bind on the line above.

## What could not be read out of the executable

**Where a row's caption comes from.** Nothing in the image maps an action id to a message id.
`FUN_1400a77c0` only sets the greyed flag; the confirm path only dispatches. The captions the
executable *does* set -- `FeGroupInGameTopSelect`'s init at `0x1400a6da0`, message ids `0x200f08`
and `0x200f09` in category 7 -- are addressed to layout elements resolved by hard-coded name hash
(`0x1eab9b`, `0x1eab9c`, ..., from a five-entry table at `0x1400a6310`).

That points at the row labels being authored in the frontend layout inside `GameDataEbl.bdt`, which
this repo does not open. A fourth row may therefore appear with no caption, or with one the layout
happened to author. **Capacity 5 in the code is suggestive that five rows were authored. It is not
evidence.**

> **Answered by the run below, and the answer made the question moot.** The layout authored three
> cells for this tab, so the fourth item never became a drawable row at all and no caption was ever
> asked for. See "Measured, one run, 2026-08-28".

## Nothing here is an Arxan redirect

`scripts/ds2-arxan-chain.py` terminates at hop 0 with a clean prologue on every function this
document names as a hook candidate:

| site | prologue |
| --- | --- |
| `0x1400a5900` item builder | `40 53 48 83 ec 50` |
| `0x1400a6090` dispatch | `48 89 5c 24 18` |
| `0x1400a41b0` top-select ctor | `48 89 5c 24 10` |
| `0x1400a4d20` per-tab init | `40 53 48 81 ec b0 00 00 00` |
| `0x1400a6b10` confirm handler | `40 53 48 83 ec 20` |
| `0x1400a67c0` dynamic-group factory | `48 89 5c 24 08` |

The usual caveat still governs the addresses: the deobfuscated image is not the byte stream that
runs, so data (vtables, globals) is trustworthy and a function body must be checked before it is
detoured. That check is the table above.

## The experiment (`crates/ds2-menu-row`) -- run, see the section after it

Off by default. `[menu_row] enabled = true`, or `scripts/ds2-run.py --menu-row`, turns it on.

It detours the quit tab's item builder (`0x1400a5900` -- one caller in the whole image,
`0x1400a4382`, inside the top-select constructor, so it reaches this tab and nothing else), lets
the original run, and appends `(0xd, 0)` to what it produced.

Action `0xd` is the payload precisely because of the `case 4: case 6:` above: it adds a row without
adding a code path.

**It refuses more often than it writes.** Before touching the vector the detour re-reads what the
original built and requires exactly `(7,0) (8,0) (9,4)`. An RVA is a number; on a build whose tabs
are ordered differently this would append to some other tab and produce a screenshot that looks
exactly like a result while being about nothing. A refusal is logged with the entries it actually
saw. The install refuses too, if the six prologue bytes at the site are not the ones recorded.

Read the log before reading the screen:

```
ds2-menu-row: hooked rva=0x000a5900 va=0x... payload=(0xd,0) open the pause menu's last tab ...
ds2-menu-row: appended action=0xd gate=0 was=[(0x7,0) (0x8,0) (0x9,4)] count=3->4 fire=1 appends=1
```

`fire` and `appends` are separate counters on purpose: "no fourth row appeared" has two very
different causes -- the append was refused, or the tab was never built because the pause menu was
never opened -- and one counter cannot tell them apart.

## Measured, one run, 2026-08-28

`--menu-row`, one character loaded, the quit tab opened by hand.

```
ds2-menu-row: hooked rva=0x000a5900 va=0x00000001400a5900 payload=(0xd,0)
ds2-menu-row: appended action=0xd gate=0 was=[(0x7,0) (0x8,0) (0x9,4)] count=3->4 fire=1 appends=1
```

**The item vector took the fourth entry.** `was=` is the integrity check passing: the tab really does
hold `(7,0) (8,0) (9,4)` in the live process, so every static claim above about which tab this is
holds at runtime and not only in the decompiler.

**A fourth item exists and is reachable. Nothing is drawn for it.** Reported from the screen: the
cursor moves onto a fourth entry and it responds; there is nothing visible there.

### Why, and it is not the caption

The open question above assumed the worst case was a blank caption. It was worse and simpler than
that: **the row does not exist as a drawable cell at all.** `FexGridControl`'s layout bind,
`FrontendEx::FexGridControl::FUN_1400216d0` at `0x1400216d0`, does not read an extent from anywhere
-- it DISCOVERS one by probing the layout and stopping at the first hole:

```c
for (row = 0; row < 15; row++)                 // 0xe < row -> done
  for (col = 0; col < 32; col++) {             // col < 0x20
      element = (*namer->vtable[0x10])(col, row);
      if (element == 0) break;                 // <-- this row ends here
      cell = FUN_14010a060(...);               // a drawable cell object
      this->_0xd4 = max(this->_0xd4, col + 1); // extent := what the layout HAS
      this->_0xd8 = max(this->_0xd8, row + 1);
  }
```

So there are two counts, they come from different places, and only one of them moved:

| field | meaning | set by | after the append |
| --- | --- | --- | --- |
| `+0xc8` | logical item count -- what the cursor may reach | `FUN_140021b30(tab, itemCount)` | **4** |
| `+0xd4` | grid extent -- how many drawable cells the layout provided | `0x1400216d0`, by probing | **unchanged** |

`FUN_140021b30` writes `+0xc8` and drives the scrollbar. It never touches `+0xd4`. That is the whole
gap between "interactable" and "invisible", and `FUN_140022160` closes the argument: resolving the
element for a cell whose column equals `+0xd4` takes the `vtable+0x48` one-past-the-end naming
branch rather than the `vtable+0x10` ordinary-cell branch.

### What this settles

* **The row count is NOT code-driven after all**, or rather: the *selectable* count is and the
  *drawn* count is not. `docs`'s earlier reading of `FUN_1400a4d20` was right about what it does and
  wrong about what that is sufficient for.
* **The engine is not the constraint.** The bind will find up to 32 x 15 cells and the item vector
  caps at 5. What is missing is authored data.
* **A visible new row needs the layout**, inside `GameDataEbl.bdt` -- and specifically needs cell 3
  of that tab's grid to exist, because the probe stops at the first hole: authoring cell 4 without
  cell 3 would find neither.
* **Captions were never reached as a question.** Whether the row label is authored in the layout or
  set from a message id is still unknown, because no row got far enough to want one.

### What a mod can still do without touching the archive

Repoint an EXISTING entry. The item vector is `(action, gate)` and both halves are ours to write, so
swapping the action on one of the three cells the layout already draws gives a working new
destination -- under that cell's existing caption, which would then be a lie. That is a trade this
document records rather than recommends.

## The layout, extracted

`menu:/42.febnd.dcx` is the pause menu's layout archive, and `scripts/ds2-ebl.py` reads it with
nothing but the standard library and the key the game ships.

```
$ python3 scripts/ds2-ebl.py info
salt          FRPG2_STEAM_GAME_DATA_EBL_SALT64
buckets       1931
entries       11699
AES-encrypted 1960
bucket check  0 of 11699 entries violate hash % 1931 == bucket

$ python3 scripts/ds2-ebl.py extract /menu/42.febnd.dcx --out /tmp/menu42
/menu/42.febnd.dcx  hash=0x27022a7f size=412019 offset=0xd989b5
  DCX -> 1108656 bytes
  BND4 -> 3 file(s)
    l42_01_OptionSetting.flo  (50240 bytes)
    option.tpf                (1048747 bytes)
    yubiwa_test.tpf           (8368 bytes)
```

### How the id becomes a path

`FeGroupInGameTopSelect`'s constructor writes `0x2a` to `TopSelect + 0x140`
(`mov QWORD PTR [rsi+0x140],0x2a` at `0x1400a425f`). That is the id field of the
`FrontendEx::FexLayoutBndResourceProxy` embedded at `TopSelect + 0x110`, and the proxy's load
(`0x140026bc0`) hands it to the path builder at `0x14048b4e0`:

```c
pwVar1 = L"menu:/%02d.febnd.dcx";
if ((id - 0x11U & 0xfffffffd) == 0)                  // only ids 0x11 and 0x13
    pwVar1 = L"gamedata:/menu/%02d.febnd.dcx";
FUN_140003690_format_wstr(out, pwVar1, id);
```

`0x2a` is 42, so the path is `menu:/42.febnd.dcx` -- which inside the archive is
`/menu/42.febnd.dcx`. The proxy then goes to `AppResourceManager` (`0x1400264d0`), which checks a
cache (`0x140b02360`) and otherwise allocates `0x130` bytes and constructs the resource
(`0x140affe10`).

### The chain, and what proves each step

| step | how it is checked |
| --- | --- |
| RSA-decrypt the BHD with `GameDataKeyCode.pem` | `BHD5` magic lands at plaintext offset 0 |
| BHD5 record layout | every entry's `hash % 1931` equals its own bucket index -- **0 violations of 11699** |
| path -> hash, `h = h*37 + byte`, lowercased | reproduces paths read out of the exe's own string table |
| BDT read | the entry's size comes back exactly |
| DCX / DFLT | the header's uncompressed-size field matches what zlib produced |
| BND4 | 3 members, names and magics as above |

The `.pem` is an RSA **public** key. FromSoftware "signed" the header with the private key, so the
public key that ships beside it is the decryption key. Nothing had to be found or guessed.

`/menu/42.febnd.dcx` carries no AES key record, so it is not encrypted either. 1960 of the 11699
entries are; `ds2-ebl.py` refuses those rather than returning ciphertext that looks like a corrupt
file.

### What the `.flo` looks like

`l42_01_OptionSetting.flo`, 50240 bytes, no strings at all -- 107 short ASCII runs, every one of
them a coincidence inside float data, and zero UTF-16 runs. It opens with a table of `u64` section
offsets (`0xa8`, `0x19b0`, `0x19b0`, `0x3790`, `0xbfe0`, `0xc080`, `0xc388`, `0xc200`) and holds
uniform binary records.

The element ids the executable passes around are IN it. `0x5f5b9f2` occurs five times and
`0x5f5c3e5` twice, each at the same offset within an identical 16-byte record prefix
(`00 00 00 00 01 00 08 00 ff ff 01 00 00 00 00 00`), followed by a field that differs per record
and an in-file offset. So an element is a fixed-size record keyed by a numeric id, and adding one
is a binary edit rather than a string-table edit.

**The `0x1eabXX` ids are NOT in this file**, and that is the shape of the id space rather than a
problem: `FUN_1400a6310` builds a *path* of three ids -- `[0x1eaba9, cell, 0x5f5b9f2]` -- so the
`0x1ea....` components address the screen and group and the `0x5f5.....` component addresses the
leaf element inside this `.flo`.

**The `.flo` record format is not decoded.** What is established is that it is decodable: fixed
records, numeric keys, no strings, and a section table to anchor them.

## Where a runtime interception would go

Four candidate layers, cheapest blast radius last. None of these is built.

1. **Rewrite the path** -- detour `0x14048b4e0` and hand back a different string. One tiny hook,
   but it only helps if the virtual filesystem can open whatever it is pointed at, which has not
   been established.
2. **Substitute the bytes** -- detour the EBL read that turns a path hash into bytes, and serve a
   modified `.febnd.dcx` from a loose file. This is the general form: everything in the archive
   becomes redirectable, the 13 GB `.bdt` is never rewritten, and iteration is "edit a file on
   disk, relaunch". It needs the `.flo` format, because the file being served has to be a valid
   one.
3. **Patch the parsed layout** -- let the resource load, then find the element records in memory
   and add one. Trades the file format for the in-memory node structure; no obvious win over 2.
4. **Synthesise the element at bind time** -- detour `FrontendEx::FexGridControl`'s layout bind
   ([`FEX_GRID_CONTROL_LAYOUT_BIND`], `0x1400216d0`) or the namer it calls at
   `[grid+0xf0]->vtable[0x10]`, and hand it a cell-3 element it would not otherwise find. The
   narrowest possible change and it needs no file format at all -- but the element it is handed
   still has to be a real drawable with its own position, so it is "clone and move a node", which
   is question 3 wearing a smaller hook.

2 is the one worth building, and its prerequisite is the `.flo` format, not more hooking.

## The architecture, taken from `../er-mods-rs`

Elden Ring has this exact problem solved, and the solution is worth copying rather than reinventing.
`er-gfx` + `crates/er-quickload/.../profile_table_gfx_files.rs` extend the ER quit menu from two
buttons to six. Two rules come out of it, and both are load-bearing:

**Ship no game-derived asset.** `er-gfx/src/options_02_040.rs` opens by saying so: *"This does not
ship a game-derived GFx file. The DLL reads the game's own Scaleform MemoryFile, applies these
content-addressed tag edits in memory, and serves the derived movie for that process."* The mod
carries EDITS, not content. It also means the mod adapts to whatever build is installed instead of
pinning one.

**Fail closed, and gate the encoder on itself.** `apply_edits` is all-or-nothing and refuses any
`new_tag` that does not parse as exactly one tag AND re-serialize to those exact bytes.
`options_02_040_quit6_swap_to_edited` validates the vtable, the payload pointer, the length and the
magic before it reads a byte, and on any failure logs `FAIL-CLOSED (serving native vanilla)` and
serves the untouched original. Serves and failures are separate counters.

The runtime move itself is three writes: derive once, cache in a `OnceLock` so the pointer outlives
the call, then repoint the native file object's `data`, `len` and `cursor`.

### ER's grid is DS2's grid

`er-gfx/src/options_02_040.rs` documents `CS::GridControl::MeasureGridFromMovie`: it *"does not take
its geometry from the property list -- it MEASURES it from the movie"*, probing the child component
named `Item_<row>_<col>` and stopping *"at the first row whose column 0 is absent"*.

That is `FrontendEx::FexGridControl`'s layout bind at `0x1400216d0`, eight years earlier, probing
`(col, row)` and stopping at the first null. Same engine lineage, same trap, and the same fix:
**the layout has to contain the cell, or the engine will not draw it.** ER's fix was four added
`PlaceObject2` tags. Ours is however many `.flo` records the equivalent turns out to be.

### Where DS2's hook goes -- and why NOT at the EBL read

Option 2 as first written said "hook the EBL read and serve a modified `.febnd.dcx`". Reading
er-mods-rs corrects that: **`er-gfx` has no zlib dependency at all**, because it edits at the point
the bytes are already decompressed. The same is available here and is strictly better:

| layer | what it costs |
| --- | --- |
| EBL read (`/menu/42.febnd.dcx`) | the DLL must DCX-decompress, rebuild BND4, re-compress -- a zlib dependency and a container writer, for nothing |
| **BND member (`l42_01_OptionSetting.flo`)** | **the bytes are already the layout. Derive, cache, repoint.** |

The archive layer is identified. `FUN_14090f1d0` at `0x14090f1d0` is the path hash -- the `imul` by
37 at `0x14090f5e8` is one of exactly TWO in the whole `.text`, and it is the same `h = h*37 + c`
that `scripts/ds2-ebl.py` reimplements, which is a pleasing cross-check: the Python that opens the
archive offline and the game's own lookup agree by construction. Its five callers are the
entry-lookup family -- `0x14090f9c0`, `0x140911570`, `0x140911c00`, `0x140912290`, `0x140912810` --
which take a container, a name, and an out-descriptor, and fill the descriptor from the matching
entry (`FUN_14092b880`). That descriptor is the DS2 equivalent of ER's `MemoryFile`: the thing whose
pointer and length a detour repoints.

**Which of the five, and the descriptor's field offsets, are not yet nailed.** That is the next
piece of static work on this side.

### The blocker is the `.flo` record format, and pattern-matching will not crack it

Enough was learned to be sure of that rather than to hope otherwise. The 16-byte prefix that
precedes element id `0x5f5b9f2` occurs exactly **three** times, not five, so the other two
occurrences of that id sit in records of a different shape; the gaps between the three are 240 and
5240 bytes, so there is no single stride to read off. The section that contains them
(`0x3790..0xbfe0`) is 34896 bytes, which 240 does not divide.

The right way to decode it is to read the game's parser, not to keep diffing byte patterns. That is
`ds2-mods-rs-glz`.

### The shortcut ER used, which is available here too

ER's edits 1-2 were machine-generated: `scripts/gfx_tag_diff.py --emit-rust` over a vanilla/edited
pair. But edits 3-4 -- the two cells that took the quit tab from four rows to six -- were **hand
authored**, by copying the record above and changing depth and position, and were gated on this:
*"the MATRIX encoder was re-derived and made to reproduce edits 2, 3 AND the `Item_2_0` of edit 4
byte-for-byte before it was allowed to emit a fourth cell."*

That is available for `.flo` and it is much cheaper than a full format decode: find the record for
the cell that already exists, clone it, change its id and its position, and let the gate be that our
encoder reproduces every existing record byte-for-byte before it is allowed to emit a new one. It
needs record boundaries and the position fields -- not the whole format.

## Measured again, 2026-08-28: the grid is one column by N rows

The first run established that the appended item is selectable and invisible. This one says why,
with a five-tab control group, because `ds2-menu-row`'s probe reports every tab and touches one.

```
tab  items  col-extent  row-extent   verdict
 0     1        1           1        ok: every item has a cell
 1     1        1           1        ok
 2     2        1           2        ok
 3     3        1           3        ok
 5     2        1           2        ok
 4     4        1           3        NO CELL -- one item more than there are cells
```

Every tab is `col-extent = 1`, and every untouched tab has `row-extent == items` **exactly**. The
layout authors precisely as many cells as the tab has items. Ours has one too few, and it is the
only one that does.

`scroll = 0x0` on all six, so there is no scrolling anywhere in this control and the "virtualised
list" reading is dead: nothing was going to scroll the fourth row into view.

**This corrected the probe's own verdict line.** It first compared `items` against the COLUMN
extent, which reported `NO CELL` for four perfectly healthy tabs. The cells are counted along the
ROW axis. `FUN_1400222c0` says the same thing from the other side: it special-cases
`rowExtent == 1` to mean a single row, and these tabs do not take that branch.

So the target is exact: **tab 4's grid needs a cell element at `(col 0, row 3)`.**

## Quitting to desktop is one byte

`FeSubStateTitleShutdown::v1` (`0x1400fde20`) is the game's own quit-to-desktop, in full:

```asm
mov rax, QWORD PTR [rip+0x15773d1]      ; [0x1416751f8], the title-flow singleton
mov BYTE PTR [rax+0x13a], 1
ret
```

Its `update` (`0x1400ff2e0`) is an empty `ret`, so the substate does not perform the shutdown -- it
requests one.

Four sites write that byte (`0x1400f4a9a`, `0x1400fbc00`, `0x1400fde23`, `0x1401c2303`) and exactly
two read it (`0x1401bf97e`, `0x1401c0196`). **Both readers are inside `GameManagerImp`'s per-frame
master update** -- the function that also drives `mapManUpdate`, `damageManUpdate`,
`bulletManUpdate`, `demoManager` and `saveRequest`. Being polled by the main loop is what makes the
write usable from anywhere: it takes effect on the next frame, through the game's own shutdown, and
the writer is not on the stack when it happens.

It does not save and it does not ask. The quit-to-title flow offers to save because that flow asks;
this is the "without a confirmation" path and the absent save is the same coin.

`ds2-menu-row` now detours [`ds2_rva::FE_INGAME_MENU_DISPATCH`] and handles one action id of its
own, [`ds2_rva::FE_INGAME_MENU_ACTION_QUIT_TO_DESKTOP`] (`0x1000`), by writing that byte. The id is
deliberately outside the game's `0..=9, 0xb, 0xc, 0xd` case range: if the detour is ever absent the
row falls to the switch's `default` and is INERT, rather than quietly doing whatever the game does
for some id we borrowed.

**So the action half of the goal is done and the presentation half is not.** The row exists, is
selectable, and quits to desktop. It cannot be seen, and that is one `.flo` record away.
