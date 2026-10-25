# Stance swapping: what the left hand and the grip add to a PvP build

Read 2026-10-24. Tags: **REGULATION** = read from the installed `regulation` params,
**PARAMDEF** = FromSoft's own (Japanese) field names in the paramdef, **COMMUNITY** = darksouls2.wiki.gg
(datamine-based) or Fextralife, **INFERRED** = fits the data, the consumer was not traced, **EXE** =
read from the executable. Nothing here was measured in a running game.

The scorer is `stance_metrics` / `choose_offhand` in `scripts/ds2-builds-recommend.py`, ported to
`crates/ds2-build-recommender-core/src/corpus/stance.rs`.

## 1. What "stance swapping" is in DS2

DS2 has no single technique called a stance swap. A build changes how it holds what it carries,
between engagements, in four ways, and each way has its own attack slots in
`WeaponActionCategoryParam`:

| Stance | Input | Attack slots it plays | Tag |
|---|---|---|---|
| One-handed | default | right weapon `Single1Hand*`; left weapon `Left1Hand*` on L1/L2 | PARAMDEF (`Pian Shou `, `Zuo Pian Shou `) |
| Right weapon two-handed | tap Y | `Single2Hand*`; L1 guards, L2 `OppositeSingle2HandStrong` (parry, or a strike on ultra greatswords, great hammers, curved greatswords) | PARAMDEF (`Liang Shou `, `Liang Shou Ni Wan `), REGULATION |
| Left weapon two-handed | hold Y | the left weapon's `Single2Hand*` and its L2 | COMMUNITY (wiki.gg, Fextralife controls) |
| Power stance | hold Y with a pair that can | R1/R2 `Dual2Hand*` (the same motions as `Single1Hand*` on all 972 rows compared); L1, L1 L1, L2 `OppositeDual2Hand*`, both blades striking | PARAMDEF (`Er Dao Liu Liang Shou `, `Er Dao Liu Liang Shou Ni Wan `), REGULATION |

Inside the one-handed stance with a weapon in each hand:

