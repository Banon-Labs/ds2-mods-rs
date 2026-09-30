# DS2 SotFS mechanics for the build recommender

Machine-readable twin: `mechanics.json` in this directory (full armor/ring/weapon tables, agility
table idx 1..99, per-weapon stamina for 324 weapons).

Labels: **VERIFIED** = read from the game (regulation params / SpEffect emevd decrypted with
`scripts/ds2-regulation.py`, or the executable `darksoulsii-deobf.bin` via `scripts/ds2-disasm.py`).
**SITE** = SoulsPlanner / MugenMonkey tables or JS. **INFERRED** = derived from VERIFIED data plus
field names or consistency, not traced in code. **COMMUNITY** was not needed.

Param columns were decoded with Smithbox DS2S paramdefs (MIT); for every param used, the measured
row stride equals the def size. Names come from `itemname.fmg`. Nothing was launched.

## 1. Equip load

**Max load = PhysicalStatsPerLevelStatValuesParam.equipLoadMax[effective Vitality]** (VERIFIED:
`RelatePhysicalStatToLevelStat.equipLoadMax = 3` = Vitality; the builder at `0x14038d790` looks the
row up by that stat from the effective stat block and stores `+0x8`). Stamina column shown too
(`staminaMax/10`, row = effective Endurance, same mechanism). Both match SoulsPlanner exactly.

Cell = `equip load / max stamina` for stat value = row + column.

| stat | +0 | +1 | +2 | +3 | +4 | +5 | +6 | +7 | +8 | +9 |
|---|---|---|---|---|---|---|---|---|---|---|
| 0 |- | 40/82 | 41.5/84 | 43/86 | 44.5/88 | 46/90 | 47.5/92 | 49/94 | 50.5/96 | 52/98 |
| 10 |53.5/100 | 55/102 | 56.5/104 | 58/106 | 59.5/108 | 61/110 | 62.5/112 | 64/114 | 65.5/116 | 67/118 |
| 20 |68.5/120 | 70/121 | 71.5/122 | 73/123 | 74.5/124 | 76/125 | 77.5/126 | 79/127 | 80.5/128 | 82/129 |
| 30 |83/130 | 84/131 | 85/132 | 86/133 | 87/134 | 88/135 | 89/136 | 90/137 | 91/138 | 92/139 |
| 40 |93/140 | 94/141 | 95/142 | 96/143 | 97/144 | 98/145 | 99/146 | 100/147 | 101/148 | 102/149 |
| 50 |102.5/150 | 103/151 | 103.5/152 | 104/153 | 104.5/154 | 105/155 | 105.5/156 | 106/157 | 106.5/158 | 107/159 |
| 60 |107.5/160 | 108/161 | 108.5/162 | 109/163 | 109.5/164 | 110/165 | 110.5/166 | 111/167 | 111.5/168 | 112/169 |
| 70 |112.5/170 | 112.5/171 | 113/172 | 113/173 | 113.5/174 | 113.5/175 | 114/176 | 114/177 | 114.5/178 | 114.5/179 |
| 80 |115/180 | 115/181 | 115.5/182 | 115.5/183 | 116/184 | 116/185 | 116.5/186 | 116.5/187 | 117/188 | 117/189 |
| 90 |117.5/190 | 117.5/191 | 118/192 | 118/193 | 118.5/194 | 118.5/195 | 119/196 | 119/197 | 119.5/198 | 120/200 |

**Load tiers** (VERIFIED, ChrEquipLoadParam row 1 "load ratio", lower bound of each category):
light 0%, middle 30%, heavy 70%, too heavy 100%, overweight 120%. So 70% is where the heavy
category starts. Whether exactly 70.0% counts as middle or heavy, and whether the per-category
roll values (rows 24/25: roll anim blend 0/30/70/100, roll distance loss 0/10/28/40/50%) are
interpolated between tiers, is not established. Use `burden < 0.70 * maxLoad`.

**Max-load multipliers** (VERIFIED, SpEffect op `1000[1]` kind 9, "while equipped"):

| item | weight | factor |
|---|---|---|
| Royal Soldier's Ring / +1 / +2 | 0.2 / 0.5 / 0.8 | x1.10 / x1.15 / x1.20 |
| First / Second / Third Dragon Ring | 0.5 / 0.8 / 1.0 | x1.05 / x1.10 / x1.125 (also HP x1.03/1.05/1.075, stamina x1.05/1.10/1.125) |
| Helm / Armor / Gauntlets / Leggings of Aurous | 1.2 / 3.2 / 1.2 / 2.0 | x1.01 / x1.015 / x1.01 / x1.015 |
| Penal Mask | 5.1 | x1.03 |

