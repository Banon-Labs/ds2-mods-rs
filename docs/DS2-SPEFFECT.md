# Applying a SpEffect to the player

What a port of er-net-effects needs from `DarkSoulsII.exe` (SOTFS, image base `0x140000000`)
before any code is written: the function that applies a SpEffect, the object it takes, how to
reach that object for the local player, and what a SpEffect id refers to. Everything here was read
statically: Ghidra, `objdump` over the on-disk executable, and the decrypted regulation read with
`scripts/ds2-regulation.py` and `scripts/ds2-emevd.py`. Nothing was run.

Each claim is tagged **verified in binary** (with the address that shows it) or **inferred**.
Function names such as `applySpEffect` and `bonfireRestSpEffect` are labels a person typed into the
Ghidra project; the claims below rest on the instructions, not on those labels.

## Summary

```text
GameManagerImp*  = *(0x1416148f0)
PlayerCtrl*      = *(GameManagerImp + 0xd0)
ChrSpEffectCtrl* = *(PlayerCtrl + 0x3e0)        // also vtable slot 0x130 of PlayerCtrl
applySpEffect(ChrSpEffectCtrl*, const SpEffectRequest*)   at 0x14014bec0

SpEffectRequest
  +0x00 i32  SpEffect id (an event id in one of the SpEffect*.emevd files)
  +0x04 i32  1 at every call site read
  +0x08 f32  -1.0 at every call site read (constant at 0x1410ac6a4)
  +0x0c u8   slot index into the table at 0x141571490 (25 for key 0)
  +0x0d u8   1, 2 or 3 depending on the caller
  +0x0e u8   0
  +0x0f u8   bit 0 is copied; every caller clears it
```

## 1. The apply function

### Address and arguments

- **Verified in binary.** `applySpEffect` is `0x14014bec0`. Its entry is `e9 1c 0d 9f 01`, a jump
  into Arxan's section; the stolen prologue at `0x141cf8f8f` and `0x141b72fc9` saves `rbx`/`rdi`,
  does `sub rsp,0x30`, moves `rcx` into `rdi` and `rdx` into `rbx`, and rejoins game code at
  `0x14014bed4`. So `rcx` is the SpEffect controller and `rdx` points at the request. No call site
  below reads `rax` afterwards.
- **Verified in binary.** The body, followed through the Arxan hops:
  1. `0x14014bed4` calls `0x14023e160` with `rcx = rdx + 0xc`. That helper (`0x141c369f9`) returns
     `0x141571490 + 2 * *(u8*)rcx`, a pointer into the slot table described below.
  2. `0x141afc209` compares the first byte of that table entry with `0x7f`.
  3. Not `0x7f` (`0x14014bf33`): the request is copied to the stack unchanged and
     `0x14022eb30(*(ctrl + 0x10), &copy)` is called at `0x14014bf67`.
  4. `0x7f` (`0x14001f5e2`): the slot is resolved from the owner. `0x14023c600(*(ctrl + 0x8), &out)`
     returns a slot index; the request is rebuilt with that index at `+0x0c`, the other fields
     copied, and bit 0 of `+0x0f` taken from the caller; then the same `0x14022eb30` call.
- **Verified in binary.** The request ends at `+0x10`. Step 3 copies the dwords at `+0x0`, `+0x4`,
  `+0x8` and `+0xc`; step 4 reads `+0x0`, `+0x4`, `+0x8` and the bytes `+0xd`, `+0xe`, `+0xf`.
- **Verified in binary.** `+0x00` is the SpEffect id. `bonfireRestSpEffect` writes `110000010`
  there, and section 3 finds that exact id as an event in `SpEffectWideUse.emevd`.
- **Inferred.** `+0x04` is a count or stack amount and `+0x08` a duration override where `-1.0`
  means "use the effect's own". Every call site read writes `1` and the float at `0x1410ac6a4`,
  which is `0xbf800000` (`-1.0`); `assignPhantomProperties` writes `r14d` to `+0x04`. The worker
  `0x14022eb30` was not traced far enough to confirm what either field does.
- **Verified in binary, meaning unknown.** `+0x0d` is `2` from `bonfireRestSpEffect`,
  `assignPhantomProperties` and `0x140418c3e`; `1` from `0x14023c540`; `3` from `0x1402d4a1b`.
  `+0x0e` is `0` everywhere. Every caller clears bit 0 of `+0x0f`
  (`and byte ptr [rsp+..], 0xfe`).

