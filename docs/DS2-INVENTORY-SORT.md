# The inventory sort DARK SOULS II already ships, and the button you cannot move

Everything in the first three sections was read statically from `darksoulsii-deobf.bin` (SOTFS
build 9527516) and from `/menu/text/english/*.fmg` with `scripts/ds2-fmg.py`, `scripts/ds2-rtti.py`,
`scripts/ds2-xrefs.py`, `scripts/ds2-disasm.py` and the Ghidra MCP daemon. **No game was launched to
establish any of it.** Addresses are VAs, the form the disassembly prints; subtract `0x140000000`
for the RVA `ds2-rva` records.

## The finding, first, because it changes what there is to build

**DARK SOULS II sorts the inventory already.** The feature is complete, shipped, and reachable from
the pause menu's Inventory tab. What it lacks is any way to move the button.

```
common.fmg  10004      '1:Sort'
            80059901   'How should the list be sorted?'
            80050101   'Default positions'    80050102  'By effect'
            80050103   'Attack'               80050104  'Weight'
            80050105   'Damage reduction'
            80050201   'Default positions'    80050203  'Defense'      80050204 'Weight'
            80050301   'Default positions'    80050302  'Weight'
            80050401   'Default positions'    80050402  'Attack'       80050403 'Held'
            80050501   'Default positions'    80050502  'Held'
```

### It is the pause menu's Inventory tab, and that was established rather than assumed

The object that owns the sort is allocated `0x3EE8` bytes by the factory at `0x1400727b0`. That
factory has no RIP-relative reference anywhere in the image -- it is reached through a vtable slot,
at `0x1410b1d78`. Walking that vtable back eight bytes to its `RTTICompleteObjectLocator` and
reading the type descriptor names its owner:

```
LoadAndExecJobSequence  inside  FeGroupInGameMenuInventory2::CreateExecJob
```

The constructor it tail-calls, `0x1400716f0`, then writes four vtables into the object -- primary
`0x1410b1b38` at `[this]`, and `0x1410b1cc8`, `0x1410b1d10`, `0x1410b1d20` at `+0x50`, `+0xc8` and
`+0x20a8`. RTTI resolves all four to `FeGroupInGameMenuInventory2`. So the Inventory tab creates it,
and the class that carries the sort is the tab itself.

## The option table

`0x14155ddf0`, fifteen entries of `{u32 FE_ITEM_PARAM_TYPE key, u32 fmgId}`, sliced per category by
`FUN_1400349d0(out, category)`, which returns a begin/end pair:

| category | range | keys |
| --- | --- | --- |
| 0 | `0x14155ddf0..de18` | 91 Default positions, 95 By effect, 92 Attack, 40 Weight, 93 Damage reduction |
| 1 | `de18..de30` | 91 Default positions, 94 Defense, 15 Weight |
| 2 | `de30..de40` | 91 Default positions, 58 Weight |
| 3 | `de40..de58` | 91 Default positions, 92 Attack, 80 Held |
| 4 | `de58..de68` | 91 Default positions, 80 Held |

The array ends exactly where `FeItemDataRepository`'s type descriptor begins, which is what confirms
the slicing rather than assuming it.

### What the keys mean, and why "Attack" is a real attack rating

The list is built by `FUN_140036080` as an array of `{i16 handle, i16 pad, f32 value}` and handed to
a stable sort (`FUN_140033410` -> `FUN_140032e50`, comparator `FUN_140033de0`). The comparator
compares the PRECOMPUTED `f32`; ties fall back to keys 91, 95, 81 in that order. The `f32` comes
from a switch over roughly ninety-five `FE_ITEM_PARAM_TYPE` keys in `FUN_140032460` and its two
siblings:

| key | meaning |
| --- | --- |
| `0x5B` (91) | Default positions -- `ItemParam` row `+0x2C`, the param's own sort id |
| `0x5C` (92) | Attack -- **the sum of all seven attack components**, a total attack rating |
| `0x5D` (93) | Damage reduction -- the sum of nine defence floats |
| `0x5E` (94) | Defense -- the sum of twelve |
| `0x5F` (95) | By effect |
| `0x28`/`0x0F`/`0x3A` | Weight |
| `0x50` (80) | Held -- the `u16` stack count |

## Choosing an option, and where the choice is kept

Each dialog row carries a closure (vtable `0x1410b1ec0`) holding the key at `+0x20` and the menu at
`+0x28`. Its member function is `0x1400ba300`:

```c
category = *this->vtable[0x140](this, &tmp);
cur      = getSortKey(inventory, category);      // 0x1401ac0a0
if (abs(cur) == key || cur < 0) key = -key;      // the SAME key again reverses the direction
setSortKey(inventory, category, key);            // 0x1401ac610
this->vtable[0x148](this, 1);                    // rebuild the list
```

