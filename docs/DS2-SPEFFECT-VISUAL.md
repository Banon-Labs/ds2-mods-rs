# SpEffects that only draw something

Which SpEffect ids a mod can apply to the player for a visible effect that changes nothing else:
no HP, stamina, stats, damage, defence, speed, status, items or souls. Written to pick a
replacement for the bonfire-rest id `110000010`, which heals, refills and repairs.

Read statically from the regulation and `DarkSoulsII.exe` (SOTFS, image base `0x140000000`).
Nothing was run. Claims are tagged **verified** (read from the file or binary named) or
**inferred**.

## There is no SpEffectParam to read

- **Verified.** `enc_regulation.bnd.dcx` has no `SpEffectParam.param`; its members were listed
  with `scripts/ds2-regulation.py extract`. A SpEffect id is an event in one of the eleven
  `SpEffect*.emevd` members, and what the effect does is the instruction list of that event
  (the branch `speffect-static` has the longer write-up in `docs/DS2-SPEFFECT.md`).
- So "neutral" cannot be checked field by field. It is checked instruction by instruction: an
  effect is purely visual when every instruction in its event is a drawing or bookkeeping
  instruction.

## Method

`scripts/ds2-speffect-visual.py <extracted regulation dir>` reads every SpEffect event with the
parser in `scripts/ds2-emevd.py` and keeps an event only when:

- every instruction is one of these, and
- at least one of them draws something.

| Instruction | Name (DarkScript3) | Class |
| --- | --- | --- |
| `100040[1]` | Spawn SFX (SFX ID, four `u8`, one `f32`) | draws |
| `100040[8]` | Apply ChrFullBodySFX (ChrFullBodySfxParam id, `u8`) | draws |
| `100700[4]` | Set SpEffect Length (`f32` seconds) | bookkeeping |
| `100130[2]` | Display Icon | rejected unless `--allow icon` |
| `100130[4]`, `100130[5]` | Apply Weapon SFX, Draw Disable Full Body SFX | rejected unless `--allow weapon-sfx` |

Every other `(bank, index)` disqualifies the event. That covers every stat, damage, defence,
movement, stamina, heal, status, equipment, bullet, soul, warp, multiplayer and sub-event call
instruction in DarkScript3's `ds2scholar-common.emedf.json`, and every "Unknown Command", so an
unknown instruction never passes as harmless.

- **Inferred.** The instruction names and argument layouts are DarkScript3's community names, not
  From's. The script checks that each decoded layout packs to exactly the instruction's stored
  argument length, so the layouts at least fit the bytes.
- **Verified.** The two accepted drawing instructions carry no amount, duration or target that the
  game could apply to stats: `100040[1]` is an sfx id, four bytes (the first is a dummy-poly id:
  `249`, `220`, `20`, `21` in the hits below) and a float that is `0` in every hit.

For contrast, the bonfire event `110000010` holds a MustSub heal call (`100050[1]`, sub-event
`851`, `100.0`), `100110[2]` Repair Equipment, `100060[0]` Restore Spell Usage (`100.0`), a run of
`100070[0]` status clears (sub-event `871`), `100090[8]` and several `1000[1]` Modify Stat
Multiplicatively.

## Result

20 of the 899 SpEffect events qualify. Adding `--allow icon --allow weapon-sfx` adds none.

Besides the event itself, each id was checked for anything else that keys on it:

- a 32-bit literal of the id in `darksoulsii-deobf.bin` (code that treats the id specially),
- a reference to the id in any regulation param (what normally applies it),
- whether its sfx id is a member of the resident bundle `Game/sfx/sfx9999.ffxbnd.dcx`
  (`scripts/ds2-ffx.py list --id`).

### Recommended

