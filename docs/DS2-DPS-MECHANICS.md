# DS2 SOTFS hit-trading mechanics for a weapon DPS model

Labels: **REGULATION** = decoded from `enc_regulation.bnd.dcx` (via `scripts/ds2-regulation.py`'s
`Param` class and Smithbox DS2S paramdefs; every decoded param's def size equals its measured
stride). **TAE** = decoded from the player's `/timeact/chr/c000100_pl.tae` in `GameDataEbl`.
**EXE** = read from `darksoulsii-deobf.bin` disassembly (`scripts/ds2-disasm.py`,
`ds2-xrefs.py`, `ds2-rtti-vtables.py`). **INFERRED** = consistent with the data and the
Japanese field names, but the consuming code was not traced. **COMMUNITY** = wiki claim.
Nothing was launched.

Data artifacts (scratchpad, not repo):
- `dps/decoded.json`: decoded rows of WeaponParam, WeaponActionCategoryParam,
  WeaponAttackMotionParam, PlayerDamageParam, DamageCtrlParam, WeaponTypeParam,
  WeaponStaminaCostParam, ArmorParam, PoiseDamageParam, HitBack*, DamageStyleRate, DamageAttackTypeRate, and others.
- `dps/attacks.json`: 26,669 rows. Each row is one (weapon, attack slot) with its motion row, anim id,
  play speeds, and every TAE hit event (window, damage row, motion value, repeat interval, max hits).
- `tae/c000100_pl-tae/anim-*.xml`: the player TAE unpacked by WitchyBND (1138 animations).

## 0. The chain from a weapon to its attacks (REGULATION)

```
WeaponParam[weaponId].weaponActionCategoryId
  -> WeaponActionCategoryParam[id].atkId<Grip><Kind>      (66 slots: Single1Hand/Single2Hand/Dual*/Left*,
                                                          Normal1st/2nd, Strong1st/2nd, Dash, Rolling, Step,
                                                          Down, Fall, Jump, GuardBreak, Opposite*, stabs)
  -> WeaponAttackMotionParam[atkId]  attackAnim, controlType, startPlaySpeed, endPlaySpeed,
                                     damageId01..03, bulletId01..03 + bulletDamageId01..03
  -> PlayerDamageParam[damageIdNN]   damageRate (motion value), staminaDamage (to a blocker), stiff,
                                     hitDistance (repeat-hit interval), childDamage, damageGroup, hitbox shape
  -> DamageCtrlParam[damageCategory] poiseDamage, negatePoiseRate, armorBreak, damageMotion, stun,
                                     guardAttackLevelType, distance falloff
attackAnim -> c000100_pl.tae anim-<attackAnim>: event 2200 {ID=n} = hitbox window using damageId0(n+1)
```

A slot value of `1` means the slot is unused (for example, guard attacks on a dagger).

## 1. Motion values and per-attack data

- **Motion value = `PlayerDamageParam.damageRate`** (JP dame-ziBei Lu , "damage multiplier"). REGULATION for the
  values. That it multiplies AR is INFERRED from the name and the values: 2H versions are exactly 1.2x
  the 1H ones (dagger R2 1.176 -> 2H R2 1.4112).
- `styleRateType` indexes DamageStyleRateParam: 0 = 1.0, 1 = 1.5, 2 = 0.8. 2H rows carry 1.
  `attackTypeRateType` indexes DamageAttackTypeRateParam: 1.0/1.15/1.3/1.5; R2s carry 1 and jump attacks 2.
  REGULATION for the values. Both multiply the hit's knockback, not its damage (EXE, "Two-handing" below).
- **Poise damage (base)** = `DamageCtrlParam.poiseDamage`, row `PlayerDamageParam.damageCategory`.
  Weapon factor = `WeaponParam.poiseDamageScalePlayer`. See section 2 for the EXE formula.
- **Stamina cost** = `WeaponParam.meleeAttackBaseCost * WeaponStaminaCostParam[costCategoryId].<slot> / 10`.
  This is from `docs/DS2-BUILD-MECHANICS.md`: the field values are REGULATION, and the /10 is INFERRED.
  See "Stamina: what an attack costs, and what a bar buys" below.
- **Stamina damage to a blocker** = `PlayerDamageParam.staminaDamage`, times `WeaponParam.staminaDamageScale`. INFERRED from the name.
- **Timing fields in the params**: `WeaponAttackMotionParam.startPlaySpeed` / `endPlaySpeed` are
  animation speed multipliers for the start and end parts of the attack (JP animeSu Du  Kai Shi Bu /Zhong Liao Bu ). The split point
  is unknown. `DamageParam.stiff` (Ying Zhi [sec]) is a duration. Whether it belongs to attacker recoil, target
  hitstun or deflect is unverified.
- **Distance falloff** (REGULATION, meaning INFERRED from JP names): DamageCtrlParam `damageDistDecrementStart/End`
  are 1.25 / 1.6 m. The max decrement rates are HP -30%, stamina -70%, stun -70%, poise -70%.
  So hits near the reach limit may deal less.

### Worked example: Dagger (1000000) and Old Knight Hammer (2730000)

WeaponParam: Dagger actcat 1000, poiseDamageScalePlayer **3.5**, menuPoiseDamage 10, uninterruptibleRate **0**,
base stamina 144. OKH actcat 2730, poiseDamageScalePlayer **16.0**, menuPoiseDamage 65, uninterruptibleRate **0.3**,
base stamina 322. In REGULATION, `menuPoiseDamage == 10 * poiseDamageScaleEnemy` for 356 of 358 weapons, so the
menu number is the PvE scale, not the PvP one.

| weapon / slot | motion row | anim | speed s/e | MV | ctrl.poiseDmg | x player scale (INFERRED) | stamina cost | hitbox (anim frames @30) |
|---|---|---|---|---|---|---|---|---|
| Dagger R1 | 10004100 | 21010010 | 1.1/1.2 | 1.00 | 10 | 35 | 13.7 | 8-15 |
| Dagger R1 chain | 10004400 | 21010020 | 2.0/1.0 | 0.925 | 10 | 35 | 15.1 | 10-14 |
| Dagger R2 | 10004600 | 21010030 | 1.4/1.4 | 1.176 | 12.5 | 43.75 | 20.2 | 15-19 |
| Dagger 2H R1 | 10005800 | 21030011 | 1.0/1.4 | 1.20 | 27 | 94.5 | 17.1 | 9-15 |
| Dagger 2H R2 | 10006200 | 21030031 | 1.0/1.4 | 1.4112 | 33.75 | 118 | 25.2 | - |
| OKH R1 | 10050000 | 34010011 | 1.2/1.1 | 0.945 | 10 (negatePoise 15%) | 160 | 30.6 | 23-28 |
| OKH R1 chain | 10050100 | 34010020 | 1.0/1.0 | 0.8741 | 10 | 160 | 33.8 | 29-35 |
| OKH R2 | 10050310 | 31010032 | 0.9/0.7 | 1.134 | 12.5 | 200 | 45.1 | 24-28 |
| OKH 2H R1 | 10051500 | 34030011 | 1.0/1.6 | 1.20 | 15 | 240 | 38.2 | 23-28 |
| OKH 2H R2 | 10051810 | 34030010 | 1.1/1.3 | 1.512 | 18.75 | 300 | 56.4 | 21-28 |
| OKH 2H R2 chain | 10051900 | 34030031 | 1.0/1.5 | 0.672 then 1.008 | 15 / 18.75 | | 58.3 | 23-28 (ID1) then 30-38 (ID0) |

A third R1 in a chain has no slot of its own. How the chain continues past Normal2nd is decided outside
the params and was not traced.

## 2. Poise and stagger

### Defender poise pool (EXE + REGULATION)
`ChrPoiseCtrl` (RTTI, vtable `0x1410bfc58`). Its update is `v3` at `0x140145a40`. It keeps current poise at chr+0x218,
min at +0x21c, and max at +0x220.
- **max poise** = `vcall(chr+0x398)->[0x1f8]()` + `ChrParam.poiseValue` (+0x118; 0 for the player row 100)
  + int(`s8 status+0x305` + `s32 status+0x3a0`). EXE. What the vcall sums is in "Max poise" below (EXE,
  2026-10-01). `status+0x305` is SpEffect `1000[0]` kind 0 (INFERRED from the kind -> `status+0x305+kind` pattern
  that three rings fit, see "Attacker poise damage"; the writer was not read); `status+0x3a0` is not identified.
  The armor values are REGULATION: Havel's 19+51+31+31 = 132; Alva 6/15/4/9; Drangleic Mail 28; max single piece 83.
  Ring of Giants is `1000[0]` kind 0 +10/20/30 (`SpEffectRing.emevd` events 40290000/1/2, REGULATION).
- **regeneration** `0x140145ba0`: `current += rate * dt`, clamped. `rate = ChrParam.poiseRecoveryValue`
  (+0x11c) = **0.56** for the player. EXE for the code and REGULATION for the value. That dt is seconds is INFERRED, so the rate would
  be 0.56 poise/s. Regen is suspended while `status+0x7e6 != 0`.
- **reset after a break** (EXE): if `status+0x7e6` was set on the last frame, it is now clear, and current <= 0, current is set to max.
  Most likely +0x7e6 is the "in stagger/damage state" flag (INFERRED). Result: once poise is broken it comes back
  full when the stagger ends. Partial damage otherwise regenerates only at 0.56/s. The "last frame" copy is
  `0x14031c460` (`status+0x7e7 = status+0x7e6`), the status reset that also zeroes hyperarmor (below).
- TAE event **111200** (handler `0x14032678d`) sets `status+0x610`. While it is set, regen uses `changePoiseRecoveryValue`
  (0 for the player), and poise is refilled to max on entering and on leaving the window. EXE. No weapon attack
  in the dump uses 111200.
- **subtraction** `applyPoiseDamage` `0x140145970` (EXE): `new = current - dmg`; if `new < min` (+0x21c) current
  becomes min, else `min(new, max)`. The value of min for a player was not read.

