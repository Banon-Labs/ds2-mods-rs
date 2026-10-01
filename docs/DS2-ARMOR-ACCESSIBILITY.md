# How likely an opponent at a given soul level is to wear each armour piece

The recommender's damage goal is scored against an opponent's defense. Today that opponent is
either the mean defense of posted builds in the soul-level bracket (`bracket_defense` in
`scripts/ds2-builds-recommend.py`) or a chosen set worn at the bracket's median stats. A third
candidate, the unweighted mean over every piece in the planner's armour table, treats the Ivory
King set as as likely at SL 30 as the Hard Leather set. This page gives every piece an
accessibility: where and when it is obtained, what stats and load it needs, and a weight
`w(piece, SL)` built from those. It is data and analysis only; the recommender's scoring is
unchanged.

- Table: [`data/ds2-armor-acquisition.tsv`](../data/ds2-armor-acquisition.tsv), one row per
  planner piece (433), keyed by the SoulsPlanner key and the game's item id.
- Script: `scripts/ds2-armor-accessibility.py` (`sources`, `table`, `defender`, `--selftest`).
  `table` prints the TSV; the file is regenerated with
  `python3 scripts/ds2-armor-accessibility.py table > data/ds2-armor-acquisition.tsv`.

The table sits in a top-level `data/` rather than `crates/ds2-build-recommender-core/data/`
because nothing compiles it in yet, and a change under `crates/` needs a game run before it can
be pushed. Moving it there is part of wiring it into the recommender.

Labels below: **GAME** = decoded from the installed game's files. **COMMUNITY** = a wiki, a guide,
or Smithbox's community-written row names. **ASSUMED** = a number with no source, said so.
Nothing was launched.

## 1. Where each piece comes from

Every one of the 433 planner pieces joins to a game item id (normalized name; the planner's
singular `Wanderer Manchette` and `Mad Warrior Gauntlet` match by a plural `s`; the four
"Aurous (Transparent)" pieces land on the 2136xxxx ids, the Maughlin gift set, by ArmorParam
weight, 1.2 against 3.8 for the helm). Every piece has at least one source. The source the table
names, by kind:

| kind | pieces | what it is |
| --- | --- | --- |
| pickup | 85 | a map pickup or chest (ItemLotParam2_Other) |
| shop | 55 | a merchant line (ShopLineupParam) |
| boss-soul | 41 | a merchant line that opens on a boss's death (Maughlin, Magerold, Titchy Gren, Shalquoir) |
| drop | 163 | an enemy drop, no fixed source exists |
| npc | 69 | an NPC's set: their gift, their death drop, or Melentia's line after they die |
| covenant | 12 | a covenant rank reward |
| ng+ | 8 | only sold in a second playthrough (Moon Butterfly, Dragonrider) |

### What is read from the game (GAME)

- **Shops.** ShopLineupParam: the item, the price (ItemParam `price` times the line's
  `priceRate`), and the event flags that open and close the line. Elite Knight Armor is 4000 souls
  from Maughlin behind flag 102611, the 1000-souls-spent line.
- **Pickups.** ItemLotParam2_Other. An 8-digit lot id `AABBxxxx` is in map `mAA_BB`
  (10105120 is in m10_10, the Forest of Fallen Giants).
- **Drops, by the game's own placement.** For each of the 28 maps with enemies, the map's
  `generatorparam` and `generatorregistparam` (GameDataEbl, AES-range entries) give
  generator -> `EnemyId`, and EnemyParam gives that enemy's `itemLotId`. A drop lot's maps are the
  maps whose generators spawn an enemy carrying it: 126 pieces' named source is placed this way.