### The slot byte at `+0x0c`

- **Verified in binary.** `+0x0c` is an index, not a key. The table runs from `0x141571490` to
  `0x1415714d4`, two bytes per entry. The first byte of each entry is a key, in index order:
  `ff fe fd fc fb fa f9 f8 f7 f6 f5 f4 23 22 21 20 1f 1c 1b 1a 19 17 16 15 14 00 12 18 1d 1e 13 12 11 7f`.
  The second byte is `0` or `1`; what it means is unknown.
- **Verified in binary.** Callers do not write the index directly. They call `0x14023e170(key)`
  (Arxan-redirected, body at `0x141cf3b01`), which scans the table for the key and returns its
  index, or `0` when it is absent. Key `0` is index `25`, key `0xff` is index `0`, key `0x7f` is
  index `33`, the "resolve it from the owner" case above.
- **Verified in binary.** Callers pass key `0` (bonfire, phantom setup, `0x14023c4e0`,
  `0x140418c08`) and key `0xff` (`0x1402d49e4`). A mod that copies the bonfire call writes `25`.

### How the game calls it

- **Verified in binary.** `bonfireRestSpEffect` (`0x1402027b0`) is the smallest complete caller.
  It takes a `PlayerCtrl*`, builds the request on the stack
  (`id = 110000010`, `+0x4 = 1`, `+0x8 = -1.0`, `+0xc = index_of(0)`, `+0xd = 2`, `+0xe = 0`,
  bit 0 of `+0xf` cleared), calls `PlayerCtrl` vtable slot `0x130` at `0x1402027f8`, and passes
  the result as `rcx` to `applySpEffect` at `0x140202806`.
- **Verified in binary.** `FUN_1402b8ee0` calls it with the local player:
  `mov rax,[0x1416148f0]; mov rcx,[rax+0xd0]; test rcx,rcx; je ...; call 0x1402027b0` at
  `0x1402b8f19`..`0x1402b8f34`. That is the whole path a mod needs, written by the game.
- **Verified in binary.** `FUN_14023c4e0` is a clean, non-redirected helper taking an owner block
  and an id in `edx`. It reads the character from `owner + 0x8`, calls its vtable slot `0x130`,
  builds `{id, 1, -1.0, index_of(0), 1, 0}` and calls `applySpEffect` at `0x14023c547`. It returns
  early if `owner + 0x18` is non-zero.
- **Verified in binary.** Other callers with a literal id: `0x1402d4a0b` writes `62170010`
  (key `0xff`, `+0xd = 3`) and `0x140418c2e` writes `91320030` (key `0`, `+0xd = 2`).
  The rest of the callers are the xrefs to `0x14014bec0`.

### Hooking and Arxan

- **Verified in binary.** None of the addresses above is in `ds2_rva::ARXAN_REDIRECTED_DO_NOT_HOOK`
  (`0x00832cb0`, `0x00c2c9e0`).
- **Verified in binary.** `applySpEffect`, `0x14023e160`, `0x14023e170` and `0x14023c600` all
  begin with a jump into Arxan's section. Calling them is ordinary: the chain runs the stolen
  prologue and rejoins. A mod that only calls `applySpEffect` needs no hook on it at all.
  `docs/ARXAN-PROBE.md` covers detouring its entry, which was run and survived.

## 2. Reaching the local player's controller

- **Verified in binary.** The argument is not `PlayerCtrl`. It is the object at
  `PlayerCtrl + 0x3e0`.
  - `PlayerCtrl`'s vtable (`0x1410e4bb8`) slot `0x130` holds `0x140312b80`, a jump to
    `0x1405ba9d0`: `mov rax,[rcx+0x3e0]` followed by a disguised `ret`. `CharacterCtrl`'s vtable
    (`0x1410df218`) holds the same pointer in the same slot, so every character has the field.
  - `assignPhantomProperties` (`0x140312dd0`, `rdi = this`) passes `[rdi+0x3e0]` straight to
    `applySpEffect` at `0x140314366`..`0x140314382`, without the vtable call.
  - The same function creates the object: it allocates it at `0x14031385f`, constructs it with
    `rdx = this + 0x3b8` at `0x140313873`, and stores it with `mov [rdi+0x3e0],rax` at
    `0x1403138a3`.