How two multipliers combine (product vs sum) is not established; SoulsPlanner applies them in
sequence (product). **No item changes item weight**: no ring/armor/weapon effect decodes to a weight
change, and DS2 has no Ring of Favor. Max load is computed from *effective* Vitality, so
VIT-adding gear (Vengarl's Helm +2, Velstadt's Helm +1, Handmaid's Ladle +1, Ring of the Embedded)
raises it.

**Lightest equipment** (VERIFIED weights; ArmorParam.weight and RingParam.weight are the real ones,
ItemParam.weight is 0 for all of them):
- Naked slot = 0.0: the bare-body ArmorParam rows 11001100..03 (items 21001100..03, armorType 0)
  weigh 0.0. Empty ring slot = 0. So the lightest possible armor+rings is **0.0**.
- Lightest named pieces: head 0.5 (Black Witch Veil, Lion Warrior Helm, Grave Warden Mask; Black
  Witch Domino Mask 0.5 needs INT 20), chest 1.4 Moon Butterfly Wings, hands 0.8 Moon Butterfly
  Cuffs, legs 1.0 Prisoner's Waistcloth.
- Ring weights: all 122 rings in `mechanics.json` `equip_load.ring_weights`; lightest 0.2. SoulsPlanner
  agrees except Abyss Seal (site 1.0, game 0.2). Weapon weights (WeaponParam.weight) agree with
  SoulsPlanner for all 321 name-matched weapons. Site armor weights disagree with the game for the
  Aurous and Astrologist sets (site has heavier values); use the game values.

Practical rule: with naked + no rings, weapons must fit under `0.70 * maxLoad(VIT_eff)`; a Royal
Soldier's Ring+2 (0.8) raises the cap to `0.70 * 1.2 * maxLoad - 0.8`, which beats naked once
`maxLoad > 5.7`, i.e. always (min max load is 40).

## 2. Stat-changing and stat-gated equipment

Encoding (VERIFIED from `SpEffectRing/Armor/Weapon.emevd`, event id = item id; kinds matched
against names and SoulsPlanner `alter` values):
- `1000[0] [kind, duration, value]`: additive. kind 6 VGR, 7 END, 8 VIT, 10 STR, 11 DEX, 12 INT,
  13 FTH, 14 ADP, 0 poise, 20 attunement slots. (9 = ATT by position, no item uses it.)
- `1000[1] [kind, duration, factor]`: 7 max HP, 8 max stamina, 9 max equip load.
- `1000[19] [packed(stat, low, high, bonusLow), bonusHigh]`: stat-dependent add, stat numbered
  1 VGR .. 9 ADP. Constant penalties use low=0, high=99.

Scaled-bonus rule, INFERRED (byte values VERIFIED; the interpolation reproduces SoulsPlanner's
Embedded table exactly): `s <= low -> bonusLow; s >= high -> bonusHigh; else
floor(bonusLow + (bonusHigh - bonusLow) * (s - low) / (high - low))`, result capped so the stat
does not pass 99 (SITE). Not established: whether `s` is the base or already-modified stat.

Rings (stat, HP, stamina, load or slot effects only; weight in kg):

| ring | wt | effect |
|---|---|---|
| Strength Ring | 1.5 | STR +5 |
| Dexterity Ring | 1.5 | DEX +5 |
| Ring of Knowledge | 0.5 | INT +5 |
| Ring of Prayer | 0.5 | FTH +5 |
| Simpleton's Ring | 1.5 | ADP +5 |
| Ring of the Embedded | 2.0 | VGR, END, VIT each: +7 at <=6, falls linearly to +3 at >=60 (<=19:+6, <=33:+5, <=46:+4, else +3; 97:+2, 98:+1 by the 99 cap); plus a damage-taken factor of 1.085 (kind 26, INFERRED meaning) |
| Southern Ritual Band / +1 / +2 | 1.0 / 1.2 / 1.5 | attunement slots +1 / +2 / +3 |
| Royal Soldier's, Dragon rings | see section 1 | |
| Life Ring..+3, Blue Seal, Northern Ritual Band..+2 | 0.2-1.2 | HP x1.05/1.075/1.125/1.15, x1.03, x0.8/0.75/0.5 |
| Ring of Giants / +1 / +2 | 2 / 3 / 5 | poise +10 / +20 / +30 |

No ring has a stat requirement: RingParam has only weight, durability, repair cost and drop weight
(VERIFIED def + rows).

Armor with requirements or stat effects (VERIFIED ArmorParam.prerequisite* and SpEffectArmor):

| piece | slot | wt | requires | effect |
|---|---|---|---|---|
| Havel's Helm/Armor/Gauntlets/Leggings | all | 7.5/19.5/11.5/11.5 | STR 20 | - |
| Jester's Cap/Robes/Gloves/Tights | all | 3/5.7/2.3/2.7 | DEX 14 | - |
| Durgo's Hat | head | 2.6 | DEX 16 | - |
| Engraved Gauntlets | hands | 5.5 | STR 12, DEX 12 | - |
| Shadow Gauntlets | hands | 1.6 | DEX 16 | - |
| Gyrm Warrior Helm/Greathelm/Armor/Gloves/Boots | all | 8.9/9.6/19.7/5.4/8.8 | STR 14 | - |
| Ironclad and Old Ironclad sets | all | 6.3/29.8/9.4/9.4 | STR 14 | - |
| Rusted Mastodon set | all | 6/15.9/7.8/9.8 | STR 14 | - |
| Mastodon set | all | 6.6/17.7/8.8/10.8 | STR 16 | - |
| Smelter Demon set | all | 9/17.6/10.4/10.4 | STR 16 | - |
| Looking Glass Mask / Armor / Gauntlets / Leggings | all | 7/18.3/10.4/10.4 | STR 16 | Mask: FTH +1 |
| King's Crown / Armor / Gauntlets / Leggings | all | 3.8/12/4.2/7.1 | STR 14, DEX 12 | Crown: INT +3, FTH +3 |
| Black Witch Hat | head | 1.5 | INT 20 | attunement slots +1 |
| Black Witch Domino Mask | head | 0.5 | INT 20 | HP x0.9 |
| Warlock Mask | head | 4.8 | INT 14 | INT +2 |
| Black Hood / Robes / Gloves / Boots | all | 0.9/2.6/1/2 | INT 16 | Hood: INT +2, FTH +2 |
| Saint's Hood / Dress / Long Gloves / Trousers | all | 0.9/2.6/1/2 | FTH 16 | Hood: FTH +1 |
| Chaos Hood / Robe / Gloves / Boots | all | 0.9/2.6/1/2 | INT 16, FTH 16 | Hood: INT +3 |
| Sanctum Priestess Tiara | head | 2 | INT 11, FTH 11 | - |
| Cale's Helm | head | 3 | - | DEX +2, END +1 |
| Mad Warrior Mask | head | 5 | - | ADP +2 |
| Peasant Attire / Long Gloves / Trousers | chest/hands/legs | 3.8/1.8/2.6 | - | each ADP +2, INT -1 |
| Peasant Hat | head | 1.3 | - | FTH +2, INT -1 |
| Crown of the Sunken King | head | 4 | - | END, STR, DEX, INT, FTH +1; VGR, VIT -1 |
| Flower Skirt | legs | 1.7 | - | VGR +2, END +1 |
| Vengarl's Helm | head | 6.8 | - | VIT +2 |
| Velstadt's Helm | head | 7.5 | - | END +1, VIT +1 |
| Moon Hat | head | 2.2 | - | INT +2 |
| Desert Sorceress Hood | head | 0.6 | - | INT +3 |
| Moon Butterfly Hat, Archdrake Helm, White/Black Hollow Mage Hood | head | 0.8/5.4/1.8/1.9 | - | INT +1 |
| Dark Mask, Dragon Sage Hood, Leydia Black Hood, Hexer's Hood | head | 3.8/2/1/0.9 | - | INT +1, FTH +1 |
| Insolent Helm | head | 4.8 | - | FTH +2 |
| White Priest Headpiece, Priestess Headpiece, Agdayne's Cuffs | head/hands | 1.4/1.2/2.9 | - | FTH +1 |
| Aurous set, Penal Mask | | | - | max load x1.01-1.03 (section 1) |

SoulsPlanner misses two requirements the game has: Jester's Gloves (DEX 14) and Old Ironclad Armor
(STR 14), and lists Black Witch Hat at INT 16 where the game says INT 20.

Weapons that change stats while equipped (VERIFIED SpEffectWeapon): Vessel Shield VGR/END/ADP/INT/FTH
+1, STR/DEX +4; Handmaid's Ladle VIT +1, END +1, ADP +2, DEX -1; Work Hook DEX +5, ADP -3; Chime of
Screams FTH +8 at <=6 falling to +2 at >=60 (scaled rule). Consumables: Old Growth / Vine /
Blackweed / Goldenfruit Balm +5 STR / DEX / INT / FTH for 45 s.

Requirements are checked against the effective stat block, which includes these bonuses (repo:
docs/DS2-ITEM-REQUIREMENTS.md).

## 3. Agility

VERIFIED in the executable, inside the physical-stats builder `0x14038d790`
(`0x14038de33`..`0x14038e0eb`, `rbx` = effective stat block `chrStatus+0x16`, `+0x10` = ADP,
`+0x06` = ATT):

```
idx = trunc((3*ADP_eff + ATT_eff) / 4)      ; if idx < 1 or idx > 99 then idx = 1
```

`idx` selects the PhysicalStatsPerLevelStatValuesParam row whose step/roll/jump invincibility %
(+0x68/+0x6c/+0x70) and equip-change/item-use speed (+0x74/+0x78) the builder stores. The same code
pattern with other offsets is how bleed/poison use (3*DEX+INT)/4 etc., so this is the game's shape,
not a one-off.

Displayed AGL = `MenuStatsParam.actionSpeed[idx]` (VERIFIED table; the menu indexing it by `idx` is
INFERRED). Closed form of that table, identical to SoulsPlanner `getAgility`:

```
AGL = 85                          if idx <= 5
    = 80 + idx                    if idx <= 30
    = 110 + floor((idx - 30) / 7) if idx < 99
    = 120                         if idx == 99
```

Hard cap: AGL 120 at idx 99, which needs `3*ADP + ATT >= 396`, i.e. ADP 99 and ATT 99 after
equipment. AGL 110 (idx 30) needs `3*ADP + ATT >= 120`; each AGL point above that costs 28 more
points of `3*ADP + ATT`. Minimum for a target idx: `3*ADP + ATT >= 4*idx`. Agility uses effective
stats, so Simpleton's Ring (+5 ADP = +15 in the sum), Peasant pieces, Mad Warrior Mask,
Handmaid's Ladle and Vessel Shield raise it, and Work Hook lowers it. No other ring or spell
touches the index.

Breakpoints (VERIFIED param values; `evasionInvincibleTime` is MenuStatsParam's column, its unit is
not established; roll % is PhysStats `rollingInvincibleTimeRate`; item-use speed is PhysStats
`useItemSpeedScale`, which is what estus uses by INFERENCE):

| first idx | AGL | evasionInvincibleTime | roll i-frame % | item-use speed |
|---|---|---|---|---|
| 1 | 85 | 6 | 25.0 | 0.975 |
| 5 | 85 | 9 | 36.2 | 0.97 |
| 9 | 89 | 12 | 45.2 | 0.97 |
| 12 | 92 | 13 | 51.2 | 1.01 |
| 16 | 96 | 15 | 58.7 | 1.10 |
| 18 | 98 | 16 | 62.5 | 1.15 |
| 20 | 100 | 17 | 66.2 | **1.20 (max)** |
| 23 | 103 | 18 | 69.5 | 1.20 |
| 27 | 107 | 19 | 74.1 | 1.20 |
| 30 | 110 | 20 | 77.5 | 1.20 |
| 43 | 111 | 21 | 82.3 | 1.20 |
| 51 | 113 | 22 | 85.3 | 1.20 |
| 64 | 114 | 23 | 90.2 | 1.20 |
| 72 | 116 | 24 | 93.2 | 1.20 |
| 90 | 118 | 25 | 98.1 | 1.20 |
| 99 | 120 | 25 | 100 | 1.20 |

Item-use speed stops improving at AGL 100. Roll invincibility keeps rising to 99 but gains little
per point after idx 30 (AGL 110). How the roll % turns into frames depends on the roll animation
and was not traced.

## 4. Stamina

Max stamina = `PhysStats.staminaMax[effective END] / 10` (table in section 1; VERIFIED, matches
SoulsPlanner). Dragon rings multiply it (x1.05/1.10/1.125).

Per-attack cost, in the same units:

```
cost = WeaponParam.meleeAttackBaseCost * WeaponStaminaCostParam[costCategoryId].<attack> / 10
```

Values VERIFIED; the multiplication and the /10 scale are INFERRED from the Smithbox field names
and the x10 stamina scale, not traced in code. 454 of 473 WeaponParam rows use cost category 10
("basic set"): 1H R1 x0.95, 1H R1 second hit x1.05, 1H R2 x1.4, 2H R1 x1.187, 2H R2 x1.75. Special
categories: 12 two-handed set, 15 two-hit, 20 bows, 30 special strong attack, 40 Ricard's Rapier,
50/60/70/80/90/100/110/120 single-weapon sets (Sanctum crossbows, Curved Nil GS, Ivory SS, Ivory
King UGS, Sorcerer's Twinblade, great lances).

Metric: `r1_1h` (one-handed R1, first hit) is the cheapest basic attack for 230 of 232 melee
weapons; the exceptions are Ricard's Rapier (R2 5.7) and Bone Fist (2H R1 second hit 8.7).
Distribution of `r1_1h` over 232 melee weapons: min 9.5 (Broken Straight Sword), p25 19.7,
median 24.6, p75 30.3, p90 32.9, max 40.95 (Smelter Hammer). Shields 11.2..30.1, bows 15.7..27.6
(bows actually fire through `rangedAttackBaseCost`, not included). Per-weapon values:
`mechanics.json` `stamina.weapons[*].stamina` (`r1_1h, r1b_1h, r2_1h, r1_2h, r1b_2h, r2_2h`), plus
weight and requirements.

## 5. Two-handing

Two-handing halves the STR requirement (`shr 1`); power stance needs `1.5 x max(L,R)` STR and DEX.
Repo-verified (crates/ds2-item-warn/src/requirement.rs, docs/DS2-ITEM-REQUIREMENTS.md), not
re-derived.

## 6. Stat scaling and elemental defense (EXE)

These were read from the executable. Full traces with addresses are in
[`DS2-DPS-MECHANICS.md`](DS2-DPS-MECHANICS.md), under "Elemental cut" and "Attack rating".

- All stat curves live in the regulation: `PhysicalStatsPerLevelStatValuesParam`, one row per stat
  value. The builder `0x14038d790` picks a row by a stat for each column:
  - STR, DEX, INT and FTH index their own columns.
  - Fire uses `trunc((INT+FTH)/2)`.
  - Dark uses `min(INT, FTH)`.
  - Mundane uses the minimum of all nine stats.
  - Physical defense uses `trunc((END+VIT+STR+DEX)/4)`.
  - Elemental resistance uses the same indices as the matching attack column, stored times 0.01.
- Weapon AR per damage type: `(lerp(min, max, level/maxLevel) * baseValueScale + sum stat_bonus * coef) * rate`.
  - `min`, `max`, `maxLevel` and `rate` come from `WeaponReinforceParam`.
  - `coef` is `WeaponStatsAffectParam[statsAffectId + infusion offset]` at the reinforce level.
  - The infusion moves its `add*Rate` into the target element and takes it out of the others.
- Elemental cut = `(stat resistance % + sum armor %) / 100`. In the planner's displayed defense D that is
  `(D + 100) / 1000`, and the +100 is the stat table's 10% floor.
- Stamina (section 4) and the agility index (section 3) come from the same builder.

## 7. Max HP (EXE)

Read 2026-09-30. The stats builder `0x14038d790` first calls `0x14038e1e0` on the effective stat
block and stores the u16 it returns at its output's `+0x0`:

```
maxHP = hpMax[VGR] + additionalHp[END] + additionalHp[VIT] + additionalHp[ATT] + additionalHp[STR]
      + additionalHp[DEX] + additionalHp[INT] + additionalHp[FTH] + additionalHp[ADP]
```

- Both columns are `PhysicalStatsPerLevelStatValuesParam`'s: `hpMax` (s16 at `+0x4`) and
  `additionalHp` (u8 at `+0x8a`).
- `0x14038e1e0` loops over the block's 11 words. It reads `hpMax` for word 0 and `additionalHp`
  for words 1-8, and skips words 9 and 10. A word outside 1-99 reads row 1.
- The words are VGR, END, VIT, ATT, STR, DEX, INT, FTH, ADP by the id jump table `0x14038e280`.
- `hpMax` equals SoulsPlanner's `getHP` at every VGR from 1 to 99.
- `additionalHp` is 2 per point to 20, then 1 per point to 50, then 70. The site leaves it out, so
  a build with the other eight stats at 20 has 320 more HP than the site shows.
- Two siblings compute the same sum with every word set to one value: `0x14038d290` at 99 (2505,
  the unit status build-up is scaled by) and `0x14038d370` at 1.

## Not established

- Whether burden == 70.0% exactly is heavy, and whether roll behavior between tiers is interpolated.
- How two max-load multipliers combine (product or sum).
- The exact interpolation rule and the stat it reads (base or modified) for the scaled bonus
  (Ring of the Embedded, Chime of Screams). The rule above matches SoulsPlanner but is not traced.
- Units of `evasionInvincibleTime`, and roll i-frames in frames.
- That the displayed AGL is read from MenuStatsParam by the same index (it matches exactly).
- Whether a ring's max-HP factor multiplies the whole of section 7's sum. The recommender assumes
  it does.
- That stamina cost is literally base x multiplier / 10 (field names plus scale, not code).
- SpEffect kinds not needed here (e.g. kinds 2-5, 18, 21-29) were left undecoded.