**Max poise, what the vcall adds** (EXE, read 2026-10-01). `PlayerGameParamCalculator` slot 0x1f8 `0x14037fdd0`:
```
sum over the 4 worn pieces of  ArmorParam.strong (+0x40) * (1 - LackOfStats(0x1c, v_piece))
+ PhysicalStatsPerLevelStatValuesParam[min(END, ADP)] +0x90
```
- The piece factor is calculator slot 0x70 -> `0x14031e680` -> slot 0x68 `0x14031e660` = `1 - slot 9`, and slot 9
  `0x1403811f0` is the `PlayerLackOfStatsParam` penalty (the same path the weapon attack's `k` uses, "Attack
  rating" below): row 0x1c (28) = `[30, 80, 0, 30]`, penalty = `clamp((30 + 50*v) * 0.01, 0, 1)` for `v > 0` and
  **0 for `v == 0`** (the second term's input is passed as 0.0). `v` is the piece's float at equip record
  `+0x2b8 + 0x30*i`, written by `0x14034a830` from `0x14034d150`: `clamp(sum of max(0, 1 - stat/req) over four stats, 0, 1)`
  with the requirements as u16 at the armour row's `+0x2c/+0x2e/+0x30/+0x32`. So **an intact piece whose stat
  requirements are met counts 1.0; a piece worn under its requirements keeps 0.7 down to 0.2 of its poise. It is not
  durability.** That the four stats are STR/DEX/INT/FTH (`0x14038d510` indices 4-7) and the four u16 are
  ArmorParam's requirements is INFERRED from the shape.
- The "+0x4c of a second row" is the player stat block: `[chr+0x490]` -> `0x14038b990` (+8) -> +0x4c. The stat
  builder `0x14038d790` writes that word at its `rdi+0x20` (`0x14038da11`; the builder's `rdi` is the reader's block
  + 0x2c, checked on three other fields: defense `rdi+0x1c` / read +0x48, STR attack `rdi+0x24` / read +0x50, status
  `rdi+0x48` / read +0x74). The value is `PhysicalStatsPerLevelStatValuesParam` row `min(END, ADP)` (stat words
  `+0x2` and `+0x10`, `cmovge` at `0x14038d9de`) field +0x90, a float. REGULATION: 0.3 per point to 30 (stat 10 ->
  3.0, 20 -> 6.0, 30 -> 9.0), then 1.0 per 5 to 50 (13.0), 1.0 per 10 to 90 (17.0), 18.0 at 99. **Every player
  has this stat poise on top of the armour.**
- ArmorParam.strong (joined by ItemParam.armorParamId and name) equals SoulsPlanner's piece poise for 405 of 427
  pieces; the recommender uses the game's value (`regulation_poise`).

### Attacker poise damage (EXE)
`ChrDamageActionCtrl::v36` `0x140139160` computes the base poise damage of a hit. The attack packet must be type 1, otherwise the result is 0:
```
base = (DamageCtrlParam.poiseDamage[+0xa0] * hit[+0x74] + vcall(defender+0x398)->[0x188](float dmg) + s8 hit[+0x70])
       * (hit[+0x90] ? u16 [hit[+0x90]+8] * 0.01 : 1.0)
```
Read 2026-10-01 with every term traced (the call chain is in the next paragraph):
- `hit+0x74` is `WeaponParam.poiseDamageScalePlayer`: `calculateDamage_attack` `0x1401373b0` stores the attacker's
  `PlayerGameParamCalculator` slot 0x190 (`0x140381120`), which looks up the WeaponParam row of the attacking hand
  and returns +0xb0 (`poiseDamageScalePlayer`) when the target is a player and +0xb4 (`poiseDamageScaleEnemy`)
  otherwise. `DamageCtrlParam` +0xa0 is `poiseDamage` (Smithbox layout). So dagger R1 = 10 x 3.5 = 35 and OKH R1 =
  10 x 16 = 160 to a player.
- **Slot 0x188 is always 0 in the shipped regulation.** Its argument is `(float)(int)result+0x34` (`0x140139198`),
  and v14 stores there the return of `calculateDamage_defense` (`0x140137e03`): the hit's integer HP damage. The slot
  (`0x140381080`) looks up row 0 of the DamageMan param at `+0xb40+0x150`, which is index 0x14 of the loader
  `0x1403b1850` (slot i at `0xb40 + 0x10*(i+1)`) and of its name table `0x14048ba80`: **`PoiseDamageParam`**. It
  returns `f[9]` if dmg >= `f[4]`, `f[8]` if >= `f[3]`, `f[7]` if >= `f[2]`, `f[6]` if >= `f[1]`, `f[0]` if >= `f[0]`,
  else 0. The one row, id 0, is ten zeros (REGULATION), so the term is 0 for every hit.
- **`s8 hit+0x70` is the Stone Ring's flat poise damage.** `calculateDamage_attack` copies it from `sAr+0xce` =
  attack block +0x86, which the attack block constructor zeroes (`0x1403ada20` -> `0x141afb8ed`, qword +0x80 =
  1.0/0) and the per-hand builder sets from the attacker's `s8 status+0x307` (`0x141b57e0a`..`0x141b57e23`, `r13` =
  status, the same register the builder tests `status+0x4b8` through). `status+0x305+k` holds SpEffect `1000[0]`
  kind k (INFERRED: Ring of Giants is kind 0 and +0x305 feeds max poise; Red Tearstone is kind 4 = 30 and
  `s8 status+0x309` is the HP% threshold the attack builder compares, "What else a hit carries"). The only
  `1000[0]` kind 2 in the eleven `SpEffect*.emevd` is event **40230000 = Stone Ring**, value **30** (REGULATION,
  name from `itemname.fmg`). So with a Stone Ring every hit carries +30 poise damage before the
  multiplications below: dagger R1 35 -> 65, OKH R1 160 -> 190.
- **`hit+0x90` is x1.0 for a player attacker.** It is `FUN_1401649b0(CharacterManager, attacker)`: the
  CharacterManager param at +0x810 (index 0x50 of `getCharacterManagerParamNameFromIndex` `0x14048b620`, base 0x300
  checked on index 0x27 at +0x580) = **`ChrMultiplayParam`**, row = byte `ChrParam+0x24` (`chr+0x38` is
  `chr_param`), entry = `min(CharacterManager+0xd58/+0xd5a, 3)` of `{null, row, row+0xc, row+0x18}`. Entry 0 is
  null, so x1.0. The player's ChrParam row 100 has byte +0x24 = 0, and ChrMultiplayParam row 0 is all 100
  (REGULATION), so every entry is x1.0 too. The same pointer's u16 +2 scales HP damage in `calculateDamage_defense`
  (`0x14013909d`).

The defender side (`ChrDamageActionCtrl::v14` `0x140137aa0`) multiplies the base by
`1 + DamageCtrlParam.damageDecrementPoiseMaxRate (+0xb8) * ctx[+0x78] * 0.01` (EXE, `0x140137eca`..`0x140137efc`),
then applies hyperarmor (next subsection) and stores the result at result+0x40. **`ctx` is not the hit block**: it is
v14's fourth argument, the hit context `0x140136ab0` builds (the hit block's own +0x78 is the HP-damage multiplier
`calculateDamage_attack` writes as `buffer[0x1e]`, a different field). `0x140136ab0` writes ctx+0x78 at
`0x140136e1c` (EXE, 2026-10-01):
```
d    = |defender position (vcall +0x148) - root attacker position (0x14040d770 on ctx+0x68)|
near = sAr[+0xdc] * DamageCtrlParam[+0xa4]      far = sAr[+0xe0] * DamageCtrlParam[+0xa8]
ctx+0x78 = 0                     if d <= defender ChrCommonParam+0x35c (0.0 in row 100) or d <= near
         = (d - near)/(far - near) if near < d < far
         = 1.0                   if d >= far
```
- The root attacker is the attacking entity with any bullet walked back to its owner (`0x140139a80`), so a
  bow's band is measured from the archer.
- `sAr+0xdc/+0xe0` are attack block +0x94/+0x98. The constructor sets both to 1.0 (`0x141c4cee4`, `0x141c4ceee`). The
  builder replaces them from the attacker's `status+0x444/+0x448` when `status+0x4b8` bit 7 is set (`0x141b891f0` ->
  `0x140299376`) or `+0x458/+0x45c` when bit 8 is set (`0x141b78056` -> `0x14032b5ca`); both paths end at `0x140070fc4`
  (`[r14+0x98]`). What sets those two bits was not read; with neither, near and far are the DamageCtrlParam values
  in metres (INFERRED unit).
- REGULATION: 1141 of 3228 DamageCtrlParam rows have a band. Row 1013100 (poiseDamage 10; 176 rows share its band) has near 1.25, far 2.0,
  poise -70%, HP (`+0xac`) -30%; 799 rows have a nonzero poise rate (610 at -70, 80 at -50, 89 at -30, 20 at -100).
  Bands run from 1.25-1.5 m (short melee) to 48-84 m (ranged). The same share scales HP damage in
  `calculateDamage_defense` (`0x1401390cc`: `1 + DamageCtrlParam+0xac * ctx+0x78 * 0.01`) and stamina damage
  (+0xb0, +0xb4 at `0x140137e2e`, `0x140137e7f`).
- So **a point-blank hit (d <= near) takes x1.0 (EXE).** A hit on a 1.25-2.0 m, -70% row landing at 1.6 m takes x0.67 poise, at 2.0 m
  or beyond x0.3. In PvP the attacker's ctx+0x78 travels in packet 28 as one byte (`0x140160ee0`, `x DAT_1410ad0cc`).

`scripts/ds2-builds-recommend.py` (`hit_poise`) takes the slot term at 0 (right), `hit+0x70` at 0 (right without a
Stone Ring), `hit+0x90` at x1.0 (right) and the falloff at x1.0 (right at point blank only).

**Order of a hit** (EXE, per-hit function `0x1401345c0`, `ChrDamageActionCtrl` vtable `0x1410bf178`):
`+0x68` calculateDamage_attack (`0x1401347ea`) -> `+0x70` v14 (`0x140134815`, poise into result+0x40) -> `+0xf0`
`damageEntity` `0x140136220` (`0x140134869`), which calls `applyPoiseDamage` when result+0x40 > 0 and
`status+0x4b8` bit 52 is clear -> `+0x80` stagger decision `0x140136570` (`0x140134a1b`). So the decision reads
poise after this hit's damage is taken off.

### Hyperarmor (EXE)
- TAE event **111900**, case in `ChrMorphemeTimeActTrackDamageActionCtrl::parseDamageActionTae` `0x140326240`
  (code at `0x140326884`..`0x1403268bf`): `status+0x7d0 = row[+0xb8]`, where row = `0x140349250(equip, hand)`
  with equip from `0x14034e6f0(chr)` and hand = `(status+0x188 != 0) - 2`. That is the same row getter (thunk
  `0x141c5212f`) on the same equip object that slot 0x190 `0x140381120` uses to return `poiseDamageScalePlayer`
  (+0xb0), so the row is the WeaponParam row (EXE by the shared getter) and +0xb8 is `uninterruptibleRate`
  (Smithbox layout; REGULATION values OKH 0.3, dagger 0). It writes every frame the event is active.
- `0x14031c460` (called from `0x1403153e0`) writes `status+0x7d0 = 0` along with the other per-frame status resets,
  so the value lasts only while 111900 keeps rewriting it (that `0x1403153e0` runs every frame is INFERRED).
- Defender side `0x140137f02`..`0x140137f14` (EXE): `if (status+0x7d0 > 0) result+0x40 *= status+0x7d0`.
  For OKH (0.3) that means it takes 30% of the poise damage during its own 111900 window.
- Stagger decision `0x140136570` (EXE, 2026-10-01):
  - `holds = (current poise > 0)` (`0x1401459d0`: `comiss` then `seta`, strictly greater), read after the subtraction.
    **A hit staggers when poise after it is <= 0**: poise damage equal to current poise staggers.
  - armorBreak (`DamageCtrlParam+0x2`) 2: `holds = 0` (`0x14013667d`).
  - armorBreak 1: `holds = 0` unless `status+0x7d0 > 0` (`0x140136689`, hyperarmor) or the `status+0x640` flag is set
    with `ctx+0x86 == 0`.
  - `status+0x640` is incremented once, at action-controller construction (`0x14030e0ec` in `0x14030dce0`), when the
    character's `ChrCommonParam+0x70` is 5. It is 0 in the player row 100 and 5 in 26 of 225 EnemyCommonParam rows
    (REGULATION), so it is **0 for every player**: in PvP armorBreak 1 staggers unless the defender is in hyperarmor.
  - `status+0x5ec` (`ultra_armor`) or `status+0x5e8` (super armor, ignored by armorBreak 2) set: no stagger reaction.
    TAE 110500/110000/110600 and `ChrCommonParam+0x70` 1/2 feed those counters (`parseDamageActionTae`, `0x14030dce0`).
  - When holds and neither armour flag, the reaction is 1 if `0x140203c10` (a chr-type table check) else 0; when
    not holds, the incoming reaction is kept (the stagger).
- Windows (TAE): OKH R1 111900 frames 20-30 around its 23-28 hit; 2H R2 20-30. Dagger R1 has no 111900 and its
  rate is 0, so a dagger has no hyperarmor. `WeaponTypeParam.toughnessPeriodScale` is 1.0 for all 144 rows, so it does nothing.

### Hitstun and "true combos" (unverified)
- The stagger animation is chosen by `DamageCtrlParam.damageMotion`: dagger R1 = 2, R2 = 3, OKH 2H R2 = 8.
  Those animations and their lengths were not mapped. Candidate damage anims sit in the TAE's `000160xxx`
  block (INFERRED). `DamageParam.stiff` (dagger R1 1.8 s, OKH R1 1.0 s) is unexplained.
- What the data does support about the user's claim: a dagger R1 does 35 poise damage at point blank (EXE,
  section 2; 65 with a Stone Ring, less beyond the hit row's near distance). So any defender whose current poise
  is 35 or less is staggered by a point-blank dagger R1 (poise left at exactly 0 staggers).
  Most light PvP builds have 0-30 armor poise plus the stat poise of min(END, ADP) (3-9 at stat 10-30), and
  their poise is refilled after each stagger.
  So "a dagger hit staggers a low-poise defender" is consistent with the data.
  "Long enough for a free follow-up" (a true combo) remains folklore until the damage-animation length is
  compared with the dagger's R1->R1 time. The dagger's next hit becomes active at the earliest around anim
  frame 13 (111500 window opens) + 10 (chain hit start) / 2.0 play speed, but the chain-input meaning of 111500 is INFERRED.
- Stun gauge: ChrParam row 100 has stunMaxValue 500, stunRecoverValue 7, stunDisableFlag 0;
  DamageCtrlParam.stun is 5 per dagger R1 and 30 per OKH R1. This is a separate gauge whose PvP effect was not traced.

## 3. Timing: where it lives, and one weapon extracted

- **Location**: `GameDataEbl` `/timeact/chr/c000100_pl.tae` (format string `gamedata:/TimeAct/chr/c%06d_pl.tae`
  in the exe). It is one of the AES-ranged BHD5 entries that `ds2-ebl.py extract` refuses. It was decrypted
  here with the entry's own key record (key + range list at the entry's aesKeyOffset in the decrypted BHD5,
  AES-128-ECB over range 0..929040), which produced a valid `TAE ` header. It was unpacked with
  WitchyBND in passive mode under a PTY. The `.anibnd` (morpheme4) holds the animations, not the events.