- **Verified in binary.** The constructor (`0x14014b8b0`, body `0x141b7455a`) writes the vtable
  `0x1410bfee0`, `+0x8 = owner`, `+0x10 = 0`, `+0x18 = 0`. `0x1410bfee0` is the vtable whose
  complete-object locator (`0x1412b35b8`) names `.?AVChrSpEffectCtrl@@`, so the object is a
  `ChrSpEffectCtrl`. `+0x10` is filled in afterwards (`0x14014bc20` is called on it at
  `0x1403138bd`) and is what `applySpEffect` hands to the worker.
- **Verified in binary.** The owner at `CharacterCtrl + 0x3b8` is a small block initialised by
  `0x14023c290`: `+0x8 = character`, `+0x10 = r8`, `+0x18 = 0`. It has the shape
  `FUN_14023c4e0` expects.
- **Verified in binary.** So, for the local player:

  ```text
  gm   = *(u64*)0x1416148f0            // null on the title screen
  pc   = *(u64*)(gm + 0xd0)            // null until a character is loaded
  ctrl = *(u64*)(pc + 0x3e0)           // null if construction failed
  applySpEffect(ctrl, &req)
  ```

  Reading the field or calling slot `0x130` are equivalent; the vtable call is what the game does.
- **Inferred.** Every link can be null and the game checks each one (`0x1402b8f2f`,
  `0x1403138aa`). A mod should do the same and skip the frame rather than fault.

## 3. The SpEffect id space

### There is no SpEffectParam

- **Verified in the regulation.** `enc_regulation.bnd.dcx` has no `SpEffectParam.param`, and the
  executable has no string naming one: the only `SpEffect` strings are the `.emevd` names below
  and RTTI. None of the param-name tables that build `param:/%s.param`
  (`getCharacterManagerParamNameFromIndex` `0x14048b620`, and the tables at `0x14048ba80`,
  `0x14048b480`, `0x14048b510`, `0x14048b570`, `0x14048bf90`) lists a SpEffect param.
- **Verified in the regulation and the binary.** SpEffects are defined as events in event scripts
  inside the regulation. `FUN_14023c250` loads them in this order, each into its own slot of the
  object at `[param_1 + 0xb0] + 0x8`:

  | Slot | File | Event ids |
  | --- | --- | --- |
  | `+0x08` | `SpEffectAbnormalState.emevd` | 900000 .. 902400 |
  | `+0x18` | `SpEffectWeapon.emevd` | 1140000 .. 11750000 |
  | `+0x28` | `SpEffectArmor.emevd` | 21140100 .. 27830101 |
  | `+0x38` | `SpEffectSpell.emevd` | 31070000 .. 35300014 |
  | `+0x48` | `SpEffectRing.emevd` | 40010000 .. 42000000 |
  | `+0x58` | `SpEffectPassiveItem.emevd` | 50880000 .. 51000000 |
  | `+0x68` | `SpEffectActiveItem.emevd` | 60010000 .. 65290000 |
  | `+0x78` | `SpEffectEnemy.emevd` | 90000010 .. 98830010 |
  | `+0x88` | `SpEffectWideUse.emevd` | 100000000 .. 170100000 |
  | `+0x98` | `SpEffectCondition.emevd` | 600 .. 789 |
  | `+0xa8` | `SpEffectMustSub.emevd` | 850 .. 931 |

  The id ranges are the lowest and highest event ids `scripts/ds2-emevd.py info` reports; the
  ranges are sparse, so an id inside one is not necessarily an event.
- **Inferred.** `param_1` of `FUN_14023c250` is `GameManagerImp`. It is called from the game-state
  function at `0x1401be130` with that function's own `rcx`, which the Ghidra project types as
  `GameManagerImp*`. Nothing here confirmed the type independently.
- **Verified in binary.** The game's own id checks line up with these bands:
  `0x14023d270` returns true for `90000000 <= id <= 99999999` (the enemy band), and `0x14022fb20`
  treats `600 <= id <= 799` (the condition band) specially.

### How an id reaches its event

