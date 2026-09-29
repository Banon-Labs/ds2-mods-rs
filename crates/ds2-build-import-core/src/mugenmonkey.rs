//! Reading `gon.savedStats={...}` out of a MugenMonkey page.
//!
//! # What the page carries
//!
//! `https://mugenmonkey.com/darksouls2/<id>` inlines the build as JSON, read off the live page for
//! build `181062` on 2026-09-29:
//!
//! ```text
//! gon.savedStats={"starting_class":"Bandit","lh1":97,...,"rh1":281,...,"ring1":25,...,
//!   "head":398,"chest":197,"hands":291,"legs":364,"Vigor":30,...,"item1":25,...,
//!   "covenant":"No Covenant","spell0":99,...,"two_hand":false,"gender":"f",...};
//! ```
//!
//! Unlike soulsplanner's literal this is real JSON, but the gear is numbers, and they are the
//! planner's own numbers rather than the game's item ids. `data/mugenmonkey.tsv` joins them to
//! names -- extracted from the planner's bundle by `scripts/ds2-mugenmonkey-tables.py` -- and the
//! names then go through [`crate::id_for`] like a soulsplanner name does. So a MugenMonkey page
//! becomes the same [`Build`] a soulsplanner page does, and nothing downstream knows which site it
//! was.
//!
//! # How an id is looked up, copied from the planner
//!
//! The page's own `findObjById(table, id)` returns the first row in table order with that id, and
//! armor is searched only in the slot's own list (`headPieces`, ...). [`name_for`] does both, so a
//! duplicated id (`weapon 75` is two scimitars) resolves the way the planner shows it.
//!
//! # What MugenMonkey does not record
//!
//! Infusions and reinforcement. Every weapon arrives uninfused, and the grant's own rule gives it
//! the item's maximum reinforcement, as it does for a soulsplanner weapon.

use crate::saved_build::{Build, MAX_STAT, ParseError, Stats};

/// `kind \t id \t name`, from `scripts/ds2-mugenmonkey-tables.py`.
const TABLE: &str = include_str!("../data/mugenmonkey.tsv");

/// What the build is assigned to on the page.
const MARKER: &str = "gon.savedStats=";

/// Weapon keys, in the order [`Build::weapons`] holds them: even positions left, odd right.
const WEAPON_KEYS: [&str; 6] = ["lh1", "rh1", "lh2", "rh2", "lh3", "rh3"];

/// Armor keys, which are also the table kinds their ids are looked up in.
const ARMOR_KEYS: [&str; 4] = ["head", "chest", "hands", "legs"];

const RING_KEYS: [&str; 4] = ["ring1", "ring2", "ring3", "ring4"];

/// `spell0` to `spell9`: the planner's ten attunement slots.
const SPELL_KEYS: [&str; 10] = [
    "spell0", "spell1", "spell2", "spell3", "spell4", "spell5", "spell6", "spell7", "spell8",
    "spell9",
];

const ITEM_KEYS: [&str; 5] = ["item1", "item2", "item3", "item4", "item5"];

/// What an armor slot's id `0` means. The planner draws it as `" Naked"`, and `naked` is one of
/// the names [`crate::is_empty_slot`] knows.
const NAKED: &str = "Naked";

/// soulsplanner's spelling of an uninfused weapon, so the weapon list is the same shape.
const NO_INFUSION: &str = "No_Infusion";

/// The planner's short forms, expanded to the catalogue's words when they end a name.
const SUFFIXES: [(&str, &str); 3] = [
    (" UGS", " Ultra Greatsword"),
    (" GS", " Greatsword"),
    (" GA", " Greataxe"),
];