- **Event meanings** (TAE/EXE; handler = `CharacterTimeActEventHandler` virtuals):
  - **2200 {ID}** = attack hitbox window, ID = damageId slot (0 -> damageId01). The ID ordering is
    TAE+REGULATION consistent (OKH 2H R2 chain uses ID 1 and 0 matching its two damage rows). The handler is not traced.
  - 120100/120200/120300 (AttackCtrl `0x140325690`) increment status counters +0x4d0/+0x590/+0x548 while active.
    120100 runs from frame 0 to about 3-4 frames before the hit ends (INFERRED: action lock).
  - **111500** (`0x140326823`, status+0x5c0) opens 2-4 frames after the hit starts and closes about 8 after it ends in
    all 29 R1 anims measured. INFERRED: the chain/cancel input window.
  - 111900 = hyperarmor (above). 111600/111700/111800 set bits 0x20/0x100/0x200 in chr+0xc0 +0xf4 (meaning unknown).
    101100 spans the whole animation.
- **Dagger R1 (anim 21010010) extracted**, anim frames at 30 fps (DS2 TAE times are seconds; x30):
  startup to hitbox **8 f**, active **8-15 (7 f)**, 111500 window 13-22, 120100 lock 0-13, anim end 30.
  With startPlaySpeed 1.1 / endPlaySpeed 1.2 that is roughly 7.3 f (0.24 s) to first active frame. Which part
  each speed covers is unknown, so this is an estimate.
  OKH R1 (34010011): hitbox 23-28, lock 0-25, 111500 26-36, hyperarmor 20-30, end 70; speeds 1.2/1.1.
- Caveat: the TAE gives animation-time events. Actual chaining is governed by the state machine (not in
  params, not traced), so "recovery" is the INFERRED 111500/120100 windows, not a verified cancel frame.
- **111500's reader** (EXE, read 2026-10-01): its handler (`parseDamageActionTae` `0x140326240`) counts
  `flags+0x5c0` up and down; in `0x140300000-0x140360000` the only other access is the track's destructor
  (`0x140325fe0`) undoing it. The consumer that would make it a chain or cancel window was not found.

### Attack play speed (EXE, read 2026-10-01)

What the game does with `WeaponAttackMotionParam.startPlaySpeed` (+0x8) / `endPlaySpeed` (+0xc):

- **Copied into the character flags.** Code in Arxan's section at `0x141b3ff3b` reads the attack row's
  +0x8 and +0xc (r14 = the row) and stores them to `sCharacterFlags+0x1f8` and `+0x1fc`
  (`PlayerCtrl+0xb8`), with a third float from the caller's frame (`[rbp+0x188]`, source not traced)
  into `+0x200`. A second writer of the same three fields, `0x141b9446b`, takes them from registers
  whose source was not traced. The row lookup is `0x140397510` -> `0x140024a47` -> `0x140359120`
  (CharacterManager param slot +0x440, index 0x13 = WeaponAttackMotionParam in
  `getCharacterManagerParamNameFromIndex` `0x14048b620`).
- **Turned into one Morpheme control parameter.** `ChrAttackMotionCtrl` binds its control parameters
  in `0x14035c770`; index 4 is `ControlParameters|Attack_SwingSpeed`. Every frame `0x14035d580` sets it
  from the action state `S = PlayerCtrl+0xc0`:
  - `S+0xa0 != 0`: flags+0x200 x startPlaySpeed;
  - else `S+0xa4 != 0`: endPlaySpeed (0 when flags byte +0x195 is set);
  - else 1.0.
- **`S+0xa0` and `S+0xa4` are counters of Morpheme event-track actions**, not TAE events:
  `ChrEventTrackActionAttack` (`0x140368e70`) counts `S+0xa0` up and down for action id 150 (0x96) and
  `S+0xa4` for id 151 (0x97), on the event's start and end flags. So each attack animation says, in
  the animation network's own event tracks, which stretch plays at the start speed and which at the
  end speed. Outside both it plays at 1.0, not at either speed.
- Those tracks live in the Morpheme network/animation data (`.anibnd`), which has not been extracted
  here; the TAE XMLs do not carry them. **Where one speed hands over to the other is therefore still
  unread**, and so is whether the windup is covered at all. That `Attack_SwingSpeed` scales the
  attack animation's playback is INFERRED from its name; the network that consumes it was not read.

What that does to `chain_timeline`'s rule (frame / 30 / mean of the two speeds): the true time to a
hit lies between frame / 30 / the largest and / the smallest of {start, end, 1.0}. For the first R1 hit
(TAE 2200 start, attacks.json):

| attack | anim | hit frame | start / end | mean rule | all start | all end | 1.0 |
|---|---|---|---|---|---|---|---|
| Black Flamestone Dagger 2H R1 | 21030011 | 9 | 1.0 / 1.4 | 0.250 s | 0.300 s | 0.214 s | 0.300 s |
| Giant Warrior Club 2H R1 | 34030011 | 23 | 1.0 / 1.6 | 0.590 s | 0.767 s | 0.479 s | 0.767 s |
| Uchigatana 1H R1 | 28010010 | 14 | 1.25 / 1.5 | 0.339 s | 0.373 s | 0.311 s | 0.467 s |

If the start speed covers the windup, as the Japanese field names (start part / end part) suggest, the
mean rule is 0.05 s (17%), 0.18 s (23%) and 0.03 s (9%) early on these three. With the speed read as 1.0
outside both tracks, the uchigatana could be up to 0.13 s (27%) late.

`scripts/ds2-poise-need.py --play-speed start|end` measures the bounds. At SL 120 for the weapons a
Black Flamestone Dagger warrior is granted: the great hammers' share of counters landing first is 69%
under the mean rule, 98.5% at the start speed and 57% at the end speed; the median `need` is 100.1,
125.1 and 90.1. The weapons' order by hit time changes too, because their start/end ratios differ.
The poise numbers rest on this unread handover more than on anything else in the model.

**TAE units.** DS2 TAE event times are float seconds of animation time. Every event time in the three
anims above is a multiple of 1/30 (2200 at 0.3, 0.7667 and 0.4667 s; GWC 111900 at 0.6667-1.0 s), so
"frame = seconds x 30" is exact and the 2200 hitbox and the 111900 hyperarmor window come from the same
event list in the same unit (`ds2-attacks-extract.py` and `tae_windows` both multiply by 30). The handler
`parseDamageActionTae` `0x140326240` compares the track's `currentTime` with `startTime`/`maxTime` as
floats (its branch for event 110300). That the track time is the animation's own time, stretched by the
play speed like the hit, is INFERRED from the tracks being `ChrMorphemeTimeAct*` driven by the animation.

### Reach, startup and recovery (`--best-weapons --json` `metrics`)

- **Hitbox shape** (REGULATION; `scripts/ds2-hit-shape.py <weapon>` prints it): each PlayerDamageParam
  row has `hitDummyPolyType`/`hitDummyPolyId` (where the hitbox starts: type 1 id 100 for every weapon
  swing), `hitModelType`, `radius` and `length` in metres (paramdef Ban Jing [m], Chang sa[m]), and `childDamage`,
  a further hitbox in the same `damageGroup`. WeaponParam `damageHitRadiusScale`/`damageHitLengthScale`
  (+0x98/+0x9c) differ per weapon where a class shares its rows: Spear 0.75, Winged Spear 1.0, Pike 1.05
  on the same 1.55 m row; Dagger and Rapier 0.7; Giant Warrior Club radius 1.4, length 0.7.
- The Whip's swing is three capsules on dummy polys 100, 101, 102 (1.0 + 1.1 + 0.2 m); a Greatsword's child
  is a 0.4 m sphere on the same dummy poly 100; a spear's child sits on body dummy poly 1.
- EXE: TAE 2200 (`parseDamageTae` `0x1403269e0`) copies the slot's `sHitboxData` (damage id, hand, then
  hitbox radius/length and recoil radius/length scales at +0x8..+0x14, defaults 1.0) into the active hitbox.
  Where those scales are filled was not traced, so that they are WeaponParam +0x98..+0xa4 is INFERRED
  from the matching order. The 2200 handler also drains stamina (`drainStamina` `0x1403335d0`).
