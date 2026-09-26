# Attunement: how the game decides whether a spell fits

A static read of `DarkSoulsII.exe` (Ghidra on the deobfuscated image, cross-checked against
`darksoulsii-deobf.bin` by RVA) and of `ds2-build-import`. It answers where a spell's slot cost
comes from, whether the budget is spent per spell, what sets the budget, and why a build import
reported `Great_Heal_Excerpt` as not equipped with the slot reading back empty.

Every claim is tagged. "Verified in binary" means the instruction was read at the address given.
"Verified in code" means the crate source says so. "Inferred" means neither, and says what would
settle it.

## Short answer

- A spell's cost is enforced only by the attunement recalculation (`0x1401b40f0`), never by the equip
  function. Attuning checks position only. (verified in binary)
- The cost is a byte of the spell's param row at `+0xF0`, copied into the inventory entry at
  `+0x21` when the entry is created. (verified in binary)
- The budget is `base + PlayerParam[0x36]`, capped at fourteen, where `PlayerParam[0x36]` is
  recomputed from the stats by the same stat setter the import already calls. (verified in binary)
- The failure in the soulsplanner build that prompted this was not a budget problem. That build names
  `Great_Heal_Excerpt` in two attunement positions. The import resolves both positions to the one
  inventory entry, so the second equip moves the spell out of attunement 2 and the game's compaction
  pulls it straight back into attunement 2. Attunement 3 reads back empty. (verified in binary and
  code; the build's spell list was read from its soulsplanner page)

## The equip function checks position, not cost

`ItemInventory2::SetEquip` is the thunk at `0x1401ac510`, which lands at `0x1401b3d50` for slots
up to `0x29`. (verified in binary)

For an attunement slot the only budget gate is at `0x1401b3e69`:

```text
1401b3e69  movzx ecx, byte [rsi+0x259ec]   ; the budget
1401b3e70  lea   eax, [rbx-0x1c]            ; attunement position, from the internal slot
1401b3e73  cmp   eax, ecx
1401b3e75  jl    0x1401b3e91                ; inside the budget: equip
1401b3e77  ...   jmp 0x1401b4330            ; otherwise: unequip that slot
```

No spell cost is read anywhere on this path, and no other refusal exists for a spell apart from the
category check (`[item-type row+0x18] == 7` at `0x1401b3e45`). (verified in binary)

So a spell that costs more than one slot is attuned just the same. A per-spell cost cannot explain
an equip that reads back empty straight away. (verified in binary)

## After the write, the game compacts and groups the attunement array

The attunement slots are eight-byte entry pointers at `bag+0x25910`, internal slots `0x1c..0x29`,
which is the equipped-slot array at `bag+0x25830` indexed from slot `0x1c`. (verified in binary:
`0x1401b3fd4` writes `bag+0x25830+slot*8`; `0x1401b4121` walks from `bag+0x25910`)

A spell write then calls `0x1401b4700(bag, position)` at `0x1401b3feb`, which:

1. Scans positions below the one written. If any is empty, it swaps the new spell into the first
   empty one. (verified in binary)
2. Scans down from the top of the budget for another slot holding the same item id (`entry+0x14`)
   and moves the new spell next to it, so copies of one spell sit side by side. (verified in binary)

Unequipping an attunement slot (`0x1401b4330`) shifts every later spell down one position with a
`memcpy` and clears the last. (verified in binary)

Consequence: the position a spell was aimed at is not necessarily where it ends up. A build whose
spell list has a gap, or which repeats a spell out of order, reads back empty or different at the
aimed position even when the spell was attuned. (verified in binary)

## Equipping an entry that is already equipped is a move

When the entry's equipped flag (`entry+0x1f` bit 1) is set, `SetEquip` searches the category's
slots for the one holding that entry (`slot entry+0x1c == entry index`) and unequips it first, at
`0x1401b3ea5`..`0x1401b3f8b`, before writing the new slot. (verified in binary)

## Where the cost comes from

The entry is filled at `0x1401af060`..`0x1401af093` (and the sibling at `0x1401af0f4`..`0x1401af118`):

```text
mov   ecx, [r9+0x24]      ; the item id
call  0x1401ab840         ; param row lookup
...
movzx eax, byte [row+0xf0]
mov   [entry+0x21], al    ; the slot cost
```

`0x1401ab840` is `[GameManagerImp+0x18]` (`CharacterManager`) then `0x140359030`, a `paramLookup`
on the param held at `[CharacterManager+0x540]+0xC8`. Ghidra's struct names that container
`SpellParam_container`. (verified in binary for the offsets; the param's name is Ghidra's label,
inferred)

The neighbouring `entry+0x20` is the cast count, from `0x1401ab750`:
`ceil(row[0xF0 + tier] * multiplier)`, where `tier` is `PlayerParam[0x37]`. So `+0xF0` is the cost
and `+0xF1` onward are casts per tier. (verified in binary for the arithmetic; the field meanings
inferred from that arithmetic)

The numeric cost of any particular spell is param data in the regulation file, not in the
executable. (not readable statically; reading `entry+0x21` of a live entry settles it)

## The budget is spent in the recalculation, cumulatively

`0x1401b40f0`, disassembled in full: (verified in binary)

```text
call  0x1401ab690                   ; PlayerParam+8, or null
jz    no_player                     ; null: keep the old budget
movzx ecx, byte [bag+0x259ed]       ; base
movsx eax, byte [player+8+0x2e]     ; PlayerParam+0x36
add   ecx, eax
...
mov   [bag+0x259ec], min(ecx, 14)   ; the budget
sum = 0
for each attunement slot, stopping at the first empty one:
    sum += entry+0x21               ; the cost
    if sum > budget:
        unequip this slot           ; later spells shift down into it
        (do not advance; sum is not reduced)
```

Because `sum` is never reduced, the first spell that pushes the running total past the budget and
every spell after it are unequipped. (verified in binary: the `jmp 0x1401b418b` at `0x1401b4183`
skips both the index and the pointer increment)

The recalculation runs in the game only from the save-load routine (`0x1401a5e46`, the image's only
direct caller) and as the tail of the base setter. `SetEquip` does not run it. (verified in binary)

So an import that recalculates before attuning and never after attunes by position only. What it
attunes is cost-checked the next time the character is loaded, and anything over the cost budget is
dropped then. (verified in binary and code)

## What sets the budget

`budget = min(14, base + PlayerParam[0x36])`.

- `base` is `bag+0x259ed`. Its only writer is `0x1401b42f0` (the store at `0x1401b431b`), which
  mirrors it to `[bag+0x25990]+0xF002` and tail-jumps into the recalculation. Its only caller is the
  save-load routine at `0x1401a5e3d`, passing a byte from the save. It is a save-carried extra, not
  the attunement stat. (verified in binary)
- `PlayerParam[0x36]` is written by the stat setter. `PLAYER_PARAM_SET_ALL_STATS` (`0x14038aca0`)
  calls `0x14038d6d0(PlayerParam+8, ...)` at `0x14038acff`. That recomputes the effective stats at
  `PlayerParam+0x1E`, then calls `0x14038d790(PlayerParam+0x34, PlayerParam+0x1E)`, which writes the
  derived block. Its byte `+2` (`PlayerParam+0x36`) is `row[+2]` of a per-level param row, looked up
  (through the Arxan-wrapped `0x140358b60`) at the effective value of the stat named by a selector
  byte in another param's first row (`[CharacterManager+0x590]`). (verified in binary, `0x14038d7c8`
  .. `0x14038d7fd`)
