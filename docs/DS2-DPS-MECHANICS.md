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
  REGULATION. What these multiply was not traced. The 1.5 is consistent with the 2H 1.5x STR, but that is unverified.
- **Poise damage (base)** = `DamageCtrlParam.poiseDamage`, row `PlayerDamageParam.damageCategory`.
  Weapon factor = `WeaponParam.poiseDamageScalePlayer`. See section 2 for the EXE formula.
- **Stamina cost** = `WeaponParam.meleeAttackBaseCost * WeaponStaminaCostParam[costCategoryId].<slot> / 10`.
  This is from `docs/DS2-BUILD-MECHANICS.md`: the field values are REGULATION, and the /10 is INFERRED.
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
  + int(`s8 status+0x305` + `s32 status+0x3a0`). EXE. That the vcall sums `ArmorParam.strong` over the
  worn pieces, and that the two status ints hold the Ring of Giants/SpEffect poise, is INFERRED.
  The armor values are REGULATION: Havel's 19+51+31+31 = 132; Alva 6/15/4/9; Drangleic Mail 28; max single piece 83.
  Ring of Giants +10/20/30 comes from `DS2-BUILD-MECHANICS.md`.
- **regeneration** `0x140145ba0`: `current += rate * dt`, clamped. `rate = ChrParam.poiseRecoveryValue`
  (+0x11c) = **0.56** for the player. EXE for the code and REGULATION for the value. That dt is seconds is INFERRED, so the rate would
  be 0.56 poise/s. Regen is suspended while `status+0x7e6 != 0`.
- **reset after a break** (EXE): if `status+0x7e6` was set on the last frame, it is now clear, and current <= 0, current is set to max.
  Most likely +0x7e6 is the "in stagger/damage state" flag (INFERRED). Result: once poise is broken it comes back
  full when the stagger ends. Partial damage otherwise regenerates only at 0.56/s.
- TAE event **111200** (handler `0x14032678d`) sets `status+0x610`. While it is set, regen uses `changePoiseRecoveryValue`
  (0 for the player), and poise is refilled to max on entering and on leaving the window. EXE. No weapon attack
  in the dump uses 111200.

### Attacker poise damage (EXE)
`ChrDamageActionCtrl::v36` `0x140139160` computes the base poise damage of a hit. The attack packet must be type 1, otherwise the result is 0:
```
base = (DamageCtrlParam.poiseDamage[+0xa0] * hit[+0x74] + vcall(defender+0x398)->[0x188](int) + s8 hit[+0x70])
       * (hit[+0x90] ? u16 [hit[+0x90]+8] * 0.01 : 1.0)
```
Whether `hit+0x74` is `WeaponParam.poiseDamageScalePlayer` is INFERRED. The name fits and the values give dagger 35 and OKH 160 per R1.
The other terms are not identified.
The defender side (`ChrDamageActionCtrl::v14` `0x140137aa0`) multiplies the base by `(1 + pct*0.01*k)`, where pct comes
from an unidentified source. It then applies hyperarmor (next subsection). It subtracts the result from current poise in
`0x140145970`, called from `0x14013639d`.

### Hyperarmor (EXE)
- TAE event **111900** (handler `0x140326884`) looks up the chr's current weapon row and copies its
  `+0xb8` into `status+0x7d0`. +0xb8 is `WeaponParam.uninterruptibleRate`; that the looked-up row is WeaponParam
  is INFERRED from the offset.
- Defender side `0x140137f02`: if `status+0x7d0 > 0`, incoming poise damage is multiplied by uninterruptibleRate.
  For OKH (0.3) that means it takes 30% of the poise damage during its own 111900 window.
- Stagger decision `0x140136689`: the flag starts as `poise > 0` after the hit. A hit whose
  `DamageCtrlParam.armorBreak` (INFERRED to be the byte at +2) is 2 always staggers. If it is 1, it staggers unless the defender has
  uninterruptibleRate > 0.
- Windows (TAE): OKH R1 111900 frames 20-30 around its 23-28 hit; 2H R2 20-30. Dagger R1 has no 111900 and its
  rate is 0, so a dagger has no hyperarmor. `WeaponTypeParam.toughnessPeriodScale` is 1.0 for all 144 rows, so it does nothing.

