# DARK SOULS II PvP build embeddings -- design decisions

Decisions from the 2026-09-27 grilling session. Nothing here is built yet.

## Goal

Similarity search. Given one build (example: MugenMonkey
[33402](https://mugenmonkey.com/darksouls2/33402)), list armor, weapons, spells and rings that
would also benefit it.

## Corpus

| Source | Where the build lives | Item identity | Infusion | Upgrade level | Soul memory |
| --- | --- | --- | --- | --- | --- |
| MugenMonkey `/darksouls2/<id>` | inline `gon.savedStats` JSON | site-numeric ids; names in `ds2application-*.js` | **absent** | absent | yes |
| SoulsPlanner `/darksouls2/<id>` | inline `savedBuild` JS object | name slugs (`Ring_of_the_Embedded`) | yes (`Mace;No_Infusion`) | absent | absent |

Both verified by fetching the pages on 2026-09-27. MugenMonkey ids run past 60000; its robots.txt
allows crawling.

## Canonical item id

The game's ItemParam row id, which is also the `itemname.fmg` text id (`scripts/ds2-regulation.py
items --fmg`, fmg from `scripts/ds2-ebl.py extract /menu/text/english/itemname.fmg`). Measured name
join after lowercasing and stripping non-alphanumerics: MugenMonkey 882/939, SoulsPlanner
1005/1052. The misses are abbreviations (`Black Knight GS`, `Fume UGS`) and site typos
(`Berseker Blade`); they go in a hand-checked alias table. A build with any still-unmapped item is
dropped.

## Build filter

Kept only if complete and usable. Covenant and consumable items are exempt from completeness.

| Rule | Detail |
| --- | --- |
| Armor | all 4 slots set |
| Weapons | at least one real weapon (not Bare Fists) |
| Rings | all 4 set |
| Spells | at least one if a catalyst, chime or pyromancy flame is equipped |
| SL1 | dropped, and so is any build whose stats all equal its starting class's |
| Weapon/shield | dropped if any STR/DEX/INT/FTH requirement > effective stat; STR halved when two-handed |
| Armor | dropped if any requirement > effective stat (armor keys `0x11..0x14`, `DS2-ITEM-REQUIREMENTS.md`) |
| Spell | dropped if INT/FTH requirement > effective stat, or slots used > attunement slots |
| Rings | no requirements |

Effective stats = base + ring bonuses (both example builds wear Ring of the Embedded). The
two-hand halving (`shr cx,1`, grips 2/3) is the game's mechanics check, see
`crates/ds2-item-warn/src/requirement.rs`. Requirement values come from the regulation params,
not the planner sites.

## Tokens

- **Weapon + infusion** is one token. Some weapons are unusable in PvP under some infusions, so the
  pair carries meaning the parts do not.
- Armor piece, ring, spell: one token each.
- A plain **weapon** token is emitted alongside every weapon+infusion token, so a rare pair falls
  back to what the weapon alone has learned.
- **Stats are not tokens.** Nine always-present stat tokens would swamp a ~15-token build and pull
  every query toward "same stats". (5-point stat bands were chosen first and then withdrawn for
  this reason.) Stats come back in at query time; see Query.

## MugenMonkey infusion inference

MugenMonkey stores no infusion, so it is inferred. The first design -- the infusion with the highest
expected damage against the corpus-average defender -- matches SoulsPlanner builds' real choice only
41.0% of the time (`--calibrate`), below the 51.8% of always guessing No_Infusion.

What ships (`InfusionModel` in `scripts/ds2-builds-recommend.py`) is a conditional logit over the
weapon's own infusions, fitted on SoulsPlanner builds, using only what a MugenMonkey build also
records: effective STR/DEX/INT/FTH (per infusion), the damage model's score per infusion, and the
share of SoulsPlanner builds carrying that weapon that chose each infusion. `--infusion-eval` hides
the infusion of 5965 SoulsPlanner weapons (3373 builds, 5-fold split by build):

| Predictor | Accuracy |
|---|---|
| Majority class (No_Infusion) | 51.8% |
| Per-weapon majority | 62.4% |
| Damage model best | 41.0% |
| Logit, stats + damage only (no per-weapon prior) | 63.3% |
| **Logit (shipped)** | **68.3%** (top two 84.7%) |

Its probability is usable as confidence: 91.9% right when p >= 0.9 (1042 weapons), 43.5% when
p < 0.5 (1744). Most errors are an elemental choice inferred as No_Infusion.

Not proven: that SoulsPlanner's labels transfer to MugenMonkey builders (No_Infusion is also the
planner's default, so some of the 51.8% may be builds whose author never picked one). Inferred
infusions are shown by `--mugen` only; they do not yet enter the item model or the panel's data
file, so MugenMonkey weapons still contribute only the plain weapon token.

## Formula source

Attack-rating scaling and defense reduction are taken from the executable by static RE. The
planner sites' JS is a cross-check, not ground truth. Not done yet; the ghidra MCP did not connect
in the session that wrote this.

## Infusability

REGULATION: ItemParam i32[5] -> WeaponParam `weaponReinforceId` (+0x08) -> WeaponReinforceParam
`attrSpec` (+0xE8) -> CustomAttrSpecParam bit mask, lowest bit first: No_Infusion, Fire, Magic,
Lightning, Dark, Poison, Bleed, Raw, Enchanted, Mundane (stone order of CustomAttrCostParam row
100000); `attrSpec = 0` means not infusable. SoulsPlanner's per-weapon infusion lists equal this
mask for all 323 name-matched weapons, so the recommender uses the planner's lists unchanged.
Per-weapon masks: `~/.cache/ds2-builds/infusable.json`. EXE: no extra filter checked.

## Model

EASE (Steck 2019): a closed-form item-item model fitted on the build x token matrix. It scores
missing items for a partial build directly, instead of averaging item vectors and taking a cosine.
PPMI + SVD was proposed first and replaced after an independent review. Identical builds (forks,
re-saves) are deduped before fitting.

## Query

For a given build, each candidate's score combines:

1. **Item model** -- the EASE score from the build's items.
2. **Stat neighbours** -- a boost from items used by the ~50 corpus builds with the most similar
   effective-stat spread.
3. **Stat fit** -- computed from game data, not learned: candidates the build cannot use are
   removed, and the rest are weighted by how well their scaling matches the stats the build
   invested in.

Results are ranked per slot (weapon, armor piece, ring, spell), leaving out items the build already
has. How the three are weighted is set by the offline evaluation.

## Evaluation

- Split by build, after deduping.
- Hide one item per held-out build; recall@k and MRR per slot, broken down by how popular the
  hidden item is.
- Must beat "most popular item per slot among builds with similar stats", or that baseline ships
  instead.
- MugenMonkey infusion inference is validated first, on SoulsPlanner builds with their infusion
  hidden: 68.3% vs 51.8% majority class (`--infusion-eval`, see above). MugenMonkey weapons still
  contribute only the plain weapon token until the item model's own evaluation shows the inferred
  weapon+infusion tokens help.

## Armor

Not learned, and not in the item model: nothing can say what looks good. Armor is computed. The
candidates are pieces the build can wear within its equip-load limit (weapons, shields and rings
counted) and meets the stat requirements of. They are ranked by expected damage taken, where the
threat mix is the per-type share of damage (physical, magic, fire, lightning, dark) that the
corpus's weapons+infusions and spells actually deal, computed with the same executable-derived
formulas.

Equip-load cap: **below 70%**, to stay out of the heavy-roll tier (the user's rule; the in-game
threshold is still to be confirmed statically).

## Where it lives

Python under `scripts/`, reusing `ds2-regulation.py`, `ds2-ebl.py` and `ds2-fmg.py`. Nothing runs
in the game, so nothing goes in `crates/`.

| Piece | File |
| --- | --- |
| Crawler: MugenMonkey + SoulsPlanner, rate-limited, disk cache (not committed) | `scripts/ds2-builds-scrape.py` |
| Item id joins, alias table, build filter | `scripts/ds2-builds-corpus.py`, `scripts/ds2-builds-aliases.toml` |
| Attack rating and defense formulas, after static RE | `scripts/ds2-damage.py` |
| EASE fit, query, evaluation | `scripts/ds2-builds-recommend.py` |

## Wieldability and minimum builds

Every answer about a weapon or build (a minimum level, a suggested build, a recommendation) must
satisfy all of these, both when asked in conversation and in the program:

| Rule | Detail |
| --- | --- |
| Equip load | the build's weapons fit under **70%** equip load together with the **lightest possible** armor and rings. Ring and armor effects on stats and weight count (e.g. a ring that raises a stat can lower the levels needed; a ring's own weight counts). |
| Equipment stats | armor and rings can add stats and armor can require stats; both are part of wieldability. |
| Endurance | matters **only** if the build's weapons sit at the high end of stamina cost, measured by each weapon's lowest-cost basic attack against all weapons. |
| Agility | required. Adaptability + Attunement give agility up to its hard cap. The target SL comes from the corpus's typical ADP+ATT allotment and the SL needed to reach the cap. |
| Soul memory | ignored. |
| Grip | anything with a STR requirement is shown two-handed only (the game halves STR two-handed). |
| Recommended minimum | the **default answer**: requirement stats at their minimum (cheapest class and gear), and VGR/END/VIT/ATT/ADP each at the **median** of its SL bracket, iterated until the SL stays in the bracket it was floored against. AGL is an output, not a target. END is floored only for high-stamina weapons. |
| Weapons for stats | `--weapons-for STATS`: every usable weapon+infusion (STR halved when only two-handing makes it usable, flagged "2H only") ranked by damage = sum over types of damage(AR_type, mean DEF_type of the SL bracket's corpus builds). Per weapon: the best infusion, plus the 2nd and 3rd only while within 10% of the best; never more than 3. SL = stat total - 53 (holds for every class). |
| Options | for each free stat, the top 1/25/50/75% value in the bracket, with the remaining points spread in median proportions; plus each stat's min-max range at the clamped SL (the slider bounds). `--sl` clamps, `--json` emits it. |

Agility (SITE: identical in SoulsPlanner `getAgility` and MugenMonkey `Agility`; agrees with the
community rule investment = floor(0.75 ADP + 0.25 ATT)): `AGL = 80 + floor((3 ADP + ATT) / 4)` up to
110, then `110 + floor((3 ADP + ATT - 120) / 28)`; 120 only at 99/99; displayed floor 85. So 110 is
the practical cap: past it, one AGL costs about 9 ADP. The i-frame breakpoints per AGL are not
sourced yet.

Brackets (the user's): **SL** 1-20, 21-40, 41-70, 71-100, 101-125, 126-155, 156-200, 201-250,
251-max. **Each stat** 0-10, 11-20, ..., 81-90, 91-99. Stat neighbours are drawn from the query's SL
bracket, nearest by per-stat bracket distance.

Mechanics (equip load table, stat/weight effects of every ring and armor piece, agility formula and
cap, per-weapon stamina) are being researched; values land here once sourced.

## Soul memory

Ignored: not a filter, not a feature.
