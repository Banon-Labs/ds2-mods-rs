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

MugenMonkey stores no infusion, so it is inferred: at **full upgrade** (+10, +5 for twinkling/boss
weapons), the infusion with the highest expected damage against a reference defender wins. The
reference defender's defense for each damage type is the corpus average, including armor, rings and
stat-derived defense. The build's own damage rings (e.g. Dark Clutch) apply.

## Formula source

Attack-rating scaling and defense reduction are taken from the executable by static RE. The
planner sites' JS is a cross-check, not ground truth. Not done yet; the ghidra MCP did not connect
in the session that wrote this.

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
  hidden. Until its accuracy is measured, MugenMonkey weapons contribute only the plain weapon token.

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

## Soul memory

Ignored: not a filter, not a feature.