### Hitstun and "true combos" (unverified)
- The stagger animation is chosen by `DamageCtrlParam.damageMotion`: dagger R1 = 2, R2 = 3, OKH 2H R2 = 8.
  Those animations and their lengths were not mapped. Candidate damage anims sit in the TAE's `000160xxx`
  block (INFERRED). `DamageParam.stiff` (dagger R1 1.8 s, OKH R1 1.0 s) is unexplained.
- What the data does support about the user's claim: a dagger R1 does about 35 poise damage (INFERRED
  scale). So any defender whose current poise is 35 or less is staggered by every dagger R1.
  Most light PvP builds have 0-30 armor poise, and their poise is refilled after each stagger.
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

## 5. What remains unverified
- That `hit+0x74` is `poiseDamageScalePlayer`, and the other terms in the poise formula.
- That the regen dt is in seconds.
- The damageMotion -> stagger animation mapping and its length, which decides true combos.
- Where startPlaySpeed switches to endPlaySpeed.
- That 111500 is the chain window.
- What styleRate and attackTypeRate multiply.
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
resolved below: it is `(D + 100) / 1000` for the defense D that SoulsPlanner shows. Not traced: whether
remote PvP hits take this same path.

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
- `0x14034f2c0`, called on each piece's vector, is Arxan-chained and was not read. The xorps
  `xmm1,0` at `0x1400399c4` suggests a floor at 0.
- The SpEffect term is added inside the per-piece loop (`0x140381510`). Whether that really counts a
  ring's elemental bonus once per armor slot needs runtime proof.
- Slot 60 (`call [rax+0x1e0]` at `0x140137b8a`) runs only when `[r12+0xc] == 2`, and its array feeds the
  `(1 - atk+0x34) * def+0x68` factor in `calculateDamage_defense`. Guard absorption is the likely
  meaning (INFERRED), not read.

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
SoulsPlanner's `physicalATKBonus`, `magicATKBonus`, `lightningATKBonus`, `darkATKBonus`, `mundaneATKBonus`
and `fireATKBonus[INT+FTH]` equal these regulation columns with these indices at every entry. Its
`physicalDEFBonus[sum]` does not match: it is off by one at 180 of 393 sums (row 3 is 63 in the regulation
and 62 in the planner).

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
`thunk 0x140358de0(CharacterManager, 0x14) < 25`. Which param that row belongs to is not established.

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

The infusion indices follow from the add-rate offsets and the target elements: 0 none, 1 Fire, 2 Magic, 3 Lightning,
4 Dark, 5 Poison, 6 Bleed, 7 Raw, 8 Enchanted, 9 Mundane. For the Dagger (+10, reinforce 1000,
statsAffectId 1005030), this model gives base 115 / 80.5+80.5 / 132.25 / 57.5 for
Standard / elemental / Raw / Mundane. Coefficients come out as 0.15/0.45, then 0.0565/0.169/0.3, then 0.038/0.113.
SoulsPlanner has 115 / 80+80 / 132 / 57 and 0.15/0.45, 0.06/0.17/0.3, 0.04/0.11, so it truncates base to an integer and
rounds coefficients to 2 dp. SoulsPlanner's Enchanted STR 0.06 is not the regulation's 0.053.

Not read: `D` (`0x14038fee0`), the extra rate edits (`0x14038f440`, and `0x1403910f0` when `sil`), and
`k` = `0x14038f6a0(...) * [ws+0x50 row +0x54|+0x60] * [[rbx]+0x38]+0xd8`. So this loop is proven for the
scaling structure, not for everything a hit multiplies in. That this path is the one the status menu
shows is also not established: `0x1403903b0` has exactly one static caller.

**Delta vs `scripts/ds2-builds-recommend.py`** (`attack_rating`, `hit_damage`, `damage`, `build_defense`):
- AR: the same `(base + sum bonus*coef) * rate` structure, but SoulsPlanner folds `rate` into its
  `atk`/`atkScale` with rounding. The error is a fraction of an AR point per stat, larger for Enchanted STR.
  Mundane uses SoulsPlanner's `modifier`, and the EXE coefficient source above is unresolved.
- Elemental cut `min(0.99, (D+100)/1000)` matches. Not modelled: the lack-of-stats factor on armor, and the
  cap gate.
- Physical stat defense: the planner's table is off by one at 180 of 393 sums. The game uses
  `row[trunc(sum/4)].defense`.