- **Per-map drop lots.** 7-digit ItemLotParam2_Chr ids (`1153000`, the Aldia's Keep mimic) are no
  EnemyParam's lot and so no generator's. Their digits are a map, `ABBxxxx` -> `mA0_BB`
  (DLC: `50BBxxx` -> `m50_BB`); all 17 of them whose Smithbox label names an area agree with that
  map. 47 pieces' named source is placed this way.
- **Requirements and weight** from ArmorParam (`prerequisiteStrength` .. `prerequisiteFaith`,
  `weight`).

A lot slot counts only with `itemNum > 0` and `itemWeight > 0`. That rule is what removes the
Black Dragon set: the Dragon Aerie Black Dragon Knights' lots list it at weight 0. 321 armour
slots in drop lots are placed by no generator and carry no map in their id (unused NPC phantoms,
event-spawned invaders); they are skipped and counted, not guessed at.

### What is community data (COMMUNITY)

- **Smithbox DS2S row names** (`Param Row Names/English`, MIT, community-written) label lots and
  shop lines: the area inside a map that holds several ("[The Gutter]" in m10_25, which is also
  Black Gulch), and the condition a shop line opens on ("[Maughlin the Armourer - Looking Glass
  Knight]"). Smithbox's `MapNames.json` names the maps.
- **Merchant locations**, from Fextralife's NPC pages:
  [Maughlin](https://darksouls2.wiki.fextralife.com/Maughlin+the+Armourer) (Majula),
  [Melentia](https://darksouls2.wiki.fextralife.com/Merchant+Hag+Melentia) (first in the Forest of
  Fallen Giants; adds an NPC's set once that NPC is dead),
  [Gilligan](https://darksouls2.wiki.fextralife.com/Laddersmith+Gilligan) (first in Earthen Peak),
  [Magerold](https://darksouls2.wiki.fextralife.com/Magerold+of+Lanafir) (Iron Keep, which the
  game's placement of his own drop lot agrees with),
  [Navlaan](https://darksouls2.wiki.fextralife.com/Royal+Sorcerer+Navlaan) (Aldia's Keep),
  [Cromwell](https://darksouls2.wiki.fextralife.com/Cromwell+the+Pardoner) (Brightstone Cove).
- **Where an NPC is.** The maps the NPC's own drop lot is placed in (GAME), first non-Majula map
  for a kill or Melentia's line, last map for a gift Smithbox labels as a quest end ("Gift",
  "Survives", "All flames").
- **The Black Dragon set**, the one set with no lot: Dragon Remnants rank 1 (helm) and rank 2
  ([Fextralife](https://darksouls2.wiki.fextralife.com/Black+Dragon+Set)), a covenant Magerold
  leads in Iron Keep.

### Where the game and the wiki disagree

- **Gilligan's prices.** The game has two lines per item: Earthen Peak at `priceRate` 0.8 (Black
  Leather Armor 960) closed by flag 201850, and Majula at 1.0 (1200) opened by it. Fextralife
  quotes 4,300 for the set "in Earthen Peak and later in Majula", which is the Majula price.
- Checked and agreeing: Alva set (Fextralife 26,300 after 15,000 spent; the game's four lines sum
  to 26,300), Havel's set (The Gutter), Royal Swordsman set (Royal Soldiers in the Forest of
  Fallen Giants in SotFS, which the generator placement shows), Black Leather set's seller.
- **Aurous set.** Fextralife places the set in a Gutter chest and does not mention the transparent
  variant. The game has two sets with one name: 21361xxx in that chest (lot 10255110), and
  2136xxxx, the planner's "(Transparent)", handed out by Maughlin's lot 1761000. Fextralife's
  [Maughlin page](https://darksouls2.wiki.fextralife.com/Maughlin+the+Armourer) gives the
  condition: talk to him with no souls once 15,000 are spent, so the gift is dated with his
  15,000-souls shop tier.
- **Maughlin's boss sets.** The same page says spending 15,000 opens them (Velstadt's, Throne
  Defender's). The game gates each of those lines on its own flag (102601-102605), which the
  Smithbox labels name after the boss. The table follows the game.

## 2. When: area order and availability soul level

Each area has a soul level a player is taken to enter it at: the lower bound of the ranges in
[Dark Souls 2 Soul Levels by Zone Guide](https://www.gamingreality.com/2014/04/dark-souls-2-suggested-levels-by-zone.html)
(COMMUNITY, 2014): Majula 1, Forest of Fallen Giants 10, Heide's Tower 30, No-man's Wharf 35,
Grave of Saints 45, Lost Bastille and Huntsman's Copse 55, Harvest Valley 60, Earthen Peak and the
Gutter 65, Iron Keep, Shaded Woods and Doors of Pharros 80, Brightstone Cove 90, Drangleic Castle,
Sinners' Rise, Shrine of Amana and the Shrine of Winter 95, Undead Crypt and Aldia's Keep 100,
Dragon Aerie 105, Memories and the Throne of Want 110.

The DLC areas are not in that guide. They are set at or above the area that gates them
(Fextralife's DLC pages): Shulva 80 (Black Gulch, with the Forgotten Key), Brume Tower 90 (Iron
Keep), Eleum Loyce 100 (the Shrine of Winter). A search summary put the three at 80-100, 90-120 and
100+; its pages refused a fetch, so those ranges are not a cited source. NG+ starts at 120, the top
of the Throne of Want's range. A cumulative 15,000 souls spent at Maughlin is put at SL 30
(ASSUMED).

A piece's `avail_sl` is the earliest of its full-weight sources (pickup, shop, boss-soul), or of
any source when it has none of those. `tier` is that level's rank among the distinct area levels.

### Against the corpus

The corpus (SoulsPlanner and MugenMonkey builds, `load_corpus`) gives each piece an empirical
first-appearance: the 5th and 10th percentile soul level of builds wearing it (`corpus_sl_p05`,
`corpus_sl_p10`). Over the 416 pieces at least 20 builds wear, the 10th percentile correlates
with `avail_sl` at 0.48. The biggest disagreements are pieces worn far below where the game hands
them out: the Peasant set (worn from SL 31-50; every source is an Undead Peasant in Brightstone
Cove, 90) and Moon Butterfly (worn from SL 87; NG+ only). A planner does not enforce acquisition,
so the corpus is a statement of what people plan, not what they can have; it cannot replace the
area order, only flag where the order may be wrong.

## 3. Stat gates and load

Requirements: ArmorParam's four prerequisites (GAME), which the planner's `require` repeats.
Load: the recommender's own rule, 70% of max equip load (`EQUIP_CAP`).

Both are measured against the builds in the soul level's bracket, not against one median build.
For each bracket build: its stats with its rings (`gear_stats`), and the load left for one slot
once everything else it carries is counted (weapons, rings, its other three armour pieces). A
piece's `wearable(piece, SL)` is the share of the bracket's builds that meet its requirements and
have the room for its weight.

## 4. The weight

    w(piece, SL) = wearable(piece, SL) x max over the piece's sources s of
                   k(s.kind) x 1 / (1 + exp(-(SL - s.avail_sl) / 10))

- The logistic is a soft "has reached the area": half the players at the area's entry level, 73%
  ten levels later. Ten is the width of the guide's ranges.
- `k` is 1 for a pickup, a shop line or a boss-soul trade, and 0.5 for an enemy drop, an NPC's
  set, a covenant rank or an NG+ line (ASSUMED): each needs a farm, a kill most players do not
  make, a rank grind or a second playthrough. It is not fitted. Fitting it would mean choosing `k`
  and the width to maximize the likelihood of the corpus's per-bracket wear counts, with the
  caveat above that the corpus is planned, not owned.
- The maximum over sources, not the named source, so an early drop still counts before a late shop
  opens.

The opponent at a soul level is `build_defense`'s arithmetic with each slot an average over pieces
under `w`, worn at the bracket's median stats (`bracket_stats`). Defense is linear in each piece's
fields, so this is the expected defense of a set drawn slot by slot.

## 5. The weighted defender

`python3 scripts/ds2-armor-accessibility.py defender`. Four opponents per level, all but the first
at the bracket's median stats: the posted builds' own mean (`bracket_defense`, what the recommender
uses), every piece unweighted, the accessibility weighting, and the pieces posted builds actually
wear (a corpus check on the weighting).

| SL | opponent | phys | magic | fire | light | dark |
| --- | --- | --- | --- | --- | --- | --- |
| 30 | posted builds | 460 | 161 | 166 | 164 | 160 |
| 30 | all pieces | 530 | 159 | 160 | 144 | 166 |
| 30 | accessibility | 482 | 140 | 139 | 134 | 144 |
| 30 | posted pieces | 459 | 153 | 155 | 147 | 159 |
| 60 | posted builds | 511 | 164 | 173 | 162 | 165 |
| 60 | all pieces | 535 | 159 | 160 | 150 | 166 |
| 60 | accessibility | 472 | 150 | 149 | 149 | 157 |
| 60 | posted pieces | 509 | 156 | 161 | 149 | 164 |
| 100 | posted builds | 550 | 174 | 185 | 168 | 172 |
| 100 | all pieces | 539 | 159 | 160 | 150 | 166 |
| 100 | accessibility | 495 | 158 | 158 | 152 | 166 |
| 100 | posted pieces | 547 | 161 | 165 | 149 | 167 |
| 150 | posted builds | 593 | 193 | 211 | 190 | 191 |
| 150 | all pieces | 551 | 159 | 166 | 162 | 166 |
| 150 | accessibility | 525 | 161 | 166 | 164 | 168 |
| 150 | posted pieces | 584 | 170 | 180 | 167 | 178 |
| 200 | posted builds | 645 | 204 | 233 | 205 | 202 |
| 200 | all pieces | 568 | 165 | 178 | 182 | 172 |
| 200 | accessibility | 544 | 167 | 178 | 184 | 174 |
| 200 | posted pieces | 640 | 171 | 190 | 180 | 179 |
| 250 | posted builds | 678 | 227 | 259 | 226 | 224 |
| 250 | all pieces | 585 | 205 | 262 | 261 | 212 |
| 250 | accessibility | 565 | 207 | 262 | 264 | 214 |
| 250 | posted pieces | 678 | 215 | 279 | 261 | 223 |

What it says:

- **No opponent's elemental defense comes near 366**, the point above which Raw beats a 50/50
  split infusion. The closest is SL 250's weighted lightning, 264, about 100 short. Up to SL 200
  every weighted element is between 134 and 184. On elemental defense the split-infusion question
  does not turn on which of these opponents is used.
- **Accessibility pulls the opponent below both others at low levels**: at SL 30 it is 10-22 lower
  per element than the unweighted mean and 16-30 lower than the posted builds, because the
  high-elemental pieces are late (DLC, Drangleic) and the early ones carry little.
- **From SL 100 up the weighting and the unweighted mean coincide on elements** (within 3): by
  then nearly everything is available, and what is left of the weighting is the load and stat
  gates. Its physical defense stays 20-45 lower than the unweighted mean, because the load gate
  drops the heaviest pieces.
- **At SL 100-200 the posted builds out-defend every piece mix worn at median stats**, by 6-55
  per element (smallest on dark at SL 100, largest on fire at SL 200). The "posted pieces" row closes nearly all of the physical gap and about a third of
  the elemental one, so most of that elemental gap is the posted builds' own INT and FTH, not
  their armour. At SL 250 the piece mixes reach the posted builds on fire and pass them on
  lightning.

## 6. Gaps

- The area soul levels are one 2014 guide's, written for the original release; SotFS moved
  enemies (Royal Swordsmen and Ironclad Soldiers into the Forest of Fallen Giants), which the
  generator placement here sees and the guide does not. The DLC levels have no cited table.
- `k` and the logistic's width are assumptions, not fits.
- An NPC's set is dated by where the NPC is met or leaves, not by when their quest ends. Melentia's
  NPC lines need the NPC dead, which most players never do; `k` = 0.5 overstates them.
- A drop's percentage in the `source` column is the item's share of its lot's weights. Whether a
  `lotDropType` 0 lot picks one slot by weight, and how often it drops anything at all, was not
  traced in the executable; the weighting does not use the percentage.
- The covenant pieces are dated by where the covenant is joined, with no allowance for the rank
  grind.
- The corpus is planned builds. It sees no playthrough count and no acquisition, which is why
  Moon Butterfly and the Peasant set are "worn" long before they can be had.