- **Cross-hand strings**: an attack of one hand, then the other hand's. When the pair meets power
  stance's stats (and is not in power stance), the follow-up is its weapon's **chain attack**,
  `Dual1Hand*` (`Er Dao Liu `). COMMUNITY (wiki.gg Dual_Wielding: "the requirements of each weapon to
  enter a Power Stance are met"). In the regulation 194 weapons' chain R1 differs from their plain
  R1, almost all by play speed (165) and a few by motion value (the katanas' 1.0 -> 1.2), which
  matches wiki.gg's per-class "faster / longer recovery / less hitstun" list.
- **Attacks out of a raised shield** (`GuardNormal*`, `ga-doGou e`): 42 weapons, all spears,
  rapiers, halberds and lances, with a shield in the left hand. REGULATION.

## 2. Power stance

- **Who pairs** (`POWER_STANCE`): wiki.gg's category table. The category is
  `WeaponTypeParam.dualWieldingPermission` (REGULATION), whose values rise in exactly the page's
  priority order and whose reclassified weapons are exactly the page's (Crypt Blacksword and King's
  Ultra Greatsword 110 with the greataxes, Spitfire and Yorgh's Spear 160 with the lances,
  Archdrake Mace and Drakekeeper's Warpick 140 with the halberds, Helix Halberd 150 with the
  spears, Silverblack Sickle 90 with the axes). 0 = cannot: every shield but Reeve's (220) and
  Orma's (230), staves, chimes, flames, bows.
- **Whose L1/L2 plays**: the right hand's weapon for a same-priority pair, else the
  higher-priority category's. COMMUNITY.
- **Stats**: STR and DEX at 1.5 x the higher one-handed requirement of the two, rounded down; INT and
  FTH only as normal. COMMUNITY; the 1.5x and that the grip writer halves nothing in power stance
  are EXE (`ds2-grip-state-chain`).
- **Damage**: each blade's hit at its own attack rating. The `OppositeDual2Hand*` rows carry every
  hitbox twice, once with a damage row and once without (the other blade). COMMUNITY + REGULATION;
  the second blade striking at the first's motion value is INFERRED.
- **Stamina**: the owner's `WeaponStaminaCostParam` `dual2HandOpposite*` fields. REGULATION.

## 3. The left hand's damage

`WeaponTypeParam.leftDamageScale` is 0.9 on every melee weapon and 1.0 or 0.865 on a few shields
and crossbows (REGULATION). It is the hand scale the attack builder keeps in `k` for the left hand,
read the same way as `rightDamageScale` for launchers (docs/DS2-DPS-MECHANICS.md). Applied to every
attack from the left slot, the left weapon two-handed included: INFERRED (the weapon stays in the
left slot).

## 4. Two-handing

Two-handing halves the STR requirement (rounded down) and does not multiply STR for attack rating
(EXE, docs/DS2-DPS-MECHANICS.md "Two-handing"). The community's "STR x1.5" is DS1's rule, and
Fextralife's DS2 Strength page says "doubled ... only for weapon requirements".

## 5. How it is scored

Each stance is scored as `moveset_metrics` scores a grip: per family (R1, R2, movement, jump) the
best engagement, the weapon's rate the matching law sum(s^2)/sum(s) over the families. The other
hand's engagements (its L1/L2 moveset, cross-hand strings, power stance L1/L2, the two-handed L2)
join a family only where they raise the rate: the matching law falls when a family under half of it
rises, and an option the build carries is one it may leave unused. The build stands in the best
stance that swings its right weapon (1H, 2H or PS); a family may take another stance's engagement
at `STANCE_SWAP` more seconds, again only where it raises the rate. So a build's stance rate is
never under its primary's best grip alone.

`choose_offhand` tries nothing, the primary's twin, the lightest shield when the primary attacks out
of a guard, the build's ten best melee weapons at its stats (each at its best infusion there) and
the next five that power stance with the primary, and keeps the highest stance rate. A generated
build only tries left hands no heavier than `offhand_room`: what still leaves its lightest full set
of armour under 70% load beside the heaviest listed weapon and the rings (before it, a second
great hammer left a SL 100 build no armour at all). It reports the best-ranked power stance partner
the stats fall short of, with the STR and DEX it needs. Poison and bleed build-up is kept per
weapon and infusion, so a Dark Uchigatana in the left hand does not bleed because a Bleed one is in
the right.

`--rank stance` (`scripts/ds2-bracket-sweep.sh`'s default) ranks every primary by that rate, and the
bracket builds put the chosen left hand in LH1.

## 6. Not counted, and why

| What | Why |
|---|---|
| Grip-change time | `STANCE_SWAP` is a stand-in (er-mods-rs's 20-frame roll entry): DS2's grip-change animations were not read and no frame data was found. INFERRED |
| Block, guard stability, chip | No guard model in the moveset rate |
| Parries (2H L2, small shields, parrying dagger, curved swords' L2) and ripostes | No crit model (p5z4.15); a parry row has no damage row |
| Guard break (forward + R1) | No damage row |
| Accel (startup cut after certain attacks) | wiki.gg points to an external spreadsheet; no numbers read |
| Chain attacks' lower hitstun | The follow-up land chance is `FOLLOW_UNSTAGGERED` for any follow-up |
| Weapon swap (RH2/LH2) | A soft swap is not part of an optimal build (user, 2026-10-24) |
| The Majestic Greatsword's separate left-handed category | The regulation has one category for it |
| Whips with daggers | wiki.gg's claws row lists whips; the daggers row does not; kept as listed |

## 7. Sources

- https://darksouls2.wiki.gg/wiki/Dual_Wielding (categories, priority, stats, chain attacks)
- https://darksouls2.wiki.gg/wiki/Parry (parry frames per tool)
- https://darksouls2.wiki.fextralife.com/Power+Stance, /Controls, /Strength
- Smithbox DS2S paramdef and its Japanese annotations: `WEAPON_ACTION_CATEGORY_PARAM`,
  `WEAPON_TYPE_PARAM`, `WEAPON_STAMINA_COST_PARAM`