/// Names the planner spells differently from the catalogue, left to right. Found by
/// `every_planner_name_resolves` against the table extracted 2026-09-29: typos, a word the
/// catalogue has in another form, and the planner's own variants of one item.
const SPELLINGS: [(&str, &str); 21] = [
    ("Ruler's Sword (1mil)", "Ruler's Sword"),
    ("Ruler's Sword (400k)", "Ruler's Sword"),
    ("Ruler's Sword (0)", "Ruler's Sword"),
    ("Curved Nil GS (NG)", "Curved Nil Greatsword"),
    ("Berseker Blade", "Berserker Blade"),
    ("Royal Solider Helm", "Royal Soldier Helm"),
    ("Creighton's Helm", "Creighton's Steel Mask"),
    ("Hollow Solider Helm", "Hollow Soldier Helm"),
    ("White Priest Headset", "White Priest Headpiece"),
    ("Desert Sorcesress Hood", "Desert Sorceress Hood"),
    ("Bone King Armor", "Bone King Robe"),
    ("Astrologist's Robes", "Astrologist's Robe"),
    ("Penal Straighjacket", "Penal Straightjacket"),
    ("Prisoner's Tatters (2)", "Prisoner's Tatters"),
    ("Sanctum Solider Gauntlets", "Sanctum Soldier Gauntlet"),
    ("Monastary Long Gloves", "Monastery Long Gloves"),
    ("Desert Soceress Gloves", "Desert Sorceress Gloves"),
    ("Royal Soliders Leggings", "Royal Soldier Leggings"),
    ("Hard Leather Leggings", "Hard Leather Boots"),
    ("Tights of Judgement", "Tights of Judgment"),
    ("Prisoner's Waistloth", "Prisoner's Waistcloth"),
];

/// The two stones, whose catalogue rows carry an HTML entity (`Smooth &amp; Silky Stone`), and
/// `Perserverance`. Kept apart from [`SPELLINGS`] only so that table stays one kind of fix.
const CATALOGUE_QUIRKS: [(&str, &str); 3] = [
    (
        "Small Smooth & Silky Stone",
        "Small Smooth &amp; Silky Stone",
    ),
    ("Smooth & Silky Stone", "Smooth &amp; Silky Stone"),
    ("Perserverance", "Perseverance"),
];

/// The catalogue's spelling of a planner name: the whole-name corrections first, then the
/// `GS`/`UGS`/`GA` suffix expansions, then the name unchanged.
pub fn catalogue_name(planner: &str) -> String {
    if let Some((_, fixed)) = SPELLINGS
        .iter()
        .chain(&CATALOGUE_QUIRKS)
        .find(|(from, _)| *from == planner)
    {
        return (*fixed).to_owned();
    }
    for (short, long) in SUFFIXES {
        if let Some(stem) = planner.strip_suffix(short) {
            return format!("{stem}{long}");
        }
    }
    planner.to_owned()
}

/// The name the planner shows for `id` in the `kind` table, or `None` for an id it does not have.
///
/// `kind` is `weapon`, `ring`, `spell`, `item`, or an armor slot (`head`, `chest`, `hands`,
/// `legs`). The first row wins, as it does on the page.
pub fn name_for(kind: &str, id: i64) -> Option<&'static str> {
    TABLE
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let mut columns = line.split('\t');
            Some((columns.next()?, columns.next()?, columns.next()?))
        })
        .find(|(row_kind, row_id, _)| *row_kind == kind && row_id.parse::<i64>().ok() == Some(id))
        .map(|(_, _, name)| name)
}