- **Not located.** The function that maps an id to its event inside those resources was not
  found. It is past `0x14022eb30`, which is Arxan-threaded and was followed only as far as its
  first checks (`0x140228860`, `0x140228820` and `0x14022fb20`, which test the id against
  hard-coded values and bands). Knowing it is not needed to call `applySpEffect`; it matters only
  for rejecting an unknown id before the game sees it.

### Ids that exist

Each id below is an event in the file named, read with `scripts/ds2-emevd.py events --id`.

| Id | File | What it is |
| --- | --- | --- |
| `110000010` | `SpEffectWideUse` | Resting at a bonfire. **Verified in binary**: the literal in `bonfireRestSpEffect`. |
| `62170010` | `SpEffectActiveItem` | Applied at `0x1402d4a0b`. **Inferred**: Seed of a Tree of Giants, whose item id is `62170000`. |
| `91320030` | `SpEffectEnemy` | Applied at `0x140418c2e` after a flag test on `[r14+0x81]`. What it does is unknown. |
| `40010000` | `SpEffectRing` | **Inferred**: Life Ring, item id `40010000`. |
| `31070000` | `SpEffectSpell` | **Inferred**: Homing Soulmass, spell id `31070000`. |
| `60010000` | `SpEffectActiveItem` | **Inferred**: Lifegem, item id `60010000`. |

- **Inferred.** Weapon, armour, ring, spell and item effects share the id of the thing that
  grants them, with a small suffix for variants (`62170000` and `62170010` both exist). The names
  come from `crates/ds2-build-import-core/data/items.tsv`, which is second-hand
  (see `scripts/ds2-item-ids.py`). Event scripts carry no names, so nothing in the game itself
  says what an event is for; the instructions are there to read, but their bank/index meanings are
  not mapped.

## 4. Whether an applied effect is sent to other players

Yes. In an online session, `applySpEffect` on a character this machine owns (the local player
included) sends a SpEffect-sync packet for each effect it adds. Offline, nothing is sent. Nothing
in the request turns the send off; only the id and the character decide it.

### The packet

- **Verified in binary.** Packet id `0x31` (49) is the SpEffect sync. `FUN_14051e3d0` allocates a
  `NetP2pPacketSpEffect` (vtable `0x1410fb6c8`), stores it at `+0x48` of the packet table reached
  through `BaseB + 0x8` (`BaseB` is the global at `0x141616cf8`; `0x140513230` returns that
  field), and registers it with the `NetSessionManager` under id `0x31`. Its vtable slot `0x18`
  (`0x14025d0a0`) sends through `NetSessionManager` slot `0x78` (to the whole session) or
  `0x70` (to one player); slot `0x20` (`0x14025cef0`) is the receive handler.
- **Verified in binary.** `0x14051e710(table, op, kind, handle, id, type, duration, via_host)`
  builds the packet and calls that sender:

  ```text
  +0x00 u32  bits 0-1 kind (0 player, 1 enemy), bits 2-3 op (0 add, 1 remove),
             bit 31 set when an enemy effect is sent to the host for it to forward
  +0x04 u32  who: the session player number (kind 0) or the character's +0x110 id (kind 1)
  +0x08 i32  SpEffect id
  +0x0c i32  a second per-effect value (called type here; meaning unknown)
  +0x10 f32  remaining duration
  ```

  With `via_host` set and this machine not the host (`[NetSessionManager + 0xa4] != 2`), only an
  enemy packet is sent, and only to the host; the host's receive handler clears bit 31 and
  forwards it to the session (`0x14025cfda`..`0x14025cff0`).

### The send inside applySpEffect

- **Verified in binary.** The worker `0x14022eb30` (reached from `applySpEffect` as section 1
  describes; `rbx` is the object at `ChrSpEffectCtrl + 0x10`, `rdi` the request) first decides
  whether the effect is synced, in `r15b`:
  1. `0x140228860(id)` true (a hard-coded list; every constant read is in the enemy band,
     `95146040`..`98830000`): not synced.
  2. else `0x14023ca50(owner)` true (a lookup on the character's kind in the table at
     `0x1410bfff3`): synced.
  3. else `0x140228820(id)` true (`40500000`, `5400000`, `21210101`, `22510100`, `22510101`,
     `40550000`, `40620000`): not synced.
  4. else synced.
