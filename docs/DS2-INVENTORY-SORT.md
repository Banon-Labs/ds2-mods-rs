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
common.fmg  10004      '①：Sort'
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

Sorting is not one of them. It rides on one of the two generic Function keys -- the `①` placeholder,
which is also `①：Switch` in Equipment, `①：Delete` in messages and `①：Default` in options -- so
rebinding it moves all of those together. That the circled numbers are input placeholders rather
than decoration is not inferred: `0x1405056a0` walks every string in a message and swaps `⑲` for
`⑳`, which is the confirm/cancel flip for Japanese pads.

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

## Not established

* Which physical button ELDEN RING puts `Sort` on.
* Whether the bag reuses freed slots, which is what "sort by acquisition" would rest on.
* Whether `FeGroupInGameTopSelect::v2` -- the tick this borrows -- keeps running while the Inventory
  tab has focus. It is on screen, so it should; **no run has confirmed it**, and if it does not, the
  fix is to hook the Inventory group's own update instead.
* Anything at all about how this behaves in a running game. **It has not been run.**