`getSortKey`/`setSortKey` are `innerBag + 0x1006C + category*2`, five `i16`s, the sign carrying the
direction. That is the same object whose `+0x10138` is `ITEM_GIVE`'s error word, one `+0x10` hop
from `ItemInventory2`.

**`[group + 0x3EE4] is NOT the sort key.** `0x140074690` writes its third argument there and
`0x140074630` branches on it being 1 or 2; it is a rebuild-mode field. This is written down because
it was misread as the sort key once.

## Acquisition order: the save does not track it

There is no timestamp and no monotonic counter on an inventory entry. `FUN_1401b87b0` classifies a
handle by range -- `0x0000..0x0EFF` is the bag (3840 entries, matching `ITEM_ENTRY_COUNT`),
`0x0F00..0x0FFF` another list, `0x1000..0x103F` the shop -- and the shop lookup `FUN_1401ac090` is
`entry = base + (handle - 0x1000) * 0x28`. **A handle is a slot index.** Descending handle would be
"newest first" only to the extent that the bag fills upward and freed slots are not reused, and
**the free-slot policy has not been established**. Nothing should claim acquisition-order sorting
until it has.

## What cannot be rebound, in either game

`win32onlymessage.fmg` 10332..10341 is the COMPLETE list of rebindable menu actions:

```
10332 Move cursor (up)     10333 (down)      10334 (right)   10335 (left)
10336 Confirm              10337 Cancel
10338 Toggle menu (left)   10339 (right)
10340 Function 1           10341 Function 2
```

Sorting is not one of them. It rides on one of the two generic Function keys -- the `1` placeholder,
which is also `1:Switch` in Equipment, `1:Delete` in messages and `1:Default` in options -- so
rebinding it moves all of those together. That the circled numbers are input placeholders rather
than decoration is not inferred: `0x1405056a0` walks every string in a message and swaps `19` for
`20`, which is the confirm/cancel flip for Japanese pads.

There is **no controller remapping in DARK SOULS II at all**. A grep across all twenty-six shipped
`.fmg` files finds no button-config screen; the options list is camera, vibration, audio, HUD and
brightness.

**ELDEN RING is no better.** Its Key Bindings list (`GR_MenuText` 280100..280501) covers movement,
camera, switch armament/item/spell, attack/guard/skill/event action/use item/two-hand, Main Menu and
Map, and stops before any menu key. Its sort prompt is `GR_KeyGuide` 120020 `"Sort"`, registered at
`eldenring.exe+0x75a647` with menu-input id `0x2E` -- the same input as the map's "Center on current
location". **Which physical button that is has not been established**: it needs ER's input-id to
button table, which is a different game's binary and was not worth the dig for a config default.

## So the mod is a rebinding

`crates/ds2-inventory-sort`. `FE_INVENTORY_SORT_DIALOG_OPEN` (`0x1400747e0`) takes one argument --
the live group -- and builds and shows the dialog itself, so the whole feature is: know where the
group is, read a button, call it.

* `0x1400716f0` (ctor) records the group; `0x1400725d0` (`v0`, the scalar deleting destructor)
  clears it. Without the second, a press after the tab closes is a call into freed memory.
* `ds2-menu-row`'s per-frame tick reads the button on the game thread. `add_tick` used to be inert
  unless some crate also registered a ROW; it now installs on either.
* `[group+0x58]` is the game's own guard: non-zero while a child dialog is up, and the shipped
  function returns having done nothing. The refusal is the game's, not the mod's.

Nothing is injected into the input path. A synthesised press would also fire every other thing the
shipped Function key does in whatever menu happened to be open, and `ds2-safe-input`'s own docs
describe the failure mode of a button nobody released.

## The equip screen: the sort is already there, the button never was

Choosing a weapon to equip opens a `FeGroupItemEquip` list, and that screen ships **no sort prompt
at all** -- so there is nothing to rebind there, only something to add.

**The sorting is already wired in, and that was read rather than assumed.** The picker's list is
rebuilt by `FeIngameItemSelectMenu::v57` (`0x140097400`, this class's vtable slot `+0x148`), and at
`0x140097428` that function calls `0x140036080` -- the same shared builder the Inventory tab uses,
which reads the per-category sort key at `0x140036108`. There are only four readers of that key in
the whole image (`0x140033bec`, `0x140036108`, `0x1400ba331` in the dialog's own closure, and one
false positive), so the equip list and the inventory list are sorted by the same value. **Choosing a
sort in the Inventory tab already reorders the equip list**; what the equip screen lacks is only a
way to choose one without leaving it.

### One opener serves both, and the game already proves it is generic

`FeGroupInGameMenuInventory2`, `FeGroupItemEquip` and `FeItemBoxMenu` (the storage box) share a base
class. Their `+0x50` vtables are identical slot for slot through `+0x28`:

```
+0x00 0x14001baf0   +0x08 0x14001bae0   +0x10 0x140072ba0
+0x18 0x140073150   +0x20 0x1400bb9c0   +0x28 0x140074f70    (all three classes)
```

So `[this+0x58]` (the busy guard) and `this+0x50` (the dialog's parent) mean the same thing in each.
Everything else the opener needs is virtual: slot `+0x140` returns the category, slot `+0x148`
rebuilds the list, and each class supplies its own -- `FeItemSelectMenu::v40` / `FeIngameItemSelectMenu::v56`
for the category, `0x140074ff0` / `0x140097400` for the rebuild.

Two of the three ship an opener -- `0x1400747e0` (inventory) and `0x1400c2ad0` (storage box) -- and
they are near-identical: same busy check, same `+0x140` call, same `0x1400349d0` slicing, same
`this+0x50` handed to the dialog. **They also build their rows from the SAME closure**, vtable
`0x1410b1ec0` and member function `0x1400ba300`. That is the decisive fact: an Inventory-named
functor that the shipped game already reuses for a different class's list. The only difference is
one field (`0x8000000005F5E110` written to `dialog+0x860`) that the inventory copy sets and the
storage box's does not.

`FeGroupItemEquip`'s constructor is `0x14008c0d0` and **takes four arguments, not three** -- it spills
`R8D` into its home slot on the entry instruction and reads `R9` fourteen bytes later. Its scalar
deleting destructor is `0x14008dc70`, slot 0 of vtable `0x1410b46c8`. Neither is Arxan-redirected.

## What a run has settled

One run, 2026-08-29, `--inventory-sort --sort-key F7`, staged sha256 `357f1386...c5e2`:

* `FeGroupInGameTopSelect::v2` -- the tick this borrows -- **does keep running while the Inventory tab
  has focus**. That was the open question and it is answered; no fallback to the Inventory group's
  own update is needed.
* `GetAsyncKeyState` reads the bound key through Proton with the focus check passing.
* The constructor detour records the live group and the vtable check passes:
  `opening the sort dialog group=0x00007fffe9929000 busy=0`.
* Calling `0x1400747e0` from that tick does not fault, and **the dialog appears on screen**.

It also found a defect: `LOGGED_LINES` was 3, and three presses outside the Inventory tab spent the
whole refusal budget before the real test. Raised to 24 -- the tick is edge-triggered, so the cap
bounds presses, not frames.

## The ELDEN RING button, and why the binary would not give it up

The default controller binding is **`lthumb` -- left stick click, L3**. It is the one value in this
crate that did not come out of a binary, and the trail is written down here so nobody re-walks it.

What static analysis *did* settle, in `eldenring-deobf.bin` (image base `0x140000000`, flat):

```
0x14075a1f6  mov edx, 0x1d4d4      ; GR_KeyGuide 120020 = "Sort"
0x14075a1ff  call 0x1407606a0      ; fetch the prompt text
0x14075a217  lea edx,[rdi+0x2e]    ; edi=0 -> menu-input id 0x2E
0x14075a21d  call 0x14075e9e0      ; register(this, 0x2E, flag, text)
```

All 39 key-guide registrations were pulled the same way. ELDEN RING's whole menu uses nine input
ids -- `0x18 0x19 0x23 0x27 0x28 0x29 0x2A 0x2E 0x2F` -- and `0x2E` carries both `Sort` and the map's
"Center on current location", so it is one generic function control doing two jobs, structurally the
same thing as DS2's `1`.

**Where it dies: ELDEN RING ships no keyboard key-name strings at all.** `Backspace`, `PageUp`,
`Numpad`, `CapsLock` and `LeftShift` all return zero hits in both ASCII and UTF-16. It draws inputs
as sprites from an atlas, so there is no name table to resolve an input id against, and the
registration only forwards the id into a closure chain (`0x14075e9e0` -> `0x14075e8b0` ->
`0x140758500` -> `0x14075d970`) that re-resolves the glyph per frame as the active device changes.
Getting from `0x2E` to a physical button means reverse-engineering the input-binding structure of a
different game -- more work than this entire DS2 feature was, for one fact.

So the player named it. `scripts/which-pad-button.py` exists for the next time this comes up: it
reads evdev directly and prints the button in the spelling `ds2-mods.toml` takes, which beats asking
someone to name a button they know as a place their thumb goes.

## Not established

* Whether the bag reuses freed slots, which is what "sort by acquisition" would rest on.
* Whether the equip picker's sort OPTIONS match the slot being equipped, and whether choosing one
  visibly reorders the list. The dialog is confirmed to open there; its contents are not.