- **`reach_m`** = (sum of the lengths on distinct weapon dummy polys, id 100 up) x length scale + the last
  segment's radius x radius scale, the longest R1 hitbox. INFERRED composition; it is the hitbox's extent
  from dummy poly 100, not the distance from the attacker (the animation's own travel is not read).
- **`startup_s`** = first live 2200 frame / 30 / mean play speed; **`recovery_s`** = animation end (TAE
  101100's end) minus the last live 2200 frame, same scale. INFERRED: the player is free at the animation's
  end; an earlier roll/block cancel is not read. **`damage_per_5s`**: the `--window` sum over 5 s of
  repeated R1 chain. The `score` is unchanged.

## 4. Multi-hit attacks (Channeler's Trident validation)

COMMUNITY claim (Fextralife): the 1H R2 and the rolling attacks (1H and 2H) spin and hit 3 times, each hit dealing damage and status build-up.

How DS2 encodes several hits (REGULATION + TAE):
1. Several **2200 events** in one animation, each with its own damage slot (`damageId01..03`).
2. The same slot fired in **two separate 2200 windows** (Twinblade 2H R2 chain: two hits of 1.2096).
3. A **repeat-hit interval**, `PlayerDamageParam.hitDistance` (JP Lian Sok HITShe Ding , "continuous hit setting").
   The hitbox can re-hit every `hitDistance` s while its 2200 window is open, so
   hits = floor(window_s / interval) + 1. That the field is a time interval is INFERRED: its values (0.1-3.0) are
   always about a whole fraction of the windows they sit on (for example 0.2 on 0.67 s, 0.25 on 0.63 s). The exe consumer is not traced.
4. `childDamage` / `damageGroup`: the child hitbox shares the parent's damageGroup (for example 40000010), so it is a
   second hitbox shape on the same swing, not an extra hit (INFERRED from the shared group id).
5. Bullets (`bulletId0n`/`bulletDamageId0n`, TAE 2202 BulletID).

**Channeler's Trident (2890000, actcat 2890)**:
| attack | hits (TAE frames) | MV per hit | max hits |
|---|---|---|---|
| 1H R2 (10018240, anim 25010030) | ID0 19-24 -> 0.576; ID1 23-35 -> 0.2688 every 0.15 s (0.4 s window) | 0.576 + 3 x 0.2688 | **4** |
| 1H R2 chain (10018250) | ID0 15-18 -> 0.4608; ID1 18-30 -> 0.2688 x3 | | 4 |
| 1H rolling (10018705) | ID0 15-18 -> 0.3552; ID1 18-30 -> 0.2688 x3 | | 4 |
| 2H rolling (10020305) | ID0 15-18 -> 0.8525; ID1 18-30 -> 0.2688 x3 | | 4 |
| 1H R1 / 2H R1 | one hit | 0.96 / 1.152 | 1 |

(A third 2200 in the R2 uses ID 2, whose damageId03 is 0, so it does nothing.) So the data confirms a
3-hit spin (damage row 10018241, MV 0.2688, poise 12.5, 0.15 s interval), and it adds a separate
opening hit. Up to 4 hits can land if the defender stays in the hitbox. Tick count assumes 30 fps anim time
at play speed 1.0-1.2; both give 3 ticks.

**Status build-up per hit**: DamageParam has no status or build-up field, so there is no per-hit status multiplier
(REGULATION). Build-up comes from the weapon. Each tick is a separate damage packet, so build-up is probably
applied per tick, but that is INFERRED and not traced in the exe.

**2H R2** (10020100, anim 25030038): its only damage row (10020100) has every hitFlag at 0, so it cannot hit anything.
It spawns bullets 1990000/1990010 (BulletParam sfx 5413/5414). `SpEffectWeapon.emevd` event **1990010** plays
SFX 5414 and applies `1000[1] [kind 25, 20 s, x1.05]`. Kind 25 is also on item event 60406020 (5.5 s, x1.15),
and kind 26 is damage taken (Ring of the Embedded 1.085), so kind 25 = damage dealt (INFERRED).
The data therefore gives x1.05 for 20 s, not +25%. The "25" is the kind code. Whether it reaches allies
(via the bullet) is INFERRED.

**General method**: `dps/attacks.json` flags every attack with `max_hits > 1`. 76 weapons have at
least one multi-hit Single-grip attack: all lances and pikes (R2 and dash 4-5), twinblades (2H R1 3-4),
Ricard's Rapier R2 (4, chain 7), Yorgh's Spear, halberds (2H R2 2, dash 3), and others.
Old Whip is a special case: every attack has two hitboxes at once in different damageGroups (1.0 + 0.35),
so whether both hit one target needs runtime proof. Treat overlapping windows with different groups as
"possibly 2", and repeat ticks or disjoint windows as real extra hits.

**Used by** `scripts/ds2-builds-recommend.py --weapons-for ... --objective bleed|poison` (`attack_hits`,
`status_hits`): build-up per hit times the hits of the weapon's best R1/R2 attack, or the hits landed
within `--window` seconds of repeating that chain. A hitbox that repeats another's exact window, tick
count and interval (the Old Whip pair) counts once; overlapping windows like the trident's count
separately. Per-tick build-up is the INFERRED part above. `--best-weapons` rows also carry the proc
damage ("What a proc does to a player" below).

## 5. What remains unverified
- Poise (section 2, all terms of the formula now EXE): what sets attacker `status+0x4b8` bits 7/8 (they swap
  the falloff band's base distances), the player's min poise (+0x21c), the `status+0x3a0` max-poise term, and
  that the `status+0x305+k` bytes are written from SpEffect `1000[0]` kind k.
- That the regen dt is in seconds.
- The damageMotion -> stagger animation mapping and its length, which decides true combos.
- Where startPlaySpeed switches to endPlaySpeed.
- That 111500 is the chain window.
- That hitDistance is a time interval.
- Per-tick status build-up.
- 278 attack slots point at animations missing from c000100_pl.tae (7 anim ids, for example 20510540 and 40030030),
  which are probably in the DLC/sfx TAEs.

## Damage per hit against a player (EXE)

`ChrDamageActionCtrl` (vtable `0x1410bf178`), slot 32 `0x140138d50`: physical
`max(AR*10 - DEF_type, damageLower) / 12`, each element `max(AR*6, damageLower) / 6 * (1 - cut)`,
summed, times MV and counter/state factors, truncated. The MUL/DIV constants {10,6,6,6,6} / {12,6,6,6,6}
are DamageAdjustParam +0x158..+0x16C, read as integers by `PlayerGameParamCalculator` (`0x140380f30`,
`0x140380ed0`). A player's elemental defense entries are always 0 (`0x140380070`), so the 6.0 cancels and
elemental defense acts only through `cut`, clamped to [0, 1] and capped at PlayerCommonParam+0x18C (99.0).
`DEF_type` is the hit's slash/strike/thrust defense (`DamageCtrlParam.attackType`, REGULATION).
`damageLower` is `PlayerDamageParam.damageLower` (70 on 1742 of 3082 rows). The elemental `cut` is
resolved below: it is `(D + 100) / 1000` for the defense D that SoulsPlanner shows. A remote PvP hit
carries the same factors: `0x140160ee0` packs the damage buffer's `+0x78`/`+0x7c`/`+0x80` into packet 28
and `0x140161370` unpacks them to the same offsets on the victim's side (read 2026-09-30; "What else a
hit carries" below).

**Where the attack and the motion value enter** (read 2026-09-29 through the Ghidra daemon). The
per-hit function on the defender is `ChrDamageActionCtrl::FUN_1401345c0(this, out, sAr *attack)`.
It calls `calculateDamage_attack` (`0x1401373b0`, vtable slot 13) and then the defense side, which
ends in `calculateDamage_defense` (`0x140138d50`, slot 32):

```
attack[i]  = (live[i] + sAr.ar[i]) * sAr[+0xb4] * MUL[i]                     0x1401373b0
damage[i]  = max(attack[i] - DEF[i], lower) * clamp(1 - cut[i]) / DIV[i]
             * DAMAGE_PARAM.damageRate * sAr[+0xb8] * bullet[i]              0x140138d50
lower      = DAMAGE_PARAM.damageLower when attack[i] > 0, else 0
bullet[i]  = 1.0, or for a bullet hit the defender calculator's slot +0x1a8, which
             PlayerGameParamCalculator (0x140380ea0) sets to 1.0 for every type
```

- `sAr` is the incoming attack (Ghidra struct `sAr`, 0xd0 bytes): `+0x0` damage param id, `+0x45` hand,
  `+0x48` physical, `+0x4c` magic, `+0x50` lightning, `+0x54` fire, `+0x58` dark, `+0x5c` flat. A PvP
  hit reaches the victim as packet 28 (`PlayerDamageActionCtrl::ChrDamagePacket_pvpDamage`
  `0x1401f7c10`, struct `sDamagePacket`), which carries the same five attack ratings.
- `sAr.ar` is the attack builder's own attack block, whole: the per-hand builder `0x140391fe0` (see
  "Attack rating" below) fills a 0x9c-byte block, and `checkCollisionForDamage` (`0x140314520`) copies
  156 bytes of it to `sAr+0x48`. So the weapon's attack rating, with its damage row's flat damage, is in
  `sAr.ar`.
- `live` is `PlayerGameParamCalculator` slot 56 (`0x140380dd0`, vtable `0x1410e4ec8` +0x1c0): kind 5 for
  hand 1 or kind 6 for hand 2, read by `0x141c4cd4e` -> `0x14022e200` -> `0x14014ce10`. That is the
  handle the stat modifiers are read through ("Two-handing" below), so it is the hand's attack add
  (SpEffect-shaped, INFERRED as the stat modifiers are), not the weapon's attack. Whether the builder's
  own kind 5/6 add (`0x14038f3b0`, below) and this one both reach a melee hit was not read. `live` is
  added only when the attacker is not a bullet: the hit context built by `0x140136ab0` sets `+0x85` when
  the attacker's class is `BulletObject` (class id via `0x140448050`), and `calculateDamage_attack`
  skips slot 56 when `+0x85` is set. A spell therefore hits with only the attack ratings its bullet
  carries.
- `damageRate` (the motion value) multiplies after the defense is subtracted. So one point of physical
  defense takes `damageRate / 12` off a hit whatever its attack, and one point of an element's defense
  `damageRate * attack * sAr[+0xb4] / 1000`; their ratio does not depend on the motion value.
- Not read here: where a bullet's `sAr` attack ratings are filled on the caster's side.

### What else a hit carries (EXE)

Read 2026-09-30. The attack block's constructor `0x1403ada20` sets its `+0x6c`, `+0x70` and `+0x74` to
1.0; they arrive as `sAr+0xb4`, `+0xb8` and `+0xbc`. The builder then writes them:

| field | writer | value | when |
|---|---|---|---|
| `+0xb4` | `0x141c59b4a` -> `0x141c38f32` | the attacker's `flags+0x420` | every player attack |
| `+0xb4` | `0x140027533` | x `flags+0x428` | `chr+0x228` set, HP% (`0x14016a440`) at or below s8 `flags+0x309`, not a spell |
| `+0xb8` | `0x141c5ccfc`, `0x141c5cd09` | `WeaponParam.damageScale` (+0x80) | not a spell |
| `+0xb8` | `0x14005ed12`, `0x141b11453`, `0x141ca8114`, `0x141b9d7cb` | `flags+0x48c/+0x490/+0x494/+0x49c` | a spell, by `spellCategory` 0, 1, 2, 4 (3 is behind an unresolved stub) |
| `+0xbc` | `0x141c5cd14`, `0x141c5cd19` | `WeaponTypeParam.counterDamageScale` (+0x6c) | always |
| `+0xbc` | `0x1404c6084` | x `flags+0x3e8` | `DamageCtrlParam.attackType` 1 (thrust) |

- `calculateDamage_attack` multiplies the five typed attacks and the seven status build-ups by
  `+0xb4`, and `calculateDamage_defense` multiplies each typed damage by `+0xb8` after the defense
  (`0x140138f8a`). Neither touches `sAr+0x5c`.
- **Counter hit** (`0x140138fc9`): damage x `DamageParam.counterDamageRate` (+0x20) x `+0xbc`, when the
  victim's `flags+0x630` is nonzero or bit 0x20 of `[chr+0xc0]+0xf4` is set (TAE 111600 sets that bit,
  section 3). `counterDamageRate` is 1.0, 0.91 or 0.8 per row (REGULATION).
- **Critical hit** (`0x14013904c`, when the hit context's `+0x83` is set): damage x
  `ChrCommonParam.grabDamageRate<type>` (0.8 for every type in its one row, 100) x the `PlayerCommonParam`
  backstab 0.8, riposte 0.9 or guard-break stab 1.0, chosen by the victim's grab category / 10000 and
  applied when `thunk_FUN_141b8b72f(CharacterManager, 8) > 24` x the victim's `flags+0x79c`.
- Grip, power stance, jump and running attacks are read by none of these writers.
- `damageScale` is not 1.0 on nine weapons (REGULATION): Light, Heavy, Shield and Sanctum Crossbow 1.25;
  Scythe of Want 0.8; Silverblack Sickle, Stone Twinblade and Dragonrider Bow 0.85; Dragonrider
  Twinblade 0.875. `counterDamageScale` runs from 1.0 to 1.6.
- Physical damage also passes slot 37 (`0x140139a40`), `ChrCommonParam.receiveSlash/Thrust/StrikeDamageRate`,
  all 1.0.

The `flags` rates look like SpEffect `1000[1]` kind k stored at `flags+0x3bc+4k` (INFERRED: five offsets
each match the effect their items describe; the writer was not read):

| kind | offset | effect | items (REGULATION) |
|---|---|---|---|
| 11 | `+0x3e8` | thrust counter | Old Leo Ring 1.125 |
| 25 | `+0x420` | attack | Drakeblood Greatsword 1.05, Dragon Torso Stone 1.15 |
| 26 | `+0x424` | damage taken | Ring of the Embedded 1.085, Iron Flesh 0.6, Numbness 0.85 |
| 27 | `+0x428` | attack at low HP | Red Tearstone Ring 1.2 |
| 28 | `+0x42c` | damage taken at low HP | Blue Tearstone Ring 0.65 |

Not read: the collision step that moves the block from the active hitbox list into `sLastHitboxData`,
the writers of the `flags` rates, `flags+0x309`, `flags+0x630` and `flags+0x79c`, the non-player builder
`0x140391e60`, and what `thunk_FUN_141b8b72f(CharacterManager, 8)` returns.

### Status build-up per hit (EXE)

Read 2026-09-30. The attack builder computes a status attack exactly as it does a damage type's
("Base, rates and the sum" below): `(bonus + base) * rate + D`, with base `WeaponReinforceParam`
`maximum<Status>` x the infusion row's `baseValueScale` and bonus the status bonus x the row's
`poison`/`bleeding` coefficient. It then scales the block's status entries (+0x18..+0x30: poison, bleed,
durability, curse, burn, toxic, petrify) by `100.0 / X` (`0x140299115`..`0x141b3831f`, the 100.0 at
`0x1410acb18`). X comes from `0x14038b9a0` -> `0x14038d290`, which fills every stat with 99 and returns
the max-HP formula, `hpMax[99]` + `additionalHp[99]` over eight stats: 1945 + 8 x 70 = 2505 (REGULATION).

On the victim, `calculateDamage_attack` multiplies each status by `sAr+0xb4` and by
`DamageAdjustParam.pcAttributeAdjust<Status>` (6.0, which `0x140137f90` divides back out). Status defense
is 0 (slot 58 writes only physical). What lands is

```
build-up = status x (1 - cut) x guard factor (1 unguarded) x ChrMultiplayParam factor (1 with no row)
cut      = armour <status> resistance (ArmorReinforceParam, x 0.01 per piece)
           + poisonResistance[trunc((3*ADP + VIT)/4)] or bleedingResistance[trunc((3*ADP + FTH)/4)] x 0.01
           + SpEffect, clamped to [0, 1]
```

`applyStatusDamage` (`0x140145c60`) adds it to the gauge at `+0x1c4 + 12*id` and procs at 100.0: SpEffect
900100 (poison) or 900200 (bleed), and the gauge drops by 100 (`0x140146430`). So a Bandit Axe (Poison)
+10 builds `(700 + 1.05 x aux) x 100 / 2505` = 27.94 + 0.0419 x aux points per hit before the cut.
SoulsPlanner's `140 + 0.525 x aux` is not this: its base is the builder's / 5 and its coefficient / 2,
so it weighs the status stats 2.5 times too heavily against the base.

Not read: the writers of the gauge's bounds (`+0x1c8`/`+0x1cc`), the source of `live` for statuses,
the row behind the guard factor, and whether a player victim has a `ChrMultiplayParam` row.

**The resistance rows** (EXE, read 2026-10-01). The stats builder starts at `0x14038d784` (the
`0x14038d790` above is inside it). Over its stat words (0 VGR, 1 END, 2 VIT, 3 ATT, 4 STR, 5 DEX,
6 INT, 7 FTH, 8 ADP; the physical-defense row is words 1+2+4+5 over 4) it stores, each x 0.01:
`bleedingResistance` (+0x48) at row `trunc((3*ADP + FTH)/4)`, `poisonResistance` (+0x4c) at
`trunc((3*ADP + VIT)/4)`, `petrifactionResistance` (+0x50) at `trunc((3*ADP + VGR)/4)` and
`curseResistance` (+0x54) at `trunc((3*ADP + ATT)/4)`.

### What a proc does to a player (EXE + REGULATION)

Read 2026-10-01; `scripts/ds2-status-procs.py` prints the events. `applyStatusDamage(obj, id, build-up)`:

| step | what | tag |
|---|---|---|
| id -> SpEffect | 0 poison 900100, 1 bleed 900200, 2 900300/900301, 3 900400, 4 900500, 5 toxic 900600, 6 901100 | EXE `0x140145ca7` jump table |
| id -> category | the byte the status is looked up by in the table at `0x1415710c0`: poison 0 (`0x14023dd00`), bleed 1 (`0x14023dab0`), toxic 8 (`0x14023de40`) | EXE |
| lockout | `0x14014bc80(SpEffectCtrl, category slot)` at `0x140145e51`; nonzero skips the whole build-up | EXE for the call, INFERRED that it tests "an effect of this category is active" (its body is behind Arxan) |
| below 100 | `calculateStatusMeter` (`0x1401462a0`) adds it, clamped to the gauge bounds, and sets a 0.5 timer at `obj+8+4*id` | EXE; what reads the timer (decay) was not found |
| at 100 | `0x140146430(obj, id, 100.0)` sets the gauge to `max(floor, gauge - 100)`, i.e. its floor, so the procing hit's surplus is lost; then `applySpEffect` | EXE |

The proc events (REGULATION, `SpEffectAbnormalState.emevd`):

| event | instruction | reading |
|---|---|---|
| 900100 poison | `100070[0] [1, 872, 0, 22.0, 9010, 0.3, 650, 249]` | category 0 for 22 s, PlayerDamageParam 9010 every 0.3 s |
| 900200 bleed | `100170[1] [9020]` | PlayerDamageParam 9020 once |
| 900210 | `100070[0] [1, 872, 1, 10.0, 0, ...]` plus stamina/regen changes | category 1 for 10 s; 9020's `spEffectIdWhenHit` |
| 900600 toxic | `100070[0] [1, 872, 8, 20.0, 9010, 0.2, 650, 249]` | category 8 for 20 s, 9010 every 0.2 s |

PlayerDamageParam 9010 is `damage01` 15 and 9020 is 200, both `damageType01` 5 (REGULATION). Type 5 is the
attack's slot 5, which `calculateDamage_defense` (`0x140138d50`, `lVar8 == 5`) passes through with no
defense, cut, motion value or multiplier (EXE). So poison is 73 ticks x 15 = 1095 over 22 s, bleed 200 flat
with no max-HP term, toxic 100 x 15 = 1500 over 20 s. The word meanings (category, seconds, row, interval)
are INFERRED from these rows: the category word matches the EXE's lookup byte for all three, and the
community's figures agree (fextralife Poison "1,050/1,095 damage over 22 seconds", Bleed "200 points",
Toxic "approximately 1,485 over 20 seconds"). Not read: the handlers of banks 100070 and 100170, gauge
decay, and whether the lockout covers the whole duration.

