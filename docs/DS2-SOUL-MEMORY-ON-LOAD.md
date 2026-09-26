# Soul memory on character load: can the live value be stale?

The build importer reads soul memory from `PlayerParam + 0xF4` and `+ 0xFC` and skips raising it
when that read already covers the target level. A live run reported a freshly created level-12
character reading exactly the soul memory an earlier press had computed for level 150 on another
character in the same launch, with Brotherhood of Blood already marked discovered. The question
here is whether the game itself can carry those fields from one character to the next without a
full reload.

Everything below is read statically from `DarkSoulsII.exe` (SOTFS) in Ghidra and from
`darksoulsii-deobf.bin`. Each claim is tagged **verified in binary** (with the address) or
**inferred**.

## Answer

**Not stale across a character switch.** A character switch means quit to title and load or create
another character, which is the only way DS2 switches. The game builds a new `PlayerParam` with
zeroed soul counters for every player it spawns. On load, the only thing that fills those counters
is the save record, and that record is freed and rebuilt from zero on every way back into the
world that passes through the title. The single way into the world that keeps the old record is an
in-game map transition, and that keeps the same character.

So the importer's read reflects the character the game loaded. The crate does not cache the
pointer or the value: `flow.rs::apply` calls `player_param()` and `read_soul_memory()` fresh on
every press. **No re-read change is needed.** The observed number has some other cause, covered at
the end.

## The PlayerParam is new for every spawned player

- **Verified in binary.** `PlayerCtrl`'s init (`0x14037f3d0`) heap-allocates `0x1e0` bytes and
  calls the `PlayerParam` constructor at `0x14037f4b1`. The constructor (`0x14038aa20`) writes zero
  to souls held (`+0xEC`), both soul-memory fields (`+0xF4`, `+0xFC`), each counter's skip byte
  (`+0xF0`, `+0xF8`, `+0x100`), and the covenant bytes from `+0x1AC` through `+0x1B7`, which
  include the per-covenant discovered flags at `+0x1AE` onward.
- **Verified in binary.** The world-entry spawn (`0x140419610`) creates the player through
  `CharacterManager` (`0x1403578e0`) and stores the result in `GameManagerImp->PlayerCtrl`. Every
  unload (`0x1405cd79b`, which sets game state `0x15`) nulls `GameManagerImp->PlayerCtrl` first,
  so the getter the crate calls cannot return the previous character's object once it has been
  unloaded.

## The only load-time write: RestoreFromRecord

- **Verified in binary.** `0x14038ad20` (`PLAYER_PARAM_RESTORE_FROM_RECORD` in `ds2-rva`) assigns
  souls held from `record + 0x1C`, soul memory from `record + 0x20` and `record + 0x24`, and the
  covenant and discovered flags from `record + 0x9C` onward. It only caps each counter at the top,
  and it skips a counter whose skip byte is set. On a freshly constructed param every skip byte is
  zero (see above), so all three counters are written.
- **Verified in binary.** Its only direct caller is `0x14037eca0` (called at `0x14037ecf0`), and
  that function's only direct caller is the world-entry spawn at `0x140419954`. The record it is
  handed is `GameDataManager + 0xD0` (class `GameDataPlayerTempData`) plus `8`.
- **Verified in binary.** The spawn restores only when the byte at `TempData + 0x368` is set. When
  it is clear, the spawn does the opposite: it serialises the new player into the record
  (`0x14041998c` calls `0x14019ea20`), which also sets the byte.

`0x1400ee240`, named in the soul-level notes as the load path that sets the level, is not the
gameplay load. **Verified in binary:** its only direct caller is
`FeSubStateTitlePlayerInformation` (`0x1400fdc40`, call at `0x1400fdcab`), the title screen's
character information page. It writes stats, level and covenant into whatever `PlayerCtrl` exists
at that moment, and it never touches souls held or either soul-memory field.

## Where the record comes from, and when it is thrown away

