# Infusion ranking: the recommender against the community

The question: when `scripts/ds2-builds-recommend.py` says a split infusion (Lightning, Dark, Magic,
Fire) beats an uninfused or Raw weapon against a player, and the community says the opposite, which
one is right? This is the evidence log for that question. Every number below is re-printed by

```bash
python3 scripts/ds2-infusion-evidence.py            # every section, about half a minute
python3 scripts/ds2-infusion-evidence.py --only margin,falsify
```

except where an item says it came from elsewhere. All of it is static or offline: the EXE read
through Ghidra, the decrypted regulation, the SoulsPlanner and MugenMonkey corpus. Nothing here was
measured in a running game.

Each item carries a strength tag:

| tag | meaning |
|---|---|
| `EXE` | read at an address in `DarkSoulsII.exe` |
| `REGULATION` | read from the shipped params |
| `SITE` | a planner site's tables |
| `CORPUS` | counted over the planner builds |
| `COMMUNITY` | a wiki, forum or guide, not checked against the game |
| `INFERRED` | reasoning from the above, not read anywhere |

## Short answer

- Against a player defender the model's arithmetic is the game's (`EXE`). Raw or uninfused only wins
  when the defender's elemental defense is far above what players at that level wear.
- The disagreement with real builds is real, not noise: when a player picked an uninfused or Raw
  weapon and the model picks an element, the model's pick is ahead by a median 12 to 19 percent.
  Between split infusions the misses are near ties.
- None of the explanations the corpus can test rescues the uninfused choice: not stat scaling, not
  weapon buffs, not a PvE intent recorded in the build. The explanations left are the ones the
  corpus cannot see: copying wiki advice, and the cost of elemental stones in a playthrough.
- The suspected naked-defender bug is not a bug. The game scales stat defense per armour piece, and a
  bare body carries half of it (`EXE`, `REGULATION`).
- The open check with the most power to change the conclusion is the NPC defender: its elemental
  defense may be subtracted like physical, so in PvE the community's "split is resisted twice" may
  be right. That calculator has not been read.

## The model (prior evidence)

**Claim.** Damage per hit against a player is `max(AR*10 - DEF, lower) / 12` for physical and
`AR * (1 - (D + 100) / 1000)` for each element. Physical defense cancels between Raw and a 50/50
split, so Raw wins only when the defender's elemental D is above about 366.

**Evidence.** `ChrDamageActionCtrl` slot 32, the cut array from `PlayerGameParamCalculator` slot 59,
and PvP packet 28 carrying the same factors; all in `docs/DS2-DPS-MECHANICS.md`. A player's
elemental DEF entries are always 0: the decompile of slot 58 (`0x140380070`) zeroes the whole
13-float array and writes only entry 0.

**Strength.** `EXE`. **Verdict.** Supports. **Would change it:** a factor applied to one damage type
and not the other that the doc has not read, such as the hand's `live` add.

## The community position (prior evidence)

**Claim.** Raw beats split on the Black Dragon Greataxe because its AR is higher and split damage
"is resisted more".

**Evidence.** Fextralife, wiki.gg, Steam and Fandom say Raw from AR alone, with no defense maths.
The wiki.gg Defense page's printed elemental equation leaves out the `1 -`, though its prose is
right. SoulsPlanner has Raw in most Black Dragon Greataxe weapon slots. A GameFAQs PvP thread
("Infusion vs Physical pvp sl150") says elemental defense acts as a percentage and that weapons
under about 400 AR do more damage infused even with no INT or FTH; its text was seen only through a
search summary, since the site refuses the fetch.

**Strength.** `COMMUNITY`. **Verdict.** The wikis reject the model without an argument that
survives the formula; the GameFAQs thread supports it. **Would change it:** a measured in-game hit.

On the greataxe itself, with a real Raw build's stats at SL 150 one-handed, the defender branch
(`--defender`) puts Raw at 80 to 87 percent of the best infusion against the average defender, the
Black Dragon set, Havel's and a bare body. The 400 AR rule of thumb and the model part ways only
because Raw's 471 is above it.

## A. How far apart are the model and the players

**Claim.** If most misses are within a few percent the disagreement is weak.

**Evidence** (`--only margin`). Calibrate's population, the model's best infusion against the chosen
one at that build's own stats and SL bracket:

| chose | model | median lead | within 5% | over 25% |
|---|---|---|---|---|
| No_Infusion | Lightning | 11.9% | 18% | 8% |
| No_Infusion | Dark | 11.5% | 20% | 11% |
| Fire | Dark | 1.7% | 97% | 0% |
| Dark | Lightning | 0.4% | 95% | 3% |
| Lightning | Dark | 2.3% | 88% | 0% |
| No_Infusion | Magic | 19.2% | 28% | 35% |
| Fire | Lightning | 1.3% | 97% | 0% |
| Raw | Lightning | 16.2% | 6% | 13% |

Over every miss the median lead is 10 percent. Counting a lead under 5 percent as agreement lifts
the match from 42.3 to 62.8 percent; under 10 percent, to 71.0.

`--only breakeven` asks how much elemental defense the defender would need before the player's
choice wins. For No_Infusion against Lightning it is a median lightning D of 340, against Dark a
dark D of 334, and Raw against Lightning 370. Players at SL 126-155 wear lightning D of 167 at the
median, 291 at p90 and 374 at p99.

**Strength.** `CORPUS` over `EXE` arithmetic. **Verdict.** Splits the misses in two. Between split
infusions they are near ties, so those are weak evidence either way. Uninfused against an element
is a large, consistent gap that only a top-percentile defender closes. **Would change it:** a
defender population with far more elemental defense than the planner builds record.

## B. Why players pick uninfused, Raw and Fire

### Scaling

**Claim.** No_Infusion choosers invest in the STR and DEX the uninfused weapon scales with, and the
model misses that credit.

**Evidence** (`--only scaling`). Medians of effective stats:

| group | STR | DEX | INT | FTH |
|---|---|---|---|---|
| No_Infusion, model says Lightning | 40 | 26 | 3 | 8 |
| No_Infusion, model says Dark | 35 | 30 | 7 | 8 |
| No_Infusion, model agrees | 40 | 40 | 3 | 7 |
| chose Lightning | 20 | 20 | 4 | 40 |

They do invest in STR and DEX. But `attack_rating` reads those stats, so the credit is already in
the score: the Lightning row keeps a median 62 percent of the uninfused physical AR and adds 191
lightning AR with FTH at base (76 percent of these builds have FTH 15 or less). The element's base
damage, not its scaling, is what wins.

**Strength.** `CORPUS`, `SITE`/`REGULATION` AR. **Verdict.** Rejects scaling as the explanation.
**Would change it:** an AR row the regulation and the game disagree on.

### Weapon buffs and resins

**Claim.** Players keep a weapon uninfused to buff it.

**Evidence** (`--only buffs`). Calibrate does not model buffs. Builds that can cast one (a buff
spell attuned and a catalyst carried) are 10.5 percent of No_Infusion choices and 38.1 percent of
elemental ones; resins are carried by 5.8 and 2.0 percent. Adding each castable buff, at
`docs/DS2-DPS-MECHANICS.md` "Spell and buff attack"'s X/100 of the weapon's rate in the buff's
element, moves agreement on those builds from 57.7 to 55.2 percent: more matches turn into misses
than the other way. Some infusion rows have no regulation row for the buff and score it as 0.

**Strength.** `CORPUS`, `EXE` buff formula. **Verdict.** Rejects buffs as the explanation; buffers
are the elemental choosers. **Would change it:** a rule that forbids buffing an infused weapon,
which was not read, or resin values, which were not read either.

### PvE intent

**Claim.** Uninfused choosers build for PvE, where the player formula does not apply.

**Evidence** (`--only intent`). A build counts as PvP from its covenant (Brotherhood of Blood,
Bell Keepers, Rat King, Blue Sentinels, Dragon Remnants, Way of Blue), an invasion orb, or its
name; PvE from Heirs of the Sun, Pilgrims of Dark, Company of Champions, or its name.

| intent | model agrees | uninfused or Raw | elemental |
|---|---|---|---|
| PvP | 38.5% | 52.6% | 36.3% |
| PvE | 48.7% | 41.9% | 54.6% |
| unmarked | 42.6% | 56.6% | 37.9% |

PvP-marked builds pick uninfused more often than PvE-marked ones, the reverse of the hypothesis.

**Strength.** `CORPUS`; the markers are a proxy and most builds carry none. **Verdict.** Rejects it.
**Would change it:** a better intent signal than covenant and name.

### Fire on INT plus FTH builds

**Claim.** Fire is the right choice when both INT and FTH are invested.

**Evidence** (`--only fire`). Fire choosers sit at a median INT 20 and FTH 20 and the model agrees
with 13.9 percent of them, picking Dark for most. But Dark leads Fire by a median 1.9 percent where
both stats are 20 or more, and 0.9 percent otherwise; the SL 126-155 average defender has fire
defense 211 against dark 191, which is the whole gap.