**Used by** `scripts/ds2-builds-recommend.py --best-weapons` (`regulation_status_procs`,
`status_cut`, `status_metrics`): each row's `metrics` carries the proc damage its window's hits deal.

## Elemental cut: where the +100 comes from (EXE)

The defender's arrays are filled next to each other at `0x140137b20`..`0x140137b8a`:
`PlayerGameParamCalculator` slot 58 (`0x140380070`, via `call [rax+0x1d0]` at `0x140137b48`) writes the
13-float DEF array, and slot 59 (`0x140381350`, via `call [rax+0x1d8]` at `0x140137b58`) writes the
13-float cut array. `calculateDamage_defense` multiplies each element by `clamp(1 - cut)`. Slot 59 does this:

```
for each armor piece (4):                                   loop 0x1403814c0..
    v = lerp(ArmorReinforceParam.min*, max*, reinforceLv / maxLevel)     0x14034dda0 (row +0x0c.. / +0x3c..)
    v = v * 0.01                                                         0x14034f7b0
    v += 0.01 * SpEffect vector (kind 7)                                 0x14038148e, 0x14034c7d0, 0x140381510
    v *= lack-of-stats factor (PlayerLackOfStatsParam row 0x10 / 0x11)   0x140381c20
cut[magic]     = sum + statblock+0x98 (magicResistance     * 0.01)      FUN_14038d270(.,0)
cut[lightning] = sum + statblock+0xa0 (lightningResistance * 0.01)      FUN_14038d270(.,2)
cut[fire]      = sum + statblock+0x9c (flameResistance     * 0.01)      FUN_14038d270(.,1)
cut[dark]      = sum + statblock+0xa4 (darkResistance      * 0.01)      FUN_14038d270(.,3)
vfunc +0x250 (0x14038179d), clamp every entry to [0, 1] (0x140381824..); cap each element at
FUN_140164af0 (0x140381871) = PlayerCommonParam+0x18c * 0.01 (99.0 -> 0.99)
```

The stat terms come from the stats builder `0x14038d790`. It is called with `lea rcx,[rsi+0x2c]`
(`0x14038d63d`), so its `rdi+X` is the block's `+X+0x2c`. It stores
`PhysicalStatsPerLevelStatValuesParam` row fields times 0.01 (the constant at `0x1410acb08`, loaded at
`0x14038dd4f`). Each row is picked by a stat:

| cut term | row index | row field | store |
|---|---|---|---|
| magic | INT | `magicResistance` +0x58 | `0x14038deae` |
| fire | trunc((INT+FTH)/2) | `flameResistance` +0x5c | `0x14038deef` |
| lightning | FTH | `lightningResistance` +0x60 | `0x14038df31` |
| dark | min(INT, FTH) | `darkResistance` +0x64 | `0x14038df79` |

The row values run from 10.0 at stat 1 to 30.0 at stat 99. So a naked character with 1 INT already has a 0.10 magic cut.

**The +100.** In SoulsPlanner's stat tables, `magicDEFBonus`, `lightningDEFBonus` and `darkDEFBonus` equal
`10 * resistance - 100` for all 99 rows, and `fireDEFBonus[s]` equals `10 * flameResistance[trunc(s/2)] - 100`
for all 197 sums (checked against the decrypted regulation, not against memory). An armor piece's
elemental D is `round(ArmorReinforceParam max% * menuResistanceScale)`, where menuResistanceScale is
10.0 on 475 of 502 rows. That holds for 262 of the 278 SoulsPlanner pieces that map to exactly one
reinforce row. The 16 misses are single-field disagreements, like Black Witch Veil magic 14 vs 55.
So with D = stat part + armor part as the planner shows it:

```
cut = (res% + sum armor%) / 100 = ((D_stat + 100) + D_armor) / 1000 = (D + 100) / 1000
```

The +100 is the 10% floor of the stat table, which the displayed number leaves out. The in-game menu's
own display formula was not traced. `menuResistanceScale` is the likely scale it uses (INFERRED).

Still unresolved:
- FUN_140164af0 applies the 0.99 cap only when `thunk_FUN_141b8b72f(CharacterManager, 8) > 25`, and
  returns 1.0 otherwise. What that lookup returns is not traced.
- `0x14034f2c0`, called on each piece's vector, floors every entry at 0 (read 2026-10-01 with
  `scripts/ds2-arxan-trace.py`: `comiss` against a zeroed `xmm1`, the smaller replaced by 0).
