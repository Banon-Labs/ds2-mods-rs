# Soul level, soul memory and the level-up cost table

What a save-load guard needs in order to refuse a character whose soul memory cannot pay for its
soul level. Everything here comes from `DarkSoulsII.exe` (SOTFS) read statically in Ghidra, plus
read-only decrypts of local `.sl2` files with `scripts/ds2-sl2.py`. Each claim is tagged
**verified in binary** (with the address that shows it) or **inferred**.

## Soul level is stored, and it is derived from the base stats

- **Verified in binary.** `sPlayerParam+0xD0` is the soul level, an `int`. The level-up menu
  (`FUN_1401fb530`) copies the block starting at `sPlayerParam+0x08` (the thunk at `0x14038b990`
  is `lea rax,[rcx+8]`) and `FUN_1401fbac0` seeds its working level from `+0xC8` of that copy,
  which is `sPlayerParam+0xD0`. `onSoulAbsorb` (`0x14013d8f0`) reads the same field as the player's
  level. The only direct setter is `FUN_14038bd70` (`mov [rcx+0xd0], edx`).
- **Verified in binary.** The game computes that level from the stats in two places,
  `FUN_14038d570` and `FUN_14038e310`, with the same arithmetic:

  ```
  level = max(1, (short)(attr[0] + attr[1] + ... + attr[8]) - 0x35)
  ```

  They sum all of `attributes` (`short[11]` at `sPlayerParam+0x08`) and then subtract `attr[9]`
  and `attr[10]` back out, so only the nine real stats count; the offset is 53. `FUN_14038d570`
  also refuses the whole assignment, leaving the old values in place, if any of the eleven shorts
  is outside `1..=99` (`0x62 < (ushort)(v - 1)`). Both functions then call `playerSvrSync(0x10)`
  and `playerSvrSync(0x12)`, which is how level and stats reach the server.
- **Inferred, consistent with the formula.** A vanilla 100% character with all nine stats at 99
  sums to 891, and 891 - 53 = 838, the known level cap. The donor save 'Elden Wolf' (stats summing
  to 309) is therefore level 256.

### Loading a character does not recompute the level

- **Verified in binary.** `FUN_1400ee240` fills `sPlayerParam` from a per-character record:
  `assignAttributes(param, rec+0x174)`, then `FUN_14038bd70(param, *(u16*)(rec+0x1d4))`. The level
  is taken from the record as-is; nothing on that path runs the formula above. A save whose stored
  level disagrees with its stats keeps the disagreement until something calls `FUN_14038d570` or
  `FUN_14038e310` (a level-up commit does).
- **Inferred from the offsets.** The same record carries the UTF-16 name at `rec+0x18a`. In the
  `.sl2` slot record `ds2-sl2.py` reads, stats sit at `0x188` and the name at `0x19E`, a shift of
  `0x14`. Applying that shift, the stored level is the `u16` at slot-record `+0x1E8`.
- **Observed in local saves (read-only decrypt).** In the character-list section of several local
  `DS2SOFS0000.sl2` files, slot-record `+0x1E8` equals `sum9 - 53` for most characters (for
  example 165, 156, 159, 170, 155) and reads 1 for blank characters (stats all 1, which the formula
  clamps to 1). A handful of downloaded characters disagree by a wide margin (stored 63 against a
  stat sum giving 91; stored 48 against 78). Which side the game trusts once such a character is
  playing needs a runtime read of `sPlayerParam+0xD0`; statically, the load path keeps the stored
  value.

## The level-up cost table

`PlayerLevelUpSoulsParam` is param index `0x27`; its container is `CharacterManager+0x580`
(**verified in binary**, CharacterManager struct and `FUN_140358b90`).

### Param file row format

- **Verified in binary** (`paramLookup` at `0x14003e8f0`, `FUN_140358b90`, `FUN_140358f10`,
  `ParamFileResourceObject` constructor at `0x140b2e630`). `ParamFileResourceObject` has its
  `Memory` object at `+0xC8`; that object's `mem_ptr` (`+0x10` within it, so `+0xD8` of the
  container) is the raw param file. Every accessor checks `container+0xD8 != 0` before use.
- In that file: `+0x0A` is the row count (`u16`); the byte at `+0x2D` picks the row-table shape.

  | `+0x2D` | row-table entry at `0x40 + i * stride` | row data |
  |---|---|---|
  | zero | stride `8`: `u32 id` at `+0`, `u32 offset` at `+4` | `file + offset` |
  | non-zero | stride `0x18`: `u32 id` at `+0`, `u64 offset` at `+8` | `file + offset` |

  `paramLookup` binary-searches that table by row id; `FUN_140358b90` indexes it by position and
  bounds-checks against the row count.

### Row layout and the cost function

- **Verified in binary** (`FUN_14038d140`, and its twin `FUN_1401ffda0`). A `PlayerLevelUpSoulsParam`
  row is read as `u16 level` at `+0`, `i32 step` at `+4`, `i32 souls` at `+8`. The cost function
  is:

  ```
  cost(L) = r.souls + (L - r.level) * r.step
  ```

  where `r` is found by starting at row index `L`, halving the index until the row exists and has
  `level <= L`, then walking forward while the next row's level is still below `L`. An exact match
  returns `r.souls` directly. The table is sparse: rows are anchor points, and the step
  extrapolates between them. The function reaches the table through
  `GameManagerImp->CharacterManager` itself, so a caller inside the process passes only `L`.