**Strength.** `CORPUS`. **Verdict.** A near tie; it neither supports nor rejects the model.
**Would change it:** a different defender fire average.

### Copying wiki advice, and what infusing costs

**Claim.** Players follow the wiki, or cannot yet afford the stones (Boltstone, Darknight Stone,
Firedrake Stone) in the part of a playthrough the build was planned for.

**Evidence.** The corpus records neither where a player read their advice nor how far they had got.
Neither can be tested from it.

**Strength.** `INFERRED`. **Verdict.** Untested. **Would change it:** builds tagged with their
source, or save files of characters mid-playthrough.

## C. Static checks that could falsify the core claim

### The bare defender

**Claim (suspected bug).** The Naked defender reads physical 49 at the median stats because stat
defense goes missing without armour.

**Evidence.** The decompile of `0x140380070` writes

```text
DEF[0] = SpEffect add + (sum over the pieces of ArmorParam +0x1c) * stat defense (stat block +0x48)
         + sum over the pieces of piece defense * lack-of-stats factor * scale
```

`ArmorParam +0x1c` is `defenseStatAffectScale`, and an empty slot holds the naked pieces
`11001100..11001103`. Their scales are 0.075, 0.225, 0.075 and 0.125, summing to 0.5, which equals
SoulsPlanner's Naked `physicalDEFBonus`. The planner's per-piece bonus equals `defenseStatAffectScale`
for every name-matched piece but a handful. At END 30, VIT 15, STR 25, DEX 25 the stat defense is
98, so a bare body has 49. The script's `--only naked` prints this.

**Strength.** `EXE`, `REGULATION`. **Verdict.** Not a bug: the model matches the game. Physical
defense cancels between Raw and split anyway, so it could not have flipped the pick.

### The 0.99 cap gate

**Claim.** The unresolved gate in `FUN_140164af0` may change the cut.

**Evidence.** The cap only binds at D of 890. The highest elemental defense of any corpus build is
540; even a quartz ring counted once per armour piece reaches it only for a ring-stacking tank.

**Strength.** `EXE`, `CORPUS`. **Verdict.** Cannot flip Raw against split at real defenses.

### The damageLower floors

**Claim.** `max(..., damageLower)` raises low hits, which calibrate's `damage()` leaves at 0.

**Evidence** (`--only falsify`). With a floor of 70 applied, the physical floor binds on some
heavily elemental rows and the elemental floor on a few with a sliver of elemental AR. The model's
pick moves for a handful of weapons and agreement stays at 42.3 percent.

**Strength.** `EXE`, `CORPUS`. **Verdict.** Cannot flip the conclusion.

### Slash, strike and thrust defense

**Claim.** A hit's typed defense, not the general one, is subtracted.

**Evidence.** Scoring with slash, strike or thrust defense as the physical one leaves agreement at
42.3 percent each. The defense cancels between infusions unless a floor binds.

**Strength.** `EXE`, `CORPUS`. **Verdict.** Cannot flip it.

### Elemental defense rings

**Claim.** The average defender leaves out quartz and dispelling rings.

**Evidence** (`--only rings`). 2.3 percent of SL 126-155 builds wear one. Counting their defense
once moves agreement nowhere; once per armour piece, to 42.8 percent.

**Strength.** `CORPUS`, `SITE` ring values. **Verdict.** Too rare to matter. Whether a ring counts
once per piece needs runtime proof.

### The lack-of-stats factor

**Claim.** Slot 58 and slot 59 scale each piece by a lack-of-stats factor the model leaves out.

**Evidence.** Both multiply each piece by `0x140381c20`'s result. A defender short of a piece's
requirement loses armour defense, which lowers the elemental cut and so helps split, not Raw.

**Strength.** `EXE`, effect `INFERRED`. **Verdict.** If it moves anything it moves toward the model.

### NPC defenders

**Claim.** In PvE the elemental DEF is subtracted like physical, so split damage meets two flat
defenses and the community's "resisted more" can be right there.

**Evidence.** Only the player calculator is known to zero elemental DEF. An enemy's calculator, and
so its elemental DEF entries, has not been read.

**Strength.** `INFERRED`. **Verdict.** Open, and the check most able to change the answer for PvE.
It cannot change the answer for PvP, which is what the model scores. **Would settle it:** the NPC
game-param calculator's defense slot.

## Verdict

For player defenders the model is right and the community's Raw-by-default is not supported by any
evidence found: the arithmetic is the game's, the gap is large, and the hypotheses that would
explain players' uninfused choices fail on the corpus. The model is not established for PvE.