/// Parse the page a GET of a MugenMonkey build link returned.
///
/// `id` is the build id the caller asked for.
/// # Errors
///
/// [`ParseError::NoSavedBuild`] when the page carries no `gon.savedStats`,
/// [`ParseError::MalformedLiteral`] when it is not a flat JSON object, and the missing, wrong-typed
/// or out-of-range field otherwise. A gear id the table does not have is
/// [`ParseError::WrongType`] naming its key: a number the planner itself would not recognise.
pub fn parse(html: &str, id: u32) -> Result<Build, ParseError> {
    let at = html.find(MARKER).ok_or(ParseError::NoSavedBuild)?;
    let fields = parse_object(&html[at + MARKER.len()..]).ok_or(ParseError::MalformedLiteral)?;
    let get = |key: &'static str| {
        fields
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
            .ok_or(ParseError::MissingField(key))
    };
    let text = |key: &'static str| match get(key)? {
        Json::Text(value) => Ok(value.clone()),
        _ => Err(ParseError::WrongType(key)),
    };
    let number = |key: &'static str| match get(key)? {
        Json::Number(value) => Ok(*value),
        _ => Err(ParseError::WrongType(key)),
    };
    let stat = |key: &'static str| {
        let value = number(key)?;
        u16::try_from(value)
            .ok()
            .filter(|value| *value <= MAX_STAT)
            .ok_or(ParseError::StatOutOfRange { field: key, value })
    };
    let named = |kind: &str, key: &'static str| -> Result<String, ParseError> {
        let value = number(key)?;
        if ARMOR_KEYS.contains(&kind) && value == 0 {
            return Ok(NAKED.to_owned());
        }
        name_for(kind, value)
            .map(catalogue_name)
            .ok_or(ParseError::WrongType(key))
    };

    let mut weapons = Vec::with_capacity(WEAPON_KEYS.len() * 2);
    for key in WEAPON_KEYS {
        weapons.push(named("weapon", key)?);
        weapons.push(NO_INFUSION.to_owned());
    }
    let armor = ARMOR_KEYS
        .iter()
        .map(|key| named(key, key))
        .collect::<Result<Vec<_>, _>>()?;
    let rings = RING_KEYS
        .iter()
        .map(|key| named("ring", key))
        .collect::<Result<Vec<_>, _>>()?;
    let spells = SPELL_KEYS
        .iter()
        .map(|key| named("spell", key))
        .collect::<Result<Vec<_>, _>>()?;
    let items = ITEM_KEYS
        .iter()
        .map(|key| named("item", key))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Build {
        id,
        class: text("starting_class")?.to_ascii_lowercase(),
        // Carried without meaning, as soulsplanner's is: nothing applies it.
        gender: i64::from(text("gender").is_ok_and(|gender| gender == "f")),
        covenant: text("covenant")?,
        grip: match get("two_hand")? {
            Json::Bool(two_hand) => i64::from(*two_hand),
            _ => return Err(ParseError::WrongType("two_hand")),
        },
        armor,
        weapons,
        rings,
        spells,
        items,
        stats: Stats {
            vigor: stat("Vigor")?,
            endurance: stat("Endurance")?,
            vitality: stat("Vitality")?,
            attunement: stat("Attunement")?,
            strength: stat("Strength")?,
            dexterity: stat("Dexterity")?,
            adaptability: stat("Adaptability")?,
            intelligence: stat("Intelligence")?,
            faith: stat("Faith")?,
        },
    })
}

/// One value in the object. Nested values are not in the page's object and are refused.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Json {
    Text(String),
    Number(i64),
    Bool(bool),
    Null,
}

/// A flat JSON object at the start of `text`, as `(key, value)` pairs in order.
///
/// Hand-written for the reason `saved_build` gives: this crate has no dependencies, and the object
/// is one level deep with four value kinds. Fractional numbers are refused; the page has none.
fn parse_object(text: &str) -> Option<Vec<(String, Json)>> {
    let mut chars = text.chars().peekable();
    let skip = |chars: &mut std::iter::Peekable<std::str::Chars<'_>>| {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
    };
    skip(&mut chars);
    if chars.next()? != '{' {
        return None;
    }
    let mut fields = Vec::new();
    loop {
        skip(&mut chars);
        if chars.next_if_eq(&'}').is_some() {
            return Some(fields);
        }
        if chars.next()? != '"' {
            return None;
        }
        let key = string(&mut chars)?;
        skip(&mut chars);
        if chars.next()? != ':' {
            return None;
        }
        skip(&mut chars);
        let value = match *chars.peek()? {
            '"' => {
                chars.next();
                Json::Text(string(&mut chars)?)
            }
            't' | 'f' | 'n' => {
                let word: String =
                    std::iter::from_fn(|| chars.next_if(char::is_ascii_alphabetic)).collect();
                match word.as_str() {
                    "true" => Json::Bool(true),
                    "false" => Json::Bool(false),
                    "null" => Json::Null,
                    _ => return None,
                }
            }
            _ => {
                let digits: String =
                    std::iter::from_fn(|| chars.next_if(|c| c.is_ascii_digit() || *c == '-'))
                        .collect();
                Json::Number(digits.parse().ok()?)
            }
        };
        fields.push((key, value));
        skip(&mut chars);
        match chars.next()? {
            ',' => {}
            '}' => return Some(fields),
            _ => return None,
        }
    }
}