- **Verified in binary.** The record is written by:
  - the save loader `SaveDataPlayer` (`0x1402e5cd0`), which copies the character's player chunk
    from the save stream into it (`0x14019eb10` for chunk version `0x6e`, `0x14019ea20` for
    `0x6f`; both copy the range holding the soul counters and set the `+0x368` byte);
  - the in-game snapshot at `0x1401bf964`, inside the handler that moves the game to state `0x1f`;
  - the world-entry spawn described above, when the byte is clear.
- **Verified in binary.** The byte is cleared only by the `GameDataPlayerTempData` constructor
  (`0x14019e9b0`), which also zeroes the whole record. The constructor's only direct caller is
  `0x14048b200`, called only from game state `0x10` (`0x1401be485`). The object is freed and its
  pointer nulled by `0x14048b190`, called from game state `0x11` (`0x1401be560`).
- **Verified in binary.** The unload in state `0x15` (`0x141cf4cea`, body at `0x1401bee21`) picks
  the next state from bit `0x08` of `GameManagerImp + 0x24b1`: clear goes to `0x11` (full teardown,
  record freed), set goes straight to `0x12` (record kept).
- **Verified in binary**, for immediate stores (`mov dword [reg+0x24ac], imm32` searched over the
  whole image). Every store of game state `0x12` is in state `0x10` (`0x1401be53a`) or in that
  bit-`0x08` branch (`0x1401bee91`). State `0x10` is stored only by state `0x0e` (`0x1401be318`),
  `0x0e` only by `0x0d`, `0x0d` only by `0x0b`. So re-entering the world after a teardown always
  allocates a fresh record. A state stored from a register would not show up in that search;
  none was seen in the state handlers read here.
- **Verified in binary.** The writes to `GameManagerImp + 0x24b1` were enumerated: `or` with an
  immediate and `mov` from a register over the whole image, and every write form inside the game
  state code (`0x1401b8000` to `0x1401c8000`). The only one that sets bit `0x08` is
  `0x1401c2c1e` in `0x1401c2a80`, which ORs in `0x0a` and runs only while the game state is `0x1e`
  (in the world). That function copies a destination map id and position into `GameManagerImp`,
  so it is a map transition request. **Inferred** from those fields: warps, and not a return to
  title. `0x1401c2cf0`, the other in-world request, clears bit `0x08` (`and 0xd7`), and the state
  `0x15` branch clears it again after use.
- **Inferred.** The save loader runs after state `0x10` has built the new record. If it ran
  earlier, the record pointer would be null, `0x1402e5cd0` would skip the copy, and no loaded
  character would keep its stats.

## New Game does not reset soul memory, but it starts from zero

- **Verified in binary.** Character creation's class apply (`0x1400de610`) works on the live
  `PlayerCtrl`: `addSoul` with the class's starting souls, then the class stats through
  `assignAttributes`, then the class covenant. It adds to soul memory instead of assigning it, and
  it leaves the discovered flags alone.
- **Inferred.** That would carry the previous character's soul memory into a new one only if the
  spawn had restored a stale record. After a quit to title the record has been rebuilt from zero,
  so the spawn restores nothing and a new character starts at zero plus the class's souls.

## What the observation must be instead

- **Inferred.** The level-12 character most likely had been through an earlier press itself.
  `AddSouls` succeeded, so soul memory reached the level-150 floor and the covenant was marked
  discovered, but the stats were never written, or a save made after that press was reloaded. Both
  fields match that earlier press exactly, which fits one character carrying its own history
  better than two characters sharing a block.
- **Still open.** The check the report already names settles it without a rebuild: quit to title,
  reload that character, and read `PlayerParam + 0xF4` and `+ 0xFC` from `/proc/<pid>/mem`. A
  value that survives the reload is the character's own. Given the teardown above, the value
  should survive.

## Code change

**None for staleness.** Recommended, for diagnosis only: add the `PlayerParam` address to the
`character now:` log line in `flow.rs::apply`. A new character is a new heap object, so the address
changes with every spawn, and two presses that print the same address were made on the same
character. This is inferred to be useful and has no runtime evidence behind it yet, so it is
recorded here and not made under `crates/`.