- The SpEffect term inside the per-piece loop (`0x140381510`) is added once per armour slot: the
  loop adds `0.01 x` the vector at `[rbp-0x10]` to each of the four pieces before the lack-of-stats
  factor. That vector is SpEffect property type 0 (`0x14014bb20` -> `0x14022e2f0` -> `0x1402259f0`,
  which sums each matching property's 10 floats at `+0x50`, `0x140222470`), not the
  `100090[4]` defense changes, which reach the cut once through slot +0x250 ("Defense from a
  SpEffect" below). Which instruction writes a type-0 property was not traced.
- Slot 60 (`call [rax+0x1e0]` at `0x140137b8a`) runs only when `[r12+0xc] == 2`, and its array feeds the
  `(1 - atk+0x34) * def+0x68` factor in `calculateDamage_defense`. Guard absorption is the likely
  meaning (INFERRED), not read.

### Defense from a SpEffect (EXE + REGULATION)

Read 2026-10-01 through the Ghidra daemon and `scripts/ds2-arxan-trace.py`. This is how a Quartz Ring,
Ring of Steel Protection, Flash Sweat or a Burr changes the damage a player takes.

- **The instruction.** `100090[4] [seconds f32, value << 16 | type]` adds and `100090[5]` subtracts
  (`SpEffectActionImpl_ChangeAtkDef`, slot 19, `0x14021b230`): asked with request byte 2 it handles
  modes 4 and 5, and adds (mode 4, bit set in the mask `0x214`) or subtracts (5) the u16 at argument
  byte 6 into float slot `byte 4` of the caller's vector. Types are DAMAGE_TYPE's, 0 physical, 1
  magic, 2 lightning, 3 fire, 4 dark, then 5-9 the status resistances.
- **The query.** `0x14014bae0` -> `0x14014ce90` writes request byte 2 (`mov byte [rsp+0x30],2`,
  `0x14000ae20`) and calls `0x14022e1e0` -> `0x14021f100`, which builds the query with that byte at
  `+0x35` and the mask 7 at `+0x33`. That it reaches slot 19 is INFERRED from the byte matching
  ChangeAtkDef's defense branch; the iterator call is mis-decoded at `0x14021f15e` and was not followed.
- **Reader, `ChrGameParamCalculator` slot +0x250** (`0x14031fe20`, not overridden in
  `PlayerGameParamCalculator`'s vtable `0x1410e4ec8`): fills a zeroed 10-float vector through that
  query, keeps its physical entry raw, and adds `0.01 x` the rest to the caller's array
  (`0x14034c7d0`, which also swaps entries 8 and 9), the raw physical entry to entry 0.
- **Physical: flat defense.** Slot +0x248 (`0x140320380`) calls +0x250 on a zeroed array and returns
  entry 0. The physical DEF builder (slot 58, `0x140380070`) adds that return
  (`call [rax+0x248]` at `0x14038020a`) once to `statDEF x sum(defenseStatAffectScale) + sum(piece
  DEF)`, for whichever slash/strike/thrust type it was asked for. So 50 is +50 physical defense, in
  SoulsPlanner's units, on every physical type.
- **Elements: cut.** The cut builder (slot 59, `0x140381350`) calls +0x250 once
  (`call [rax+0x250]` at `0x14038179d`) on the summed array after the four pieces and the stat
  resistances, then clamps every entry to [0, 1] and caps the elements at 0.99. So 15 is +0.15 cut,
  `+150` in the displayed defense D of `cut = (D + 100) / 1000`. The physical entry the call adds to is
  zeroed afterwards (`mov dword [rsp+0x50],0` at `0x14038185f`) and never written out, so a physical
  value does not also become a physical cut.

What carries it (REGULATION, `SpEffectRing.emevd` / `SpEffectSpell.emevd` / `SpEffectActiveItem.emevd`):

| source (event) | change | seconds |
|---|---|---|
| Ring of Steel Protection / +1 / +2 (40050000-2) | physical +50 / +75 / +100 | worn |
| Spell / Thunder / Flame / Dark Quartz Ring, +0..+3 (40060000-40090003) | magic / lightning / fire / dark cut +5 / 8 / 10 / 15% | worn |
| Dispelling Ring / +1 (40140000-1) | all four elements +6 / 12% | worn |
| Sorcery / Lightning / Fire / Dark Clutch Ring (41040000-41070000) | physical -80 (`100090[5]`) | worn |
| Magic Barrier (32190010; +11..+14 are 15% for 70-95 s, chooser not traced) | all four +10% | 60 |
| Great Magic Barrier (32200010) | all four +25% | 90 |
| Sacred Oath (32230010) | physical +75 | 40 |
| Flash Sweat (33180010) | fire +30% | 90 |
| Iron Flesh (33190010) | physical +100, all four +20% | 25 |
| Small Blue / Yellow / Orange Burr, Dark Troches (60160000-60190000) | magic / lightning / fire / dark +15% | 90 |
| Whisper of Despair (34070010) | physical -200 (`100090[5]`) | 20 |

A spell's effect row is its SpellParam id + 10: Magic Barrier's own event names it
(`100120[6] [32190010, ...]`); for the others the +10 is INFERRED from that pattern. Ring of Resistance,
the bite rings, Perseverance and Common Fruit change status types only.

Not read: `1000[1]` kind 26, which Iron Flesh (x0.6) and Numbness (x0.85) carry and which section
"What else a hit carries" INFERS to be a damage-taken rate; no reader of `flags+0x424` in the damage path
was found, so the recommender leaves it out. Ring of Steel Protection's `1000[1]` kind 59 (x0.9, +2
x0.875) is not decoded either.

## Attack rating: stat scaling (EXE)

Where it comes from: the only caller of the scaling function (`0x1403920bc`) and the helpers it calls.
Most of this code is Arxan-shattered, with no Ghidra function and no `.pdata`. It was read with
`scripts/ds2-arxan-trace.py`, which follows the fragments and prints only real instructions. Every
address below is a real instruction in `darksoulsii-deobf.bin`.

**Stat bonus per stat** (the SoulsPlanner `*ATKBonus` tables). The same builder `0x14038d790` stores
integer `PhysicalStatsPerLevelStatValuesParam` fields in the stat block. `0x14038d260` (-> `0x1406d5760`,
`mov eax,[rcx+rax*4+0x50]`) reads them back by index:

| idx | row index | row field | store |
|---|---|---|---|
| 0 | STR | `physicalAttackByStrength` +0x0c | `0x14038da44` |
| 1 | DEX | `physicalAttackByDexterity` +0x10 | `0x14038da79` |
| 2 | min of the 9 stats (loop `0x14038da80`, skips words 9-10) | `physicalAttackByAbyss` +0x8c | `0x14038dac8` |
| 3 | INT | `magicAttack` +0x14 | `0x14038daf8` |
| 4 | trunc((INT+FTH)/2) (`0x14038dafb`..`0x14038db08`) | `flameAttack` +0x18 | `0x14038db31` |
| 5 | FTH | `lightningAttack` +0x1c | `0x14038db66` |
| 6,7,8 | min(INT, FTH) (`cmovge`) | `darkAttack` +0x20 | `0x14038dbb0/dbfa/dc42` |

The stat block's words are `[rbx + 2*(id-1)]` (jump table `0x14038e280`: 1 VGR, 2 END, 3 VIT, 4 ATT,
5 STR, 6 DEX, 7 INT, 8 FTH, 9 ADP). Physical stat defense is stored the same way:
`row[trunc((END+VIT+STR+DEX)/4)].defense` goes to `+0x48` (`0x14038d97b`..`0x14038d9c6`), which slot 58 reads.
Slot 58 multiplies it by the sum of the four pieces' `ArmorParam.defenseStatAffectScale` (`+0x1c`), so a
bare body (naked pieces, 0.5 in all) gets half of it; `docs/DS2-INFUSION-EVIDENCE.md` "The bare defender".
SoulsPlanner's `physicalATKBonus`, `magicATKBonus`, `lightningATKBonus`, `darkATKBonus`, `mundaneATKBonus`
and `fireATKBonus[INT+FTH]` equal these regulation columns with these indices at every entry. Its
`physicalDEFBonus[sum]` does not match: it is off by one at 180 of 393 sums (row 3 is 63 in the regulation
and 62 in the planner).

**Status bonus per stat** (read 2026-09-30). `0x14038d240` is `mov eax,[rcx+rdx*4+0x74]` on the same
stat block, and the builder fills that array (its `rdi+0x48..+0x54`) from the same param:

| idx | row index | row field | store |
|---|---|---|---|
| 0 | the stat `RelatePhysicalStatToLevelStat` byte +0xb names | `weaponBreakAdditionalEffect` +0x24 | `0x14038dc72` |
| 1 | trunc((3*DEX + FTH)/4) | `bleedingAdditionalEffect` +0x28 | `0x14038dcaf` |
| 2 | trunc((3*DEX + ADP)/4) | `poisonAdditionalEffect` +0x2c | `0x14038dcef` |
| 3 | the stat byte +0xe names | `curseAdditionalEffect` +0x30 | `0x14038dd22` |

The scaling function multiplies idx 2 by the poison coefficient into `out[5]` and idx 1 by the bleeding
coefficient into `out[6]` (calls at `0x14039054e`, `0x14039056d`). SoulsPlanner's `auxATKBonus[3*DEX + FTH]`
and `auxATKBonus[3*DEX + ADP]` equal these columns at `row[trunc(sum/4)]` for every sum, and the bleeding
and poison columns are identical (REGULATION). The same builder reads the agility row the same way:
`row[trunc((3*ADP + ATT)/4)]`'s `changeEquipSpeedScale` (+0x74) goes to `rdi+0x88` (`0x14038e08d`). That row
also holds `stepInvincibleTimeRate`, `rollingInvincibleTimeRate` and `jumpInvincibleTimeRate` (+0x68..+0x70).
The stores after `0x14038e08d` were not read.

**Base and coefficients against SoulsPlanner** (REGULATION, measured 2026-09-30, over 2623 weapon and
infusion rows joined by name). Computed as the scaling function and the rate move below compute them, the
site's Mundane modifier agrees with `abyssRate x 0.01 x` the Mundane physical rate for all 209 Mundane
weapons. Every poison and bleed term disagrees: the site's base is this model's divided by 5 and its
coefficient this model's divided by 2 (Bandit Axe Poison: 140 and 0.525, against 700 and 1.05). For the
status terms the builder's are the ones a hit carries ("Status build-up per hit" above).

The damage terms mostly agree. An earlier count of 509 disagreeing physical bases and 433 STR coefficients
included shields, which the site gives no attack. Without shields, and with Mundane given no STR/DEX term
as the scaling function gives it none, 84 of 2115 infusion rows differ by more than the site's rounding
(a base by 1 or more, a coefficient by more than 0.011). The rate move is now read to the end ("The count", below),
and where the site disagrees its own rows contradict it. Heide Lance Dark has lightning 60 and dark 90
where this model has 90 and 60, while the site's Lightning row matches this model. The Ivory King Ultra
Greatsword Fire row has this model's magic 65 and bleed 114, but physical 217, the Magic row's (217.5),
where the move gives 245. Murakumo's uninfused STR/DEX are 0.07/0.51 against the regulation's 0.15/0.59,
and its name has one `WeaponReinforceParam` row. So `scripts/ds2-builds-recommend.py` takes every
infusion's base and coefficients from the regulation (`regulation_attack`). Shields and names with no
regulation row keep the site's.

**Scaling function `0x1403903b0`** (Arxan entry, body from `0x141cf33c8`). Arguments are `(this, ws, out[10])`.
`ws+0x30` is the reinforce level, `ws+0x31` the infusion index, `ws+0x40` the WeaponStatsAffectParam row.
`rsi = 0x140397c40(row, level) = row + 8 + level * 0x24`: the 9 coefficients for that upgrade level.

```
infusion 9 (Mundane):  out[0] = bonus[2] * coef(ws+0x50)      0x141bf101e, 0x141b73fdf
otherwise:             out[0] = bonus[0]*c[0] + bonus[1]*c[1]    0x141b499a4, 0x141c61b8e
    infusion 8 (Enchanted) also adds bonus[3]*c[8]               0x141c61b97, 0x1406dcfc2
out[1] magic     = bonus[3] * c[2]                              0x141c6752f
out[2] lightning = bonus[5] * c[3]                              0x141aff3b7
out[3] fire      = bonus[4] * c[4]                              0x141b580be
out[4] dark      = bonus[6|7|8] * c[5]  (index by ws+0x70)       0x141c37eb3
out[5], out[6]   = status bonus (0x14038d240) * c[6], c[7]       0x141ca3259, 0x141cf178c
```

`c[]` is `WeaponStatsAffectParam` `physicalByStrength, physicalByDexterity, magic, thunder, fire, dark,
poison, bleeding, physicalByEnchant` for that level. The row is `WeaponReinforceParam.statsAffectId + t[inf][1]`
(`0x14034ec60`: `movzx edx,[rax+1]; add edx,[rbx+0x4c]`). `t` is the 10x3 byte table at `0x1410c3e10`.
The Mundane coefficient (`0x1401641c0`) is `byte[obj+0x13a] * 0.01` of the row at `ws+0x50`, or 1.0 when
`thunk 0x140358de0(CharacterManager, 0x14) < 25`. That row is the weapon's `WeaponTypeParam`, and `+0x13a` is its
`abyssRate` (read 2026-09-29).

**Base, rates and the sum.** Loop `0x141b8ba3d`..`0x141b8ba7f`, over 10 damage types:

```
AR[i] = max(0, ((bonus[i] + base[i]) * rate[i] + D[i]) * k)
base  = lerp(WRP.min[i], WRP.max[i], clamp(level / WRP.maxLevel)) * WSA.baseValueScale   0x14034e820, 0x141b11a0b..a1c
rate  = WRP.physicalRate.. (+0xa0), with the infusion's add rate moved in, * 0.01, floored at 0
        add rate = WRP[+0xc4 magic,+0xc8 thunder,+0xcc fire,+0xd0 dark,+0xd4 poison,+0xd8 bleed,
                   +0xdc crude,+0xe0 enchanted,+0xe4 abyss] picked by infusion   0x14034fe10
        target element t[inf][0] += add (cap 1000); every other nonzero element -= add / count   0x14034fed0, 0x14034ff74
bonus = the scaling function above, plus SpEffect weapon-attack adds (0x14038f3b0, kind 5/6 per hand; 0x140391020)
```

The move, read 2026-09-30. The builder reaches it as `0x14034c580` -> `0x14034c760` -> `0x14034c5c0`,
and `0x14034c760` passes `1.0` as the factor `0x14034fe10` multiplies the add rate by. `count` is the
number of nonzero rates other than the target. The target gains `add` only while it is under the cap
(1000.0 at `0x1410ad5f0`), and only rates above zero lose `add / count`. `0x14034c580` then floors all
ten at 0.

The count, read 2026-09-30. `0x14034c5c0` (body at `0x141c4f18a`) copies the nine rates into ten slots,
with slot 7 (durability) set to 0 (`0x141c4f1b9`..`0x141c4f1fd`). It counts the slots that are not 0.0
(loop at `0x141c5f5da`) and passes that count to the move (`0x141b75f8e`). So `count` includes the
poison, bleed, petrify and curse rates as well as the five damage types.

`0x14034c5c0` has two other callers, `0x14034f8c4` and `0x14034f94e`. Both are in one function, and it
moves `WeaponReinforceParam+0x50`, the nine per-type values ahead of the stability rates, not `+0xa0`.
It then pushes each value toward 100: `x + (100 - x) * (level term + stat terms)`, where the stat terms
are six calls to `0x14034fd40`. Each call uses one of this level's STR, DEX, INT (physicalByEnchant for
Enchanted), FTH, fire and dark coefficients. So the attack rates have one move, and
`scripts/ds2-builds-recommend.py` `infused_rates` is it.

On a catalyst this is the whole infusion. The elemental rows `statsAffectId + 1..4` repeat the
standard row's `baseValueScale` (1.0) and coefficients. So a Magic Staff of Wisdom casts sorceries at
magic rate 110 (add 10). A Lightning one goes to magic 90 and lightning 10, and a Lightning Dragon
Chime to lightning 115 and dark 85. Checked against SoulsPlanner's stored infused catalysts
(`scripts/ds2-builds-recommend.py` `infused_rates`): 192 of 194 base and scaling terms agree (base
within its integer truncation, scaling within 0.011). The two misses are the Blue Flame's Lightning
and Dark scaling, 0.15 here against the site's 0.135.

The infusion indices follow from the add-rate offsets and the target elements: 0 none, 1 Fire, 2 Magic, 3 Lightning,
4 Dark, 5 Poison, 6 Bleed, 7 Raw, 8 Enchanted, 9 Mundane. For the Dagger (+10, reinforce 1000,
statsAffectId 1005030), this model gives base 115 / 80.5+80.5 / 132.25 / 57.5 for
Standard / elemental / Raw / Mundane. Coefficients come out as 0.15/0.45, then 0.0565/0.169/0.3, then 0.038/0.113.
SoulsPlanner has 115 / 80+80 / 132 / 57 and 0.15/0.45, 0.06/0.17/0.3, 0.04/0.11, so it truncates base to an integer and
rounds coefficients to 2 dp. SoulsPlanner's Enchanted STR 0.06 is not the regulation's 0.053.

This loop is inside `0x140391fe0`, the attack builder for every player attack (melee, arrows and spells;
see "Spell and buff attack" below), so it is the path a hit takes, not a menu-only path. `D[i]` is the
attack's damage row `damage01..03` of type `i` (`0x14038fee0` / `0x14038f2d0`). `0x14038fee0` zeroes ten
floats, converts each `damage0n` (+0xc, +0x10, +0x14) to float and hands it with its `damageType0n` byte
(+0x19, +0x1a, +0x1b) to `0x14038f2d0`, then adds the ten into the attack. `0x14038f2d0` is a jump table
at `0x14038f370`: types 0-4 (physical, magic, lightning, fire, dark) add to slots 0-4, 6 poison to slot 5,
7 bleed to 6, 8 durability to 7, 12 petrify to 8 and 9 curse to 9. Types 5, 10 and 11 (toxic) are
dropped, so a row's toxic add reaches no attack. In the regulation, 71 damage rows of named weapons'
live hits add physical attack this way (Santier's Spear two-handed R1 +130 on one of its two
WeaponParam rows, Smelter Hammer +100, Bone Fist up to +120, Ricard's Rapier R2 +40 and +90, Syan's
Halberd one-handed R1 +30), the Bat Staff's R2s add 700-1000 poison, and Mytha's Bent Blade's +300 is
toxic. `k` = (1 - the
PlayerLackOfStatsParam penalty, `0x14038f6a0` -> calculator slot 13 = 1 - slot 9 `0x1403811f0`) x
`WeaponTypeParam.rightDamageScale` or `leftDamageScale` x `ChrParam.damageAdjustRate`. Over the 334 named
weapons in the regulation, the right-hand scale is 1.0 for 315 (0.9 for 18, 0.865 for one) and the left-hand
one 0.9 for 303; every catalyst's right-hand scale is 1.0, and `ChrParam.damageAdjustRate` is 1.0. Not read:
the extra rate edit at `0x14038f440` beyond the buff below.

