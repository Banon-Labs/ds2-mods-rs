# The pause menu, its six tabs, and where a new row would come from

Everything in the first four sections was read statically from `darksoulsii-deobf.bin` (SOTFS build
9527516) with `scripts/ds2-rtti.py`, `scripts/ds2-xrefs.py`, `scripts/ds2-disasm.py`,
`scripts/ds2-arxan-chain.py` and the Ghidra MCP daemon. **No game was launched to establish any of
it**, which is also why the last section is an experiment rather than an answer.

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

## The row count is code-driven

This is the fact that makes an extra row plausible rather than wishful. The per-tab init
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

So the visible row count is the item vector's count. Appending an entry asks for a row.

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

## The experiment (`crates/ds2-menu-row`)

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

### What each outcome would mean

| what the tab shows | what it establishes |
| --- | --- |
| four rows, fourth captioned | the layout authored five rows; a new item needs only an entry and a dispatch case |
| four rows, fourth blank | the row exists but its caption lives in `GameDataEbl.bdt`; adding a *labelled* item needs the archive |
| three rows, `appended` in the log | the grid is bounded by something other than the count, and `FexGroupList`'s binding is the next thing to read |

Only the third outcome would falsify the reading in this document.