- **Verified in binary** (`FUN_1401fb970`, `FUN_1401fb800`). `cost(L)` is the price of going from
  level `L` to `L + 1`: raising the level deducts the cached cost, increments the level, then
  caches `cost(new level)`; lowering it decrements the level and refunds `cost(new level)`. The
  menu refuses to raise past level 1000 (`cmp [rcx+0xf0], 0x3e8`), and `FUN_1401fbd70`, the
  menu's own consistency check, requires `level == start level + points spent` and `level < 0x3e9`.

## What the guard compares

With the pieces above, a character at level `N` whose class started at level `S` has spent at
least `cost(S) + cost(S + 1) + ... + cost(N - 1)` souls, and soul memory
(`sPlayerParam+0xF4`) counts every soul ever gained, so it can never be below that sum.

- **Inferred.** The class start level is not in the save; its lowest-risk substitute is the
  highest start level any class has, which gives a lower bound that no legitimate character can
  fail. The per-class values live in the character-creation data and were not read here.
- **Inferred.** `total_get_soul_2` (`+0xFC`) is a second counter written by the same `addSoul`; which
  one the matchmaking bracket uses is a separate question.
- **Verified in binary.** `RelatePhysicalStatToLevelStatParam` (index `0x28`, `FUN_140358f10`)
  is not the level rule. `FUN_14038d784` reads its first row as a table of which stat feeds each
  derived value (HP, stamina, defences, resistances) and looks each one up in
  `PhysicalStatsPerLevelStatValuesParam`. It never touches the soul level.

## The guard as built

`ds2-soul-memory-guard`, switched on by `[soul_memory_guard] enabled = true` (`scripts/ds2-run.py
--soul-memory-guard`), detours `PlayerParam::RestoreFromRecord` (`0x14038ad20`). That function is
the gameplay load's only writer of the soul counters (see `docs/DS2-SOUL-MEMORY-ON-LOAD.md`), and
its first call is `assignAttributes` (`0x14038aca0`), whose recompute ends in `FUN_14038e310`. So
after it returns, `+0xD0` holds the level derived from the loaded stats and `+0xF4`/`+0xFC` hold
the record's soul memory (**verified in binary**). The guard then calls `FUN_14038d140` for each
step from level 14 to the character's level, compares the sum with `+0xF4`, and logs one line. It
refuses nothing: no safe way to turn a load away from inside the spawn was found.

Read live on 2026-09-26 with `scripts/frida/soul-guard-inputs.js` (read-only):

- `PlayerLevelUpSoulsParam` at `[[GameManagerImp]+0x18]+0x580`, file at `+0xD8`: 852 rows, shape
  byte `4` (wide table). Row 0 is `{level 0, souls 0}`, row 1 `{1, 500}`, row 2 `{2, 528}`, row 3
  `{3, 557}`, row 851 `{999, 100000}`. Row 0 at level 0 is what makes `FUN_14038d140`'s halving loop
  end for every level of one or more.
- The `i32` at row `+4` read as `1`, `2`, `3` and `999` on those rows, the same as the level, and
  not `0` as the note on `PLAYER_LEVEL_UP_SOULS_COST_OFFSET` in `ds2-rva` says. It only matters for
  a level with no exact row, which no level up to 838 is.
- A character at level 13 (stats summing to 66) held a soul memory of 1650. Summed from level 1 its
  floor is 8192, so a start of 1 would call a fresh character short; summed from 14 it is 0.

### A level-1 verdict after a continue is the slot, not the hook

The first runs with the guard on (`--continue-slot 0`) logged `verdict=supported level=1
soul_memory=0` and read stats `1,1,1,1,1,1,1,1,1` from `[[GameManagerImp]+0xD0]+0x490` in the
world. That looked like the hook reading a default-constructed object. It is the save: a read-only
`scripts/ds2-sl2.py --slots` of both `DS2SOFS0000.sl2` and `.co2` in the save folder those runs used
lists slot 0 as blank (stats summing to 9, no name) and slot 1 as the only character, `Swornsword
Ole`, stats summing to 66. The earlier level-13 read (stats summing to 66, soul memory 1650) was
that slot-1 character, at the same heap address. So `--continue-slot 0` loaded the blank slot, and
the guard read what was loaded.

`FUN_14038bd70`, the level setter, is not a load path to hook instead (**verified in binary**): its
only caller is `FUN_1400ee240`, and that function's only call site is `0x1400fdcab` in
`FeSubStateTitlePlayerInformation` (`0x1400fdc40`), the title screen's character information page.

## Open

- Whether the game trusts a stored level that disagrees with the stats, once the character is in
  the world, needs a runtime read of `sPlayerParam+0xD0` against the stats.
- The actual row values of `PlayerLevelUpSoulsParam` come from the regulation file and were not
  read; a runtime caller of `FUN_14038d140` does not need them.
- The ban criteria FromSoftware applies server-side have no primary source; this guard is defence
  in depth beside the offline default, not a reproduction of their rule.