### Two-handing (EXE)

Read 2026-09-30. Two-handing does not multiply Strength for attack rating. What it does change:

| Two-handing | Read at |
|---|---|
| halves the weapon's Strength requirement (`shr cx,1` for grip 2 or 3), which reaches the attack only through the lack-of-stats factor `k` | `FUN_14034a980` -> `0x14034d3c0`, `0x14034ce60`; the attack reads the cached ratios through `0x14031fcb0` |
| attacks from the two-handed `atkId` slots, with their own damage rows (section 1) | REGULATION |
| guards with `WeaponTypeParam.stabilityAddRateSingle2Handed` (`+0xd4`) | `0x14034d6c8` |
| multiplies knockback by `DamageStyleRateParam.rate`, 1.5 on the two-handed rows (`styleRateType` 1) | `FUN_1403eee20`, `FUN_1403ee840` |

Why no Strength multiplier can reach the attack:

- The scaling function `0x1403903b0` reads the Strength bonus from the one player stat object,
  `[chr+0x490]` -> `0x14038b990` (`+8`), at `+0x50` (`0x14038d260`). Its only compares are on the
  infusion (`ws+0x31` = 8 or 9), and it adds the Strength and Dexterity terms into one physical value,
  so nothing after it can scale Strength alone.
- That bonus is `physicalAttackByStrength` of the `PhysicalStatsPerLevelStatValuesParam` row for the
  effective Strength (`chrStatus+0x16`). The stat builder `0x14038d790` writes it, and all three of its
  calls (`0x14038d644`, `0x14038d6f4`, `0x14038c1b5`) pass that block. The row lookup `0x140358b60` is
  called only by the builder and its stat-1 and stat-99 helpers: every `scripts/ds2-xrefs.py` hit is in
  `0x14038c2ab`..`0x14038e24f`.
- The effective block is `clamp(base + modifier, 1, 99)` (`0x1401ffcc0`), and it is what the attributes
  menu shows (`docs/DS2-ITEM-REQUIREMENTS.md`). The modifier's eleven signed bytes (`source+0x30b`..
  `+0x315`, read by `0x14014cfe0`) line up one for one with the SpEffect `1000[0]` stat kinds 6 to 16.
  The handle comes from `0x14014bb40`, which sits among `ChrSpEffectCtrl`'s methods (vtable
  `0x1410bfee0`) and reads the same `+0x10` member as that class's destructor. That the modifier is the
  SpEffect stat total is inferred from this shape, not read from the code that fills it.
- No SpEffect raises Strength when the grip changes. Across the eleven `SpEffect*.emevd` scripts,
  `1000[0]` kind 10 (Strength) is in five events, all items: ring 41010000, armour 21650100 and
  21650110, weapon 11320000 and active item 60235000. No `1000[19]` scales Strength. The grip writer
  `FUN_14034f470` sets two dirty flags and sends `ChrEquipPacket_weaponStance` (`FUN_1401628d0`), and
  applies no SpEffect.
- The attack builder `0x140391fe0` and the functions it calls make no ChrAsmCtrl (`PlayerCtrl` slot
  `0x120`) or equip-object (`ChrAsmCtrl` slot `0x70`) call. The equip-object getters that `0x14031fcb0`
  uses (`0x1403491e0`, `0x140349100`, `0x140349170`) return the cached lack-of-stats ratios.

**`styleRateType` and `attackTypeRateType` scale knockback.** `FUN_1403edc70` reads `DamageParam`
`+0x41` (directionType), `+0x5e` (movementPowerType), `+0x5f` and `+0x60`. `FUN_1403edee0` builds a
normalised direction for the directionType. `FUN_1403eee20` multiplies it by the `rate` of
`DamageMovementPowerParam`, `DamageAttackTypeRateParam` and `DamageStyleRateParam` (lookups
`0x1403b5640`, `0x1403b54d0` and `0x1403b5780` on `DamageMan`) and by `FUN_1403ee490`, then hands it to
`FUN_1403efc30`. When a hit breaks an object, `FUN_1403ee840` uses the three params' `break_rate`
instead (`FUN_1403ee320`, `FUN_1403ee2e0`, `FUN_1403ee360`). Nothing else calls the three lookups.

## Ranged attack: bows, greatbows, crossbows

Read 2026-10-01; the call sites were read by a sibling investigation of the Sanctum Crossbow the
same day. Rows re-print with `scripts/ds2-ranged-evidence.py ammo|launchers|scales|baseless`.
`scripts/ds2-builds-recommend.py` scores a launcher this way (`regulation_ranged`,
`ranged_options`, `ranged_value`).

```
shot attack[e] = (launcher AR[e] + ammo row damage0n of type e) x WeaponTypeParam.rightDamageScale
shot damage    = hit_damage(shot attack, defense, MV = ammo row damageRate, damageLower) x WeaponParam.damageScale
```

| Fact | Tag | Re-print |
|---|---|---|
| A launcher is a WeaponParam row whose WeaponTypeParam `shootCategory` is 1 (bows), 2 (greatbows) or 3 (crossbows) | REGULATION | `launchers` |
| Ammunition has no WeaponParam row: no base, no scaling, no upgrade. Its attack is its ArrowParam row's PlayerDamageParam damage ids | REGULATION | `ammo` |
| ArrowParam has four (light bullet, heavy bullet, light damage, heavy damage) sets. Arrows and greatarrows fill set 1 only; bolts fill sets 0-2 alike (MV 0.392) and set 3 at MV 0.247 | REGULATION | `ammo` |
| A bow reads set 1 | INFERRED: the only set with damage rows | |
| Which crossbow reads set 3 | UNPROVEN (guess: the volley crossbows, Avelyn and Sanctum Repeating) | |
| Arrows: Wood +0, Iron phys +30, Magic/Fire/Dark +110, Lightning +90, Poison poison +1200, Lacerating bleed +1500; MV 1.176, damageLower 70 | REGULATION | `ammo` |
| Greatarrows: Iron phys +50, Lightning +150, Fire +100, Destructive phys +100 (and +100 durability, type 8); MV 1.176 | REGULATION | `ammo` |
| Bolts: Wood phys +200, Heavy phys +280, Magic/Lightning/Fire/Dark +200 element and +200 phys; MV 0.392, damageLower 34 | REGULATION | `ammo` |
| A normal shot builds its attack with the per-hand builder `0x140391fe0` (call `0x14005210b` -> `0x140392d51`, type mask off), handed the ArrowParam light bullet `[rdx]` and damage id `[rdx+8]` | EXE | |
| So the shot carries the launcher's whole attack in every type it has, infusion included, plus the ammo row's flat per its types (`D` in the loop above, `0x14038fee0`) | EXE | "Base, rates and the sum" |
| `k` holds the hand's `rightDamageScale`: 0.9 on every launcher, 0.865 on Avelyn, 1.0 on every melee weapon (one shield, Orma's Greatshield, is 0.9) | REGULATION (values), EXE (that `k` holds it) | `scales` |
| The ammo row's `damageRate` is the motion value after the defense, as for a melee hit | INFERRED from the shared builder and hit path | |
| `WeaponParam.damageScale` (1.25 on Light, Heavy, Shield and Sanctum Crossbow; 0.85 Dragonrider Bow) applies after the defense on a shot | INFERRED: read for melee (`0x141c5ccfc`), not traced on the shot path | |
| ArrowParam `menu*Attack` (Wood Arrow 50, Heavy Bolt 100) is not the shot's attack | INFERRED display-only | |
| A bow fires two-handed only | INFERRED: its one-handed motions' damage rows have damageRate 0 | `launchers` |
| An arrow's slash/strike/thrust type | not read; scored against general physical defense | |
| Fire rate | not read: the bow shot animations (`40230010`, `41230010`) are not in the unpacked `c000100_pl.tae`, and the aim/fire/reload loop is EzState's. Scored per shot | |

**The Sanctum Crossbows and the Bow of Want have a special shot**, their two-handed L2
(`WeaponActionCategoryParam.atkIdOppositeSingle2HandStrong`), which fires the weapon's own bullet
(REGULATION):

| Weapon | Motion | Bullet damage row | Shots |
|---|---|---|---|
| Sanctum Crossbow | 20090100 | 20090100: dark +50, MV 7.2, damageLower 255 | 1 |
| Sanctum Repeating Crossbow | 20090000 | 20090000: dark +5, MV 1.85, damageLower 130 | 3 (`BulletParam.automaticShootNum`, 0.1 s apart) |
| Bow of Want | 20071900 | 20071900: lightning +200, MV 1.2, damageLower 70 | 1 |