| Id | File | Instructions | Length | Sfx in `sfx9999` | Applied by | Literal in exe |
| --- | --- | --- | --- | --- | --- | --- |
| `140001010` | WideUse | Spawn SFX `659`, dummy poly `249` | 1.5 s | yes | `SystemDamageParam` row `310070030`, `+0x48` | none |
| `120000310` | WideUse | Spawn SFX `659`, dummy poly `249` | 1.5 s | yes | `SystemDamageParam` row `200014040`, `+0x48` | none |
| `32170011` | Spell | Spawn SFX `5216`, dummy poly `249` | 2 s | no | `PlayerDamageParam` row `32170011` and `BulletParam` `32170000`, `72170000` (Soul Appease, inferred) | none |
| `21470120` | Armor | Spawn SFX `5425`, dummy poly `249` | 3 s | no | `PlayerDamageParam` row `50000010` | one unexplained hit at `0x141937f40`, in data |
| `120000010`, `120000020`, `120000030`, `120000110`, `150101000` | WideUse | Spawn SFX `3739`, dummy poly `249` | 0.5 s | no | `SystemDamageParam` rows `200018010`, `200018020`, `200018030`, `210210110`, `320030000` | none |

- **Verified.** `140001010` is the first choice: a single Spawn SFX plus a 1.5 s length, its effect
  file `f0000659.ffx` is in the resident bundle, no code in the executable names the id, and it
  sits in the WideUse band like the bonfire id. `120000310` is the same event under another id.
- **Inferred.** What sfx `659` looks like is not known; nothing here renders it. The same sfx is
  used by the enemy effect `96720020`, and in `SystemDamageParam` these ids sit in the `+0x48`
  field of damage rows, so they are probably hit sparks on the struck character. Dummy poly `249`
  is used by most hits and is probably a point on the body.
- **Inferred.** An sfx that is not in `sfx9999` has to come from a bundle loaded for the current
  area or character. It may not draw everywhere, which is why the `5216`, `5425` and `3739` rows
  rank lower. Where those bundles live was not looked up.

### Avoid

| Id | Why |
| --- | --- |
| `62170000` | Seed of a Tree of Giants (inferred from the item id). Its event is Spawn SFX plus a 3600 s length, but the executable holds the id as a literal (bytes at `0x1401e6a20` and `0x1402d49c0`), and the item's AI effect is not in the event at all. |
| `60405030`, `60406030` | Dragon Head and Torso Stone full-body sfx (inferred). The executable holds both ids as literals (`mov ebx` at `0x1401ac297`; bytes at `0x1401ac26f`). |
| `901100` | AbnormalState band, full-body sfx param `400010`, no length (may not expire). Code loads it at `0x140145dbb` (`mov r14d, 0xdbfec`). |
| `96270000`, `96720020` | Enemy band. Both are in the id list at `FUN_1402289e0` (verified; its caller is inside the Arxan-threaded `0x14022fb20`, meaning not traced). `0x14023d270` also special-cases the whole 90000000..99999999 band. |
| `91060010`, `96560400`, `96560410`, `96560420` | Enemy band (see above); lengths 60 s and 5 s. |
| `3530000` | Bone Fist weapon effect, sfx on dummy polys `20`/`21` (weapon points, inferred), 0.95 s. Tied to a weapon model. |

## Network

- **Verified in binary, from `docs/DS2-SPEFFECT.md` on `speffect-static`.** An id is kept local only
  when it is in one of two hard-coded lists (`0x140228860`, enemy band only; `0x140228820`,
  `40500000`, `5400000`, `21210101`, `22510100`, `22510101`, `40550000`, `40620000`). None of the
  recommended ids is in either, so applying any of them to the local player in an online session
  sends a SpEffect-sync packet (`0x31`) to the session, as the bonfire id does, and other players
  apply it to their copy of the player. For a purely visual effect that is harmless, but it is not
  local.

## Not checked

- What any of these effects looks like in game.
- Whether the damage-param rows that normally apply these ids do anything to the SpEffect beyond
  naming it.
- The meaning of the three other bytes and the float of Spawn SFX.