- **Verified in binary.** A synced effect must pass `0x140228790(character)` or the whole call
  returns `0` and applies nothing. It passes when there is no net manager
  (`[GameManagerImp + 0x22f0]` null), when that manager's byte `+0x38` is `0`, when the character
  is a player whose `[[chr + 0xe8] + 0x28]` is non-zero, or when it is an enemy whose entry in
  `EnemyGeneratorManager` (`0x140419ab0`) has bit 0 of `+0x42` set.
- **Verified in binary.** `+0x04` of the request is a repeat count: the body runs while
  `esi < [rdi + 4]` (`0x141afc3ee`), and returns at once when it is `<= 0` (`0x141beb005`). Each
  pass adds the effect with `0x14022fe50` and, if synced, calls `0x140228f60` at `0x14022ecdd`
  with `(character, id, type, duration)`, but only when all of these hold: `[owner + 0x18]` is
  zero (`0x14023cb50`), `0x14022fe50` returned a non-zero handle, and the duration from
  `0x14022e820` is `>= 0`. That duration is the larger of the effect's own and `+0x08` of the
  request, so `+0x08 = -1.0` means "the effect's own".
- **Verified in binary.** `0x140228f60` returns without sending when `[GameManagerImp + 0x22f0]`
  is null or `0x140513610` is true. `0x140513610` is true unless `BaseB + 0x18` reports an active
  session (`0x1402c6eb0` or `0x1402c6d80` non-zero). It repeats the ownership test above, then:
  for a player-type character (`0x140203be0`, table byte `0x1410bfff1 == 0`) it looks up the
  session player number with `0x14051b5d0`, which returns `[table + 0x174]` when the character is
  `GameManagerImp->PlayerCtrl`, and sends `op 0, kind 0, via_host 0`; for any other character it
  sends `op 0, kind 1, who = [chr + 0x110], via_host 1`.
- **Verified in binary.** Removals go the same way: `0x1402290d0` runs the same session and
  ownership tests and sends `op 1` for each removed entry.
- **Inferred.** For the local player online, step 4 applies to the bonfire id `110000010` and to
  any id outside the two lists, `0x140228790` passes because `[[chr + 0xe8] + 0x28]` marks a
  locally controlled player, and the packet goes to the whole session. The meaning of
  `[chr + 0xe8] + 0x28` and of the manager's `+0x38` was not traced.
- **Not traced.** Whether `0x14022fe50` rejects some requests on `+0x0d`, `+0x0e` or `+0x0f`
  (which would return a zero handle and so skip the send). The worker itself never reads those
  bytes; it passes the whole request to `0x14022fe50`.

### Receiving an effect

- **Verified in binary.** The handler `0x14025cef0` drops a packet whose length is not the full
  layout above, and one with `kind >= 2`. Op `0` calls `0x140228a30`, op `1` calls `0x140228c00`.
  For a player (`kind 0`) the character looked up by `who` (`0x14051b3c0`) must be the one that
  sent the packet (`0x14051b380`), so a player can only sync effects on their own character. If
  the character is not loaded yet the effect is queued (`0x1402c8040`) and later replayed by
  `0x140228dc0`.
- **Verified in binary.** A received add does not go through `applySpEffect`. It is applied only
  when `0x140228790(character)` is false, that is, on a character this machine does not own, by
  `0x14014bcc0` (`ChrSpEffectCtrl`, id, type, duration), whose body `0x14022e890` builds its own
  request with bit 0 of `+0x0f` set and calls the same `0x14022fe50`. It never calls
  `0x140228f60`, so a received effect is not sent again. Bit 0 of `+0x0f` therefore marks an
  effect that came from the network; every local caller clears it.
- **Inferred.** So a mod that calls `applySpEffect` on the local player's controller while online
  broadcasts the effect to everyone in the session, and each of them applies it to their copy of
  that player. Keeping it local needs a hook: on `0x140228f60`, on the sender `0x14051e710`, or
  on the check `0x140513610`, and not a flag in the request.

## Still unknown

- What `+0x0d` and `+0x0e` of the request mean, and what `0x14022fe50` does with them.
- Whether other players actually see an effect sent this way. The send path is read from the
  binary; nothing was run with two machines.
- Which thread the game calls `applySpEffect` on, and whether calling it from a hook on another
  thread is safe.
- The id-to-event resolver, and what the game does with an id that has no event.