That attack's builder call (`0x1406d5502`) sets the type mask `0x1403910f0`, so only the row's own
types keep their rate (EXE; that this call is the special's is INFERRED from the call site). The
Sanctum Crossbow's special is therefore pure dark: its dark attack plus 50. No bolt is fired, so the
ammunition does not change it (INFERRED). Players say the Sanctum Crossbow deals pure dark with a
Dark infusion and dark bolts (COMMUNITY); the rows say the pure-dark part is the special, and a
normal shot carries the launcher's physical too.

**A type with a rate and coefficient but no base still attacks.** Both Sanctum Crossbows have
`darkRate` 100, `maximumDark` 0 and a dark coefficient of 0.4 (REGULATION, `baseless`; no other
named weapon has such a type). `AR = (bonus + base) x rate` gives them dark from INT/FTH alone, and a
Dark infusion moves the rates to physical 70 / dark 130. `attack_rating` built a type only from its
base until 2026-10-01, so it gave these two no dark at all.

## Stamina: what an attack costs, and what a bar buys

Read 2026-10-01. Rows re-print with `scripts/ds2-stamina-evidence.py --categories --chr <names>`.
`scripts/ds2-builds-recommend.py` scores it this way (`regulation_stamina`, `attack_stamina`,
`shot_stamina`, `max_stamina`, `bar_attacks`, `stamina_metrics`; `--rank per-stamina|bar`).

```
R1 cost (menu) = WeaponParam.meleeAttackBaseCost x WeaponStaminaCostParam[costCategoryId].<slot> / 10
                 slot: Nnormal1st1H / normal2nd1H, or normal1st2H / normal2nd2H, alternating down the chain
shot cost      = WeaponParam.rangedAttackBaseCost / 10
max stamina    = PhysicalStatsPerLevelStatValuesParam.staminaMax[effective END] x ring stamina factors / 10
```

| Fact | Tag |
|---|---|
| Base costs: Dagger 144, Uchigatana 252, Giant Warrior Club 374 (melee); Long Bow 346, Light Crossbow 410, Dragonslayer Greatbow 1012 (ranged) | REGULATION |
| Category 10 (454 of 473 rows): 1H R1 0.95 / 1.05, 2H R1 1.187 / 1.312, 1H R2 1.4, 2H R2 1.75. So a 2H Giant Warrior Club R1 pair costs 44.4 + 49.1, a 1H Dagger pair 13.7 + 15.1 | REGULATION |
| That the game multiplies base by the category field | INFERRED from the field names (JP: "attack stamina multiplier, one-handed weak 1"), not traced. Where the EXE takes a per-hitbox stamina amount (`ChrAttackDamageCtrl` slot `+0xf08 + i*0xc0 + 0xb8`, read by `0x1403741c0` and drained from `parseDamageTae` `0x140326ddd`), its setter `0x1403748c0` jumps into an Arxan stub | |
| The /10: the regulation's staminaMax is ten times the menu's, matching SoulsPlanner | REGULATION + SITE; a ratio of the two does not depend on it |
| A shot reads `rangedAttackBaseCost` with no category multiplier | INFERRED: every launcher names category 10, whose fields are melee slots; category 20 ("bow") is named by no weapon |
| `drainStamina` `0x1403335d0`: a drain that crosses 0 is not refused; stamina becomes the drained value clamped to [`staminaLoanMax` -150, `staminaLoanMin` -25] and the out-of-stamina timer starts | EXE (code), REGULATION (ChrParam 100) |
| So an attack starts while stamina is above 0 (the bar's last swing may overdraw it) | INFERRED from the above |
| Every one of the 372 player TAE animations with a 2200 hitbox has a 101100 event; its handler `parseStaminaTae` `0x1403287b0` holds `disable_stamina_regen` while it is open | TAE + EXE |
| So an R1 chain regenerates nothing between swings | INFERRED: that one attack's 101100 reaches the next attack's start was not measured per weapon |
| Regeneration itself (`0x140333880` and caller): `ChrParam.staminaRecoveryValue` 519 x factors x dt | EXE (code), REGULATION (value); not used by the ranking |

**What the recommender ranks.** `--rank window` (the default, unchanged) is the R1 hits landed in
`--window`. Every `--best-weapons` row now also carries `metrics`: `stamina_per_attack`,
`stamina_per_window` (the attacks whose hits the window counts), `damage_per_stamina` (that window's
damage over it), `max_stamina` and, from a full bar, `bar_attacks`, `bar_damage`, `bar_seconds`.
`--rank per-stamina` ranks by `damage_per_stamina`; `--rank bar` by `bar_damage`, the damage of
the R1 chain a full bar of the build's own max stamina pays for. The build is still the one
optimized for one hit: END is wherever the floors put it, so `bar` reads that END.

## Spell and buff attack (EXE)

Read 2026-09-29 through the Ghidra daemon and `scripts/ds2-arxan-trace.py`. A spell's attack is built
once, when the cast is prepared, by the same per-hand builder as a melee attack, and is carried by its
bullet unchanged to the hit:

- A per-frame update (`0x14038f200` -> `0x140393a30`) calls `0x140393c00` per hand. Its case 12 (spell)
  calls `0x1403936a0`, which takes the bullet and damage ids from `SpellParam` `+0xc`/`+0x10` (or the
  left-hand ids) through `0x140390050`, then `0x140391bf0`, which calls the builder `0x140391fe0` for
  player-type characters (byte 3 of the table at `0x1410bfff0` for the type at `chr+0x54`) with the
  rate mask on. `0x1403747b0` stores the result in `ChrAttackDamageCtrl +0x760 + slot*0xc4`; the TAE 2300
  event copies it into the bullet (`0x1403740e0` -> `0x140373880`). Nothing is recomputed at the hit.
- In the builder, per type `e`: the catalyst's rate `rate[e]` is kept only when `e` is one of the spell
  row's `damageType01..03` (`0x1403910f0`), so no physical attack leaks into a magic spell. The attack is
  then the loop above, `((bonus + base) * rate + D) * k`, with the catalyst's base, scaling and rate and the
  spell row's `damage01..03` as `D`. Every catalyst's `WeaponParam.damageScale` and right-hand
  `WeaponTypeParam` scale is 1.0 in the regulation, so a catalyst whose requirements are met casts with
  `k = 1`: **a spell's attack in its element is the catalyst's attack rating in that element plus the
  row's flat damage**, and its damage is that times `damageRate` after the defense
  ("Where the attack and the motion value enter" above).
- The attack block's `+0x6c` (`sAr+0xb4`, which `calculateDamage_attack` multiplies in) is
  `ChrStatus_Rate[25]` (`flags+0x420`). Its `+0x70` (`sAr+0xb8`, multiplied in after the defense) is
  `flags+0x48c + 4*spellCategory` times the catalyst's `damageScale`. Their writers were not read; both
  are 1.0 unless an effect changes them. That the block lands at `sAr+0x48` is INFERRED from every
  field lining up; the copy itself was not read.

A weapon buff goes through the same builder. Its `100090[2]` (`SpEffectActionImpl_ChangeAtkDef` slot 19,
`0x14021b230`, mode 1) adds `flat` to the element's stat bonus (`100090[3]` subtracts; `[4]` and `[5]`
are defense). Its `100080[1]` (handler `0x14022c4c0`) stores `X * m1` in a `SpEffectEach_Property`
slot, and `0x14014bb90` -> `0x1402259f0` adds it to the weapon's rate in that element as `X / 100`; `m1`
comes from an active `100080[2]` and is 1 without one. A standard weapon's elemental rates are 0, so a
buffed weapon gains `(its own elemental base + coefficient * stat bonus + flat) * X / 100` in the buff's
element -- the weapon's `WeaponReinforceParam maximum<Elem>` and `WeaponStatsAffectParam` coefficient,
nothing of the catalyst's. For a +10 Dagger (base 115 in every element, coefficient 0.4) that is about 22
magic from Magic Weapon (X 15, flat 30) and 39 from Crystal Magic Weapon (X 30, flat 15) before the
INT term. Which hand a buff lands on depends on a key byte at `+0x2e`; where that key comes from is
INFERRED. Resins were not read.

Buffs in the regulation (`SpEffectSpell.emevd`, event id = spell id):

| Spell | Element | X (100080[1]) | flat (100090[2]) | Seconds |
|---|---|---|---|---|
| Magic Weapon | magic | 15 | 30 | 90 |
| Great Magic Weapon | magic | 20 | 30 | 90 |
| Crystal Magic Weapon | magic | 30 | 15 | 90 |
| Sunlight Blade | lightning | 30 | 15 | 90 |
| Flame Weapon | fire | 20 | 15 | 90 |
| Dark Weapon | dark | 30 | 15 | 90 |
| Resonant Weapon | dark | 35 | 0 | 60 |

**Child bullets** (read 2026-09-30). `0x140445ec0` spawns a bullet's children. It reads
`BulletParam` `childeBulletId`/`childeDamageId` at `+0xd4`/`+0xd8`, `+0xdc`/`+0xe0` and
`+0xe4`/`+0xe8`, and builds each child's spawn request with a copy of the parent's attack block
(parent `+0x80..+0x118`, request `+0x30`) and the child's damage id. What happens to that block
depends on the owner:

- **A player-type character** (owner handle type 2, and byte 3 of the `0x1410bfff0` table set, the
  same test as the builder above). The child keeps the parent's block whole. Only slot 5 is replaced,
  by the child row's `damage0n` of type 5.
- **Anyone else.** The block's first 0x34 bytes are zeroed, and each of the child row's `damage01..03`
  is added into the slot of its `damageType0n`.

That second path writes the child row's damage into the same slots, by damage type, that the player
path copies from the parent. So those slots are the per-type attack the hit reads. A player's child
bullet therefore hits with the attack built at the cast: the catalyst's rating plus the spell row's
flat damage (`SpellParam.baseSpellDamageId`), in that row's types. The child's own row supplies
only what is read at the hit, its `damageRate` and `damageLower`.

For 33 of the 55 damaging spells the biggest hit is on a child row. Most child rows repeat the spell
row's type and flat damage. Four do not:

| Spell | Spell row | Child row the model used before |
|---|---|---|
| Wrath of the Gods | lightning 300 | lightning 0 |
| Combustion | fire 50 | fire 0 |
| Great Combustion | fire 50 | fire 0 |
| Outcry | fire 0 + dark 100 | fire 0 |

The three spells whose damage an SpEffect event spawns (`SCRIPTED_SPELL_BULLET`) have no parent
bullet, and how their attack is built was not read.

**Soul-consuming spells** (read 2026-09-30). `0x140390050`, which hands the builder a spell's bullet
and damage ids, looks up `SpellParam.soulConsumeParamId` (`+0xfc`) in `SpellSoulConsumeParam`
(`CharacterManager+0x550`, `0x140359000`). A spell with such a row does not use its own ids. It
takes one of five `bulletAndDamageId0N` pairs (the `Left` set for the left hand) by
`r = (consumeSoul - souls held) / consumeSoul`, where souls held is `[chr+0x490]+0xec`:

| `r` | Souls held | Pair |
|---|---|---|
| r <= 0 | at least `consumeSoul` | 01 |
| 0 < r <= 0.2 (`0x1410ad5e0`) | at least 80% | 02 |
| 0.2 < r <= 0.6 (`0x1410e548c`) | at least 40% | 03 |
| 0.6 < r < 1 | some | 04 |
| r >= 1 | none | 05 |

Climax (`consumeSoul` 5000) has pairs 35050000..35050004 at damageRate 6.75 / 6.0 / 4.0 / 2.5 /
0.15, and its own `baseSpellDamageId` is pair 01. So the regulation's spell row, which the model
uses, is the damage of a caster holding at least 5,000 souls. Every soul-consuming spell's own row
is its pair 01. The thresholds are Resonant Soul 100, Great Resonant Soul 500, Resonant Flesh and
Resonant Weapon 2,000, Lifedrain Patch 3,000, and Dark Dance and Climax 5,000. Climax alone has
`consumeType` 1, which is read beside `r` but does not pick the pair.

Not read: the inputs of the stat-penalty descriptor (`0x14031fd10`, `0x140333790`, `0x14038fb10`),
the writers of `flags+0x420`, `+0x428` and `+0x48c..+0x49c`, what `consumeType` and a cast do to the
souls held, and the Arxan-wrapped lookups `0x1403b56d0` / `0x1403b5500`.

### Which catalyst casts it

Read 2026-09-30. `0x140397a30(spell, weaponType)` is the whole check: it returns false for a null row,
switches on the spell row's first int (`SpellParam.spellCategory`) and returns whether one byte of the
WeaponTypeParam row is nonzero. Any category outside 0..4 cannot be cast.

| `spellCategory` | WeaponTypeParam byte | Paramdef name | Catalysts in the regulation with it |
|---|---|---|---|
| 0 | `+0x0` | `allowMagic` | 13 staves (Pilgrim's Spontoon among them), Staff of Wisdom, Blue Flame, Sorcerer's Twinblade, Black Witch's Staff, Sanctum Shield |
| 1 | `+0x1` | `allowMiracle` | 11 chimes (Mace of the Insolent among them; not Caitha's Chime), Black Witch's Staff, Sanctum Shield |
| 2 | `+0x2` | `allowPyromancy` | Pyromancy Flame, Dark Pyromancy Flame |
| 3 | `+0x3` | `allowDarkMagic` | the 13 staves, Black Witch's Staff, Sanctum Shield |
| 4 | `+0x53` | `allowDarkMiracle` | the 11 chimes, Caitha's Chime, Black Witch's Staff, Sanctum Shield |

So hexes come in two lists that only the Black Witch's Staff and Sanctum Shield both cast. Caitha's
Chime casts only category 4, and the Staff of Wisdom, Blue Flame and Sorcerer's Twinblade cast no hex.
Of SpellParam's rows, 25 are category 3 and 22 category 4.

The following reads show which rows the two arguments are:

- The weapon row is the hand's `[equip + slot*0x48 + 0x218]`. `0x140349fc5` stores there what
  `0x140359210` returns, and `0x140359210` looks the id up in `CharacterManager+0x450`, the
  WeaponTypeParam container. The per-hand getter `0x140349360` -> `0x14014cbeb` reads it back.
- The spell row is the attuned slot `0x140348eb0` returns. Its `+0x8` and `+0xa` are checked against
  stats 6 and 7 as INT and FTH (`0x1403978c0`), and its `+0x4` is read as `isDual2HandedSpellAllowed`
  (`0x140395930`), which is SpellParam's layout.

Two paths reach the check:

- **Casting.** `0x140395930` handles the per-hand attack kind 12, the same spell case as the builder
  above. It calls `0x140396ba0` -> `0x1403963b0`, which checks casts left (`0x1403487c0`), INT/FTH
  (`0x1403978c0`), then the hand's catalyst (`0x140396510` -> `0x140397a30`, calling `0x140395f40`
  when it fails), then `0x140397a00`.
- **The HUD's spell icon.** `FeScenePanelLEquip` `0x14050c940` -> `0x1401ffeb0` runs the same checks
  for one hand, then the other.

`0x140347e00` returns a two-bit mask of the hands whose weapon type has any of the five bytes set.
`scripts/ds2-builds-recommend.py` `SPELL_SCHOOLS` is this table.
Not read: `0x140397a00` (`0x1401a46a0`), and the two bytes from `0x14019e110` that `0x1403978c0`
subtracts from the INT and FTH requirements.

**Delta vs `scripts/ds2-builds-recommend.py`** (`attack_rating`, `hit_damage`, `damage`, `build_defense`):
- AR: the same `(base + sum bonus*coef) * rate` structure, with `rate` folded into `atk`/`atkScale` as
  the regulation gives it, unrounded (`regulation_attack`). Mundane's `modifier` is the game's
  (`abyssRate`, above). Shields and weapons with no regulation row keep SoulsPlanner's numbers
  ("Base and coefficients against SoulsPlanner", above). An Enchanted weapon that also deals magic scales
  that magic by its physical INT coefficient, because the port has one `magic` key for both.
- Bleed and poison scaling: the site's table and its `3*DEX + FTH` / `3*DEX + ADP` index are the game's
  ("Status bonus per stat", above). The base and coefficient they combine with are the regulation's,
  in gauge points per hit (`regulation_status`, "Status build-up per hit"); the victim's resistance is
  not modelled.
- Elemental cut `min(0.99, (D+100)/1000)` matches. Not modelled: the lack-of-stats factor on armor, and the
  cap gate.
- Physical stat defense: the planner's table is off by one at 180 of 393 sums. The game uses
  `row[trunc(sum/4)].defense`.
- Per hit: the damage row's flat attack (`regulation_hit_flat`, added before the defense) and
  `WeaponParam.damageScale` (`regulation_damage_scale`, after it) are modelled. Not modelled: the
  attacker's `flags` rates on `sAr+0xb4` (1.0 without a SpEffect), the counter and critical-hit
  factors, and the hand's `live` add.