- Which stat that selector names, and the slot value at attunement 30, are param data. (inferred:
  attunement; the value is not readable statically)
- The only other writers of `bag+0x259ec` in the image are the recalculation (`0x1401b4150`), a
  set-to-value helper (`0x1401b40e2`, called with fourteen from `0x1401a7db9`), and the bag reset
  (`0x1401b1552`, `0x1401b2a72`). A byte-pattern search of the whole image finds no other reference
  that stores to it. (verified in binary)

This corrects an earlier reading that the recalculation cannot derive the budget from the
attunement stat. It can: the stat setter refreshes `PlayerParam[0x36]` and the recalculation reads
it, provided `0x1401ab690` finds a player. (verified in binary)

## The failed import, explained

The soulsplanner page for the build lists its spells as
`Climax; Great_Heal_Excerpt; Great_Heal_Excerpt; No_Spell; ...`. The plan skips empty names but
keeps each name's position, so it asks for Climax at attunement 1 and Great Heal Excerpt at both
attunement 2 and attunement 3. (verified in code: `ds2-build-import-core/src/equip.rs`, `plan`)

What the code as written does with that:

1. The grant list is built before anything is granted, so `already_held` answers the same for both
   occurrences. Whether one or two copies end up in the bag, the second equip resolves to the first
   copy, because `entry_for_item` returns an entry with the equipped flag as soon as it sees one.
   (verified in code: `ds2-build-import/src/game.rs`, `entry_for_item`)
2. Attunement 2 equips normally.
3. Attunement 3: the entry is already equipped, so `SetEquip` unequips it from attunement 2, which
   compacts the array and leaves attunement 2 empty. It writes attunement 3. Then `0x1401b4700`
   finds attunement 2 empty and moves the spell back into it. Attunement 3 is empty. (verified in
   binary, sections above)
4. The read-back at attunement 3 sees nothing and the import logs a refusal.

Of the causes the issue listed, it is the third: something other than the budget. The recalculation
ran, and the budget it answered was at least two (the crate's own positional check, which reads the
same byte, did not drop the spell, and nothing between that read and the equip writes the byte).
Slot cost plays no part in the equip. (verified in binary and code)

## What a fix changes

Nothing below is implemented. Rust edits in this repo wait for runtime evidence.

1. Resolve each spell position to its own entry. In `ds2-build-import/src/game.rs`, give
   `EquipRequest` an `exclude: &[usize]` list, and have `entry_for_item` skip entries in it, so an
   entry already attuned earlier in the same import is not taken again. Have `equip` return the entry
   it used, and have `equip_everything` in `ds2-build-import/src/flow.rs` collect those for spells.
2. Grant one copy per occurrence. In `build_items` in `ds2-build-import/src/lib.rs`, count how often
   each spell id appears in `build.spells`, and grant that count minus the copies already carried,
   instead of skipping the id whenever one copy is held. (inferred: that the game keeps separate
   copies of one spell as separate entries, which the grouping in `0x1401b4700` implies but does not
   prove)
3. Aim spells at compacted positions. In `equip_everything`, attune spells into consecutive positions
   (the count of spells placed so far) rather than the build's raw position, because the game moves
   them there anyway. Check the result as the set of spells in the attunement slots after the loop,
   not slot by slot, because repeated spells are regrouped.
4. Check cost, not position. Replace `slot.position >= live` with a running total of each entry's
   `+0x21` byte, stopping at the first spell for which `total + cost > budget`. That is the
   recalculation's own rule, so what the import attunes is what survives the next load.
5. Run the recalculation once more after the spell loop and log any spell it drops, so a cost the
   crate got wrong shows up in the same run rather than on the next load.