/// The rest of a JSON string whose opening quote has been taken.
fn string(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<String> {
    let mut out = String::new();
    loop {
        match chars.next()? {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                'b' | 'f' => {}
                'u' => {
                    let hex: String = (0..4).map(|_| chars.next()).collect::<Option<_>>()?;
                    // A lone surrogate becomes the replacement character: the fields that could
                    // carry one are the title and the description, which nothing here reads.
                    out.push(
                        char::from_u32(u32::from_str_radix(&hex, 16).ok()?).unwrap_or('\u{fffd}'),
                    );
                }
                other => out.push(other),
            },
            other => out.push(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The object on the live page for build 181062, 2026-09-29, verbatim, inside a page-shaped
    /// wrapper.
    const PAGE: &str = concat!(
        "<html><body><script>window.gon={};",
        r#"gon.savedStats={"starting_class":"Bandit","lh1":97,"lh2":0,"lh3":0,"rh1":281,"rh2":0,"rh3":0,"ring1":25,"ring2":38,"ring3":44,"ring4":10,"head":398,"chest":197,"hands":291,"legs":364,"Vigor":30,"Endurance":40,"Vitality":11,"Attunement":40,"Strength":60,"Dexterity":60,"Adaptability":3,"Intelligence":24,"Faith":54,"item1":25,"item2":5,"item3":30,"item4":-1,"item5":-1,"covenant":"No Covenant","spell0":99,"spell1":103,"spell2":23,"spell3":97,"spell4":0,"spell5":0,"spell6":0,"spell7":0,"spell8":0,"spell9":0,"two_hand":false,"soulMemory":16386499,"gender":"f","author":"Gabriel_xxt","public":true,"title":"Untitled","description":null};"#,
        r#"gon.gameInfo={"game":"darksouls2","gameID":2,"buildID":181062,"yourBuild":false};"#,
        "</script></body></html>"
    );

    #[test]
    fn the_live_page_becomes_a_build() {
        let build = parse(PAGE, 181_062).unwrap();
        assert_eq!(build.id, 181_062);
        assert_eq!(build.class, "bandit");
        assert_eq!(build.covenant, "No Covenant");
        assert_eq!(build.grip, 0);
        assert_eq!(
            build.weapons,
            [
                "Espada Ropera",
                NO_INFUSION,
                "Blossom Kite Shield",
                NO_INFUSION,
                "Bare Fists",
                NO_INFUSION,
                "Bare Fists",
                NO_INFUSION,
                "Bare Fists",
                NO_INFUSION,
                "Bare Fists",
                NO_INFUSION,
            ]
        );
        assert_eq!(
            build.armor,
            [
                "Crown of the Ivory King",
                "Llewellyn Armor",
                "Llewellyn Gloves",
                "Llewellyn Shoes"
            ]
        );
        assert_eq!(
            build.rings,
            [
                "Flynn's Ring",
                "Old Leo Ring",
                "Ring of Blades",
                "Chloranthy Ring"
            ]
        );
        assert_eq!(
            &build.spells[..5],
            [
                "Sacred Oath",
                "Sunlight Blade",
                "Whisper of Despair",
                "Replenishment",
                "No Spell"
            ]
        );
        assert_eq!(
            build.items,
            [
                "Dried Root",
                "Elizabeth Mushroom",
                "Brightbug",
                "No Item",
                "No Item"
            ]
        );
        assert_eq!(
            build.stats.each().map(|(_, value)| value),
            [30, 40, 11, 40, 60, 60, 3, 24, 54]
        );
    }

    /// Every empty slot the page can carry is one the grant already skips.
    #[test]
    fn the_empty_slots_are_the_ones_the_grant_skips() {
        let build = parse(PAGE, 181_062).unwrap();
        for empty in [
            "Bare Fists",
            "No Spell",
            "No Item",
            NO_INFUSION,
            NAKED,
            "No Ring",
        ] {
            assert!(crate::is_empty_slot(empty), "{empty}");
        }
        assert_eq!(
            build
                .spells
                .iter()
                .filter(|spell| crate::is_empty_slot(spell))
                .count(),
            6
        );
    }

    /// Armor id 0 is the slot left empty, whichever slot, not the first `default...` row.
    #[test]
    fn an_empty_armor_slot_is_naked() {
        let page = PAGE.replace(r#""head":398"#, r#""head":0"#);
        assert_eq!(parse(&page, 1).unwrap().armor[0], NAKED);
    }

    /// The planner's first-row rule, on the one weapon id it has twice.
    #[test]
    fn a_duplicated_id_resolves_to_the_first_row_as_the_page_does() {
        assert_eq!(name_for("weapon", 75), Some("Monastery Scimitar"));
    }

    #[test]
    fn a_page_without_the_build_is_refused() {
        assert_eq!(
            parse("<html><body>no build</body></html>", 1),
            Err(ParseError::NoSavedBuild)
        );
        assert_eq!(
            parse("gon.savedStats=[1,2];", 1),
            Err(ParseError::MalformedLiteral)
        );
    }

    /// An id the planner itself does not have names the key it was under.
    #[test]
    fn an_unknown_gear_id_names_its_key() {
        let page = PAGE.replace(r#""rh1":281"#, r#""rh1":99999"#);
        assert_eq!(parse(&page, 1), Err(ParseError::WrongType("rh1")));
    }

    #[test]
    fn strings_with_escapes_parse() {
        let fields = parse_object(r#"{"a":"x\"y\\zé","b":null,"c":-1,"d":true} trailing"#).unwrap();
        assert_eq!(fields[0].1, Json::Text("x\"y\\z\u{e9}".to_owned()));
        assert_eq!(fields[1].1, Json::Null);
        assert_eq!(fields[2].1, Json::Number(-1));
        assert_eq!(fields[3].1, Json::Bool(true));
    }

    /// How much of the planner's gear the game catalogue knows by name. Every name that is not an
    /// empty slot either resolves to an item id or fails this, so a catalogue or table change that
    /// loses a name fails here instead of in a player's grant.
    #[test]
    fn every_planner_name_resolves() {
        let mut unresolved = Vec::new();
        for line in TABLE.lines().filter(|line| !line.starts_with('#')) {
            let mut columns = line.split('\t');
            let (Some(kind), Some(_), Some(name)) =
                (columns.next(), columns.next(), columns.next())
            else {
                continue;
            };
            if crate::is_empty_slot(name) || name.starts_with("default") {
                continue;
            }
            // Unsafe-to-spawn is a resolution: the catalogue knows the item and the grant refuses
            // it on purpose (the Darksign, the Crushed Eye Orb).
            match crate::id_for(&catalogue_name(name)) {
                Ok(_)
                | Err(
                    crate::ItemError::Ambiguous { .. } | crate::ItemError::UnsafeToSpawn { .. },
                ) => {}
                Err(_) => unresolved.push(format!("{kind}\t{name}")),
            }
        }
        assert!(
            unresolved.is_empty(),
            "{} names do not resolve:\n{}",
            unresolved.len(),
            unresolved.join("\n")
        );
    }
}
