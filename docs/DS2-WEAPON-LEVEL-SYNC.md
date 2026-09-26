# Weapon level sync in multiplayer: feasibility

The feature: while we are in someone's world (invading is the main case) or they are in ours,
limit our weapons to the highest weapon upgrade any other player in that world has equipped.
If the host is at +0 and their phantom is at +9, the limit is 9. Our +10 weapons drop to +9, and
weapons at +9 or below stay as they are. The limit only ever lowers our own weapons. When the
encounter ends, every weapon goes back to its real level.

Everything below was read from `DarkSoulsII.exe` SOTFS (Ghidra daemon on port 8766, and the flat
dump `darksoulsii-deobf.bin`) on 2026-09-26. Nothing was run. The game was not attached, because
the running process is being held for another investigation.

- **[static]** means the claim was read in the disassembly or the decompiler.
- **[inferred]** means it follows from what was read but was not traced to the end. Each one
  says what would prove it.

## Built and run solo: `ds2-weapon-sync` (2026-09-26)

The design below is shipped as `crates/ds2-weapon-sync`, off by default, turned on with
`scripts/ds2-run.py --weapon-sync` (`--weapon-sync-test-cap N` adds a pretend remote player at
+N). Every run was offline and alone, `--no-seamless`, on slot 1 of the redirected save, with a +10
Dagger in right hand 1.

**Measured:**

- **The cap works.** With `test_cap = 3` it logged `CAPPED cap=+3 pushed=[s1:1000000+10->+3]
  after: inventory=[+0,+10] records=[+0,+3] live=[+0,+3]`. `scripts/frida/weapon-sync-read.js`
  read the same three values independently.
- **A level-only change is lost unless the item changes first.** The first build pushed the Dagger
  once at +3. The live state took it and the record table stayed at +10. The record writer
  `0x1403463d0` builds the new table, then keeps it only when the item id or the `u16` at `+0x0C`
  differs from the old record (the `u16` is the constant `1`). The level is the byte after that
  `u16`, so a same-item level change is computed and dropped. The packet 61 receiver writes a
  peer's copy of us through the same function, so a peer would keep the old level too. The crate
  now pushes Fists first and then the real item at the new level. [static + runtime]
- **The in-world restore works.** Removing `test_cap` from the live `ds2-mods.toml` logged
  `RESTORED ... records=[+0,+10] live=[+0,+10]`. A Frida read afterwards showed the same equipped
  entries, still flagged equipped (`+0x1f = 0x2`), with levels and infusions intact.
- **The save keeps the real level.** Run with the cap live on both copies and saves allowed
  (`--no-save-file-row`), the 300 s autosave rewrote `DS2SOFS0000.sl2` at 16:28:19. Decrypted
  with `scripts/ds2-sl2.py -x`, every Dagger inventory record in the slot's payload
  (`USER_DATA002`, `{u32 id, u32, f32 durability, u8 level, u8 infusion}` at `0x9570..0x95f0`)
  still reads level `0x0a`. The next launch, uncapped, logged
  `world ... built with: inventory=[+0,+10] records=[+0,+10] live=[+0,+10]`.
- **The per-frame seam runs.** `NET_SESSION_UPDATE` runs 60 times a second on the game thread in
  the world (`scripts/frida/tick-count.js`), and at the title too (`tick live player=0x0`).

**Not proven, blocked on a second player (Seamless Co-op is not installed yet):**

- reading another player's weapon levels from their record table;
- when those records are first filled after a join;
- the encounter-end trigger with a real player leaving, dying or disconnecting;
- what a peer's copy of our weapon shows after the Fists-then-item push, and whether the Fists
  step is visible to them;
- whether damage reads the capped copy (Q1).

`scripts/frida/weapon-sync-cap.js`, the Frida version of the cap, froze the game on its one
attach. Its header says why, and says not to attach it as it stands.

## Verdict: possible, with caveats

It can be built without touching the save. The game already copies a weapon's level from the
inventory into two runtime-only structures, and the same function also sends that level to every
peer. If we clamp the level in that one function, the character's equipment and what peers see
both carry the capped level, while the inventory entry that is saved keeps the real one.

The caveats:

1. **It has not been proved that damage reads the capped copy.** Static reading shows where the
   damage-side weapon record keeps its level ([Q1](#q1)). It does not show the attack-rating
   code reading it. One read watchpoint during a swing settles it.
2. **We only learn the other players' equipped weapons.** Nothing that reaches our client
   carries another player's inventory, and nothing before the join carries a weapon level at
   all ([Q2](#q2)). So the limit is the highest level among the up to six weapons each player
   has equipped, and it moves when they change weapons.
3. **The pause menu will still show real levels.** It reads the inventory entry, and we
   deliberately leave that alone.
4. **The server upload has not been traced far enough** to say whether it reports the
   inventory's level or the equipment copy's level ([Q5](#q5)).
5. **The comparison uses the raw number.** Boss-soul and unique weapons top out at +5, where
   normal weapons top out at +10. A host with a +5 boss weapon caps us at +5. Whether that is
   wanted is the user's call.

## Q1: where the level lives, and what reads it {#q1}

There are three copies of a weapon's level. Only the first one is saved.

| Copy | Where | Saved? |
| --- | --- | --- |
| Inventory entry | `ItemInventory2` bag entry (`ITEM_ENTRY_STRIDE` `0x28`), `+0x25` low nibble. `+0x1e` is the item type, `+0x26` low nibble is the infusion. **[static]** | Yes. It is the save's source. **[inferred]** |
| Equipment record table | `CharacterCtrl+0x378` (`ChrAsmCtrl`, already bound as `chr_asm_ctrl`) -> `ChrAsmCtrl+0x20` (vtable `0x1410e0a38` slot `0x60`, `0x140151390` = `mov rax,[rcx+0x20]; ret`) -> `+0x08`: `0x34` records of `0x14` bytes each. Record `+0x04` item id, `+0x08` category (weapon = 0), `+0x0C` `u16` 1, **`+0x0E` level**, `+0x0F` infusion, `+0x10` `f32` durability. Weapon slots are indices `0..5`. **[static]** | No. **[inferred]** |
| Live weapon state | `ChrAsmCtrl+0x28` (`ChrAsmEquip`, slot `0x70`) -> weapon entry `(hand*3 + index) * 0x48`: `+0x50` weapon id, `+0x58` WeaponParam row, `+0x60` the row named by WeaponParam `+0x0C`, `+0x68` WeaponTypeParam row, **`+0x70` level**, `+0x71` infusion. **[static]** | No. **[inferred]** |

**The function that copies inventory -> equipment -> network: `0x14037f890`.** It is our hook.

- `0x1401b66a0` is the inventory-side "slot changed" notifier. It has 12 call sites in
  `0x1401b0fc0..0x1401b4700`: equipping, swapping and the other inventory paths. For weapon
  slots it reads the entry's `+0x25 & 0xF` and `+0x26 & 0xF`, packs
  `{+0 slot, +4 item id, +8 durability, +0xC u8 level, +0xE u8 infusion}`, and calls
  `0x14037f890(GameManagerImp->PlayerCtrl, &req)`. **[static]**
- `0x14037f890` writes the record table through `0x1403463d0`. If the same weapon id is already
  in that slot, it also updates the live weapon state through `0x1403482a0`, which stores
  `+0x70`/`+0x71`. It then calls `ChrEquipPacket_remoteWeaponChange` (`0x140162c50`), which
  sends P2P packet `61` built from the same `req`. **[static]** Its only direct caller is
  `0x1401b66a0`. **[static]**
- Neither function goes through an Arxan redirect. `scripts/ds2-arxan-chain.py` stops at hop 0
  for both. Prologues: `0x14037f890` is `48 89 5c 24 10 48 89 74`, `0x1401b66a0` is
  `48 89 5c 24 08 48 89 74`. **[static]**

**The read point for damage has not been pinned down.** `0x14034a390` fills the live weapon
state's param rows, and `0x140348b40` rebuilds a record from `+0x70`/`+0x71`, which shows that
`+0x70` is that structure's level. The code that turns the `+0x60` row plus `+0x70` into attack
rating was not found. **[inferred]** It is the obvious candidate, since remote characters have
no inventory and still get their level through packet 61.

**How to prove it:** during a swing, put a Frida read watchpoint on the local player's weapon
entry `+0x70`, and on the record's `+0x0E` for comparison, with one thread armed as AGENTS.md
requires.

**Answer:** one function to hook, and no inventory write. When the limit changes, we re-drive
`0x14037f890` once per weapon slot `0..5` with the capped level. We build `req` from the
inventory entry exactly as `0x1401b66a0` does, so the game's own function does the work.

## Q2: what our client knows about the other players' levels {#q2}

**Nothing before the join.**

- `createMatchingParameter` (`0x1402aa540`) builds the matching parameter from these inputs,
  and no weapon level is among them **[static]**:
  - the value at `DAT_141614278`
  - soul memory (`player_param->total_get_soul_1`)
  - `player_param+0xd0`
  - the NG+ count (`0x14019ee30`)
  - the sum of bonfire intensities
  - `NetSvrProperties+0x244`
  - covenant
  - a platform or mode byte
  - an area value

  The protobuf `Frpg2RequestMessage.MatchingParameter` has 12 scalar fields. Its ByteSize is
  `0x140c74920`, and its vtable is at `0x1411113f8`. So the brief's premise is wrong for this
  binary: DS2 SOTFS matchmaking has no weapon-level factor. **[static]**
- The invasion target list entry, `Frpg2RequestMessage.BreakInTargetData` (ByteSize
  `0x140c73f60`), has one `u32` and two byte strings. It carries no stats. **[static]**

**After the join: every remote player's equipped weapons, with levels.**

- The receiver `ChrEquipPacketReceiver` (vtable `0x1410c1680`, dispatch `0x140162150`),
  case `0x3d` (61), takes an 8-byte packet: `u32` item id, then a byte (`slot & 7`, plus bit 3
  = "durability <= 0"), then a byte (**level in the low nibble**, infusion in the high nibble).
  It accepts the packet only if the slot is below 6, the level is below `0xb` and the infusion
  is below 10. It then writes the level into that remote character's record table through the
  same `0x1403463d0`. **[static]**
- So a remote player's weapon levels can be read from
  `[[remote PlayerCtrl + 0x378] + 0x20] + 0x08 + slot*0x14 + 0x0E` (`& 0xF`) for slots `0..5`,
  or from `[[... + 0x378] + 0x28] + 0x70 + n*0x48`. **[static]**
- `ds2-invasion-path/src/census.rs` already lists the remote players: the `PlayerCtrl` vtable
  test plus `is_person`. **[static]**
- **When those records are first filled is not known.** The remote `PlayerCtrl` is built in
  `0x1403572e0` (the `NetworkPlayer_%06u` factory) from a per-player slot at
  `GameManagerImp+0x650`. The packet or struct that dresses a new remote character at join was
  not traced. **[inferred]** It must carry the equipment, because the character appears
  armed. **To prove it:** read the remote record table's `+0x0E` bytes once a remote player's
  roster line appears.
- Empty hands are item `3400000` (Fists, per `0x14034a390`) at level 0. With a highest-level
  rule they never raise the limit, and they do no harm.

**Stand-in if the join data turns out to be empty:** there is none from matchmaking. The
fallback is "no limit until a packet 61 arrives", or a fixed limit. Neither is good.

## Q3: detecting when an encounter starts and ends {#q3}

What the repo already has:

- `ds2_rva::NET_SESSION_BUSY` (`0x14025f690`), as used by `ds2-menu-row/src/session.rs`. It
  returns true when the session state from `0x14025ed80` is 1 or 2. Those states come from the
  counts at `0x140513150` and `0x140513130` (lists under `BaseB+0x18`) and from
  `0x140253c10(session+8)`. **[static]** This is what the shipped Quit Game row uses to lock
  itself in multiplayer.
- `ds2-invasion-path/src/census.rs` `remotes()` returns the remote players who are people. It
  leaves out bloodstain replays (phantom params `0x12`/`0x13`) and NPCs. **[static]**, and
  proven live in that crate.
- The `ds2-net-effects` branch (not on main) adds the SpEffect send path. It has no session
  events. The `er-net-effects` crate is ELDEN RING code and does not apply.

**Phantom type table [static]:** `DAT_1410c0050`, 20 entries of `0x10` bytes each, indexed by
the phantom param id at `PhantomBlock+0x3c`.

- Entries `0x12`/`0x13` have byte 1 = 0. Those are exactly `REPLAY_PHANTOM_PARAM_IDS`, which
  confirms the indexing.
- Byte 2 is 1 for entries 1-4, 7, 9 and 13, and 2 for entries 5, 6, 8, 10-12 and 14-17.
  **[inferred]** Byte 2 is the side (1 = cooperator, 2 = hostile). Checking it needs a live
  invasion.

The session object also keeps 5 per-player records of `0x4c` bytes each at `session+8 -> +0x10`.
Each record's `+0x08` byte indexes the same table. **[static]**

**Proposed signals.** All are read on the game thread, and the one that reads the level is
polled once per frame:

- **Start:** `NET_SESSION_BUSY` is true, and the roster contains at least one remote person.
  The local player's own `PhantomBlock+0x3c`, with byte 2 of its table entry equal to 2, marks
  us as the invader. **[inferred]** That the local player's phantom param is set while
  invading was not traced.
- **The limit changes:** recompute it whenever any remote record's level byte changes. This
  covers packet 61 (a weapon swap) and players joining or leaving.
- **End:** the roster has no remote person, or `NET_SESSION_BUSY` is false. An invader going
  home (after dying, a kill, or a disconnect) reloads the world. The reload rebuilds equipment
  from the inventory, so the real levels come back without us doing anything. **[inferred]**
  The rebuild path on load was not traced.

## Q4: getting the real levels back {#q4}

We never write the inventory entry. Its `+0x25` is what `0x1401b66a0` reads and what the save
mirrors, and the save does not see the equipment record table or the live weapon state.
**[inferred]** Nothing was found that writes either of them back to the inventory or to
`SaveDataItemInventory2` (vtable `0x1410da418`), but that was not proved absent either.

So a crash, a disconnect, a quit-out or a death cannot leave a capped level in the save.
Autosave during the session saves real levels.

Ending without a world load (a phantom leaving our world while we host) needs one explicit
re-drive: `0x14037f890` for slots `0..5` with the limit removed. That also sends packet 61 with
the real levels.

**To prove it:** limit a +10 to +3, force an autosave (take an item from a corpse), quit to the
title, and reload. The inventory must show +10.

## Q5: online risk {#q5}

What the binary shows is sent:

- **To peers (P2P):**
  - Packet `61`, the weapon change: item, slot and 4-bit level. The receiver's only checks are
    level <= 10 and infusion < 10. It compares against nothing else. **[static]** A capped level
    looks exactly like a legitimately lower-upgraded weapon.
  - Packet `60` (`0x140162d00`, 12 bytes) sends only a fixed item id (`6000000` or -1) and a
    flag. **[static]**
  - Packet `64` (`0x140162b20`) sends a length-prefixed `u32` list into `+0x588` of the equip
    object. What it lists was not identified. **[inferred]** It is not the weapon level: it is
    written to a different object than the record table.
- **To the server:** `Frpg2ClientLib::PlayerStatusSyncManager` (`0x1406af5f0` builds it; the
  diff-and-send is `0x1406b01f0` -> `requestUpdatePlayerStatus` `0x1406b21d0`). It uploads a
  diff of `Frpg2PlayerData.AllStatus`, which contains:
  - `WeaponStatus`: 6 `u32` fields, ByteSize `0x140cc9050`.
  - `EquipmentInfo`: 6 repeated varint fields, ByteSize `0x140cc7b60`.

  **[static]** The game-side code that fills these messages was not found. It reaches the
  upload through `Frpg2PlayerImpl` vtable `0x141111cb8` slot `0x50`. So it is **unknown**
  whether the server receives a weapon level at all, and if it does, whether that level comes
  from the inventory (real) or from the equipment copy (capped).
- The server's own checks cannot be read from the client binary. Nothing above says how
  `EquipmentInfo`/`WeaponStatus` are used server-side.

**Risk as far as the binary shows:** the design never writes the item data the server knows
about, which is the inventory and the save. What peers see is an ordinary, valid level. The one
thing still open is the `AllStatus` source, and it should be traced before the feature ships:
find the function that writes `WeaponStatus`'s has-bits (`+0x40`) and follow it back.

## Proposed design

1. Detour `0x14037f890`. When a limit is active and `req.category` is a weapon (`FUN_1401544d0`
   kind `1`), set `req[+0xC] = min(req[+0xC], cap)` and call the original. Every weapon update
   for the local character then carries the cap, and so does every packet 61 we send.
2. Poll once per frame on the game thread: session busy, the census roster, and each remote's
   record-table levels for slots `0..5`. When the recomputed limit changes, re-drive
   `0x14037f890` for our slots `0..5`, building each `req` from our inventory entry as
   `0x1401b66a0` does.
3. When the session ends: clear the limit and re-drive once, or do nothing if a world load is
   already rebuilding the equipment.
4. Every address goes into `ds2-rva` as a named constant before any feature code uses it.
5. Before any of this is DLL code, prototype it in Frida (AGENTS.md: Frida first). One hot
   reload settles both Q1's read point and Q2's join-time fill.
