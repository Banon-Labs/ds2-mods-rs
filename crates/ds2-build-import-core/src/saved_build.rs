//! Reading `savedBuild={...}` out of a soulsplanner page.
//!
//! # Why there is a parser here at all and not a JSON one
//!
//! What the server inlines is not JSON. It is a **JavaScript object literal** -- barewords for
//! keys, single quotes for strings, no quoting on the numbers -- so no JSON parser will read it,
//! and pulling in one that tolerates JavaScript would be a dependency for a grammar this flat.
//! The literal is one level deep with two value kinds, and that is the whole grammar this module
//! implements. `scripts/ds2-soulsplanner.py` does the same job with three regexes.
//!
//! # The body is the test, not the script tag
//!
//! A request that returns no build still returns a page, and that page still has a
//! `<body><script>` -- carrying the bootstrap `;var plannerId='darksouls2';` and nothing else. So
//! the thing that distinguishes a real answer from an empty one is whether `savedBuild` appears
//! AFTER `<body>`, which is what [`parse`] looks for. Finding it in the head would mean the page
//! changed shape and the assumption needs re-measuring, so that is refused rather than accepted.
//!
//! # The list fields are one string with semicolons in it
//!
//! `armor`, `weapons`, `rings`, `spells` and `items` arrive as a single quoted string of
//! `;`-separated names. Every other field is a bareword string or an integer. The empty slots are
//! spelled out (`No_Spell`, `No_Infusion`, `Bare_Fists`) rather than omitted, so a list's LENGTH
//! is fixed by the planner and carries no information -- it is not validated here.

/// Fields whose single quoted string is really a `;`-separated list.
const LIST_FIELDS: [&str; 5] = ["armor", "weapons", "rings", "spells", "items"];

/// The key the starting class arrives under, **trailing underscore and all**.
///
/// It is `class_`, not `class`, in the bytes the server sends -- read off the live page for build
/// `253` on 2026-08-28. This looks exactly like a Python author renaming a reserved word on the
/// way out, and it is not: `scripts/ds2-soulsplanner.py` copies every key verbatim, and the
/// underscore is in the response. Assuming it away cost one red integration test, which is the
/// cheapest place this could have been found.
const CLASS_KEY: &str = "class_";

/// A build, as the planner recorded it.
///
/// Unknown keys are DROPPED rather than refused: the planner may add a field, and a build that
/// fails to load because the site gained a column nobody here uses would be the wrong failure.
/// Missing KNOWN keys are a different matter and come back as [`ParseError::MissingField`].
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Build {
    /// The id from the URL. Not in the literal -- the caller knows it, the page does not repeat it.
    pub id: u32,
    /// Starting class, lowercase with underscores (`swordsman`, `cleric`).
    pub class: String,
    /// `0` or `1`. The planner does not say which is which and neither does this.
    pub gender: i64,
    /// Covenant name with underscores (`Brotherhood_of_Blood`), or `No_Covenant`.
    pub covenant: String,
    /// Which hand grip the planner drew. Carried through without interpretation.
    pub grip: i64,
    /// Head, chest, hands, legs.
    pub armor: Vec<String>,
    /// Interleaved `name, infusion` pairs across the six weapon slots.
    pub weapons: Vec<String>,
    /// Four ring slots, `No_Ring` where empty.
    pub rings: Vec<String>,
    /// Attunement slots, `No_Spell` where empty.
    pub spells: Vec<String>,
    /// Consumable slots, `No_Item` where empty.
    pub items: Vec<String>,
    /// The nine levelled stats.
    pub stats: Stats,
}

/// The nine stats a Dark Souls 2 character levels.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Stats {
    pub vigor: u16,
    pub endurance: u16,
    pub vitality: u16,
    pub attunement: u16,
    pub strength: u16,
    pub dexterity: u16,
    pub adaptability: u16,
    pub intelligence: u16,
    pub faith: u16,
}

/// The highest a Dark Souls 2 stat goes. A page claiming more is a page this parser misread.
pub const MAX_STAT: u16 = 99;

impl Stats {
    /// Every stat, with its name, in the order the planner emits them.
    pub const fn each(&self) -> [(&'static str, u16); 9] {
        [
            ("vigor", self.vigor),
            ("endurance", self.endurance),
            ("vitality", self.vitality),
            ("attunement", self.attunement),
            ("strength", self.strength),
            ("dexterity", self.dexterity),
            ("adaptability", self.adaptability),
            ("intelligence", self.intelligence),
            ("faith", self.faith),
        ]
    }

    /// The nine stats **in the order `PlayerParam` stores them**, which is NOT the planner's order.
    ///
    /// # The one difference, and it is silent
    ///
    /// soulsplanner emits adaptability SEVENTH; the game stores it LAST, after intelligence and
    /// faith. Every other stat is in the same place. So a caller that hands [`Self::each`]'s values
    /// to the game writes adaptability into intelligence, intelligence into faith and faith into
    /// adaptability -- three stats wrong, no error, and a character whose total is right so the
    /// LEVEL still comes out correct. That is the worst shape a bug can have: the number you would
    /// check agrees, and the character is wrong.
    ///
    /// The order is asserted against `ds2_rva::PLAYER_PARAM_STAT_NAMES` in `ds2-build-import`, the
    /// one crate that depends on both, so the two cannot drift apart without a test failing.
    pub const fn in_game_order(&self) -> [u16; 9] {
        [
            self.vigor,
            self.endurance,
            self.vitality,
            self.attunement,
            self.strength,
            self.dexterity,
            self.intelligence,
            self.faith,
            self.adaptability,
        ]
    }
}

/// Why a page did not yield a build.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ParseError {
    /// No `<body>` in the response. Not a soulsplanner page.
    NoBody,
    /// The page loaded and carries no build -- the bootstrap-only response. Almost always the
    /// fragment form of the URL, or an id that does not exist.
    NoSavedBuild,
    /// `savedBuild` was found, but not as `savedBuild = { ... };`. The page changed shape.
    MalformedLiteral,
    /// A key this parser needs was not in the literal.
    MissingField(&'static str),
    /// A key was there with the wrong kind of value.
    WrongType(&'static str),
    /// A stat was outside `0..=MAX_STAT`, which means the value did not come from where we think.
    StatOutOfRange { field: &'static str, value: i64 },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::NoBody => write!(f, "the response has no <body> -- not a planner page"),
            ParseError::NoSavedBuild => write!(
                f,
                "the page carries no savedBuild -- the id may not exist, or the link used the # form"
            ),
            ParseError::MalformedLiteral => {
                write!(f, "savedBuild is there but is not an object literal")
            }
            ParseError::MissingField(field) => write!(f, "savedBuild has no {field}"),
            ParseError::WrongType(field) => write!(f, "savedBuild's {field} is the wrong kind"),
            ParseError::StatOutOfRange { field, value } => {
                write!(f, "{field}={value} is outside 0..={MAX_STAT}")
            }
        }
    }
}

/// One value in the literal: a single-quoted string or a bare integer.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Value {
    Text(String),
    Number(i64),
}

/// Parse the page a GET of [`crate::build_path`] returned.
///
/// `id` is the build id the caller asked for; the literal does not carry it.
pub fn parse(html: &str, id: u32) -> Result<Build, ParseError> {
    let fields = parse_fields(saved_build_literal(html)?);

    let text = |key: &'static str| -> Result<String, ParseError> {
        match fields.iter().find(|(name, _)| name == key) {
            Some((_, Value::Text(value))) => Ok(value.clone()),
            Some((_, Value::Number(_))) => Err(ParseError::WrongType(key)),
            None => Err(ParseError::MissingField(key)),
        }
    };
    let number = |key: &'static str| -> Result<i64, ParseError> {
        match fields.iter().find(|(name, _)| name == key) {
            Some((_, Value::Number(value))) => Ok(*value),
            Some((_, Value::Text(_))) => Err(ParseError::WrongType(key)),
            None => Err(ParseError::MissingField(key)),
        }
    };
    let stat = |key: &'static str| -> Result<u16, ParseError> {
        let value = number(key)?;
        u16::try_from(value)
            .ok()
            .filter(|value| *value <= MAX_STAT)
            .ok_or(ParseError::StatOutOfRange { field: key, value })
    };
    let list = |key: &'static str| -> Result<Vec<String>, ParseError> {
        debug_assert!(LIST_FIELDS.contains(&key), "{key} is not a list field");
        let raw = text(key)?;
        Ok(split_list(&raw))
    };

    Ok(Build {
        id,
        class: text(CLASS_KEY)?,
        gender: number("gender")?,
        covenant: text("covenant")?,
        grip: number("grip")?,
        armor: list("armor")?,
        weapons: list("weapons")?,
        rings: list("rings")?,
        spells: list("spells")?,
        items: list("items")?,
        stats: Stats {
            vigor: stat("vigor")?,
            endurance: stat("endurance")?,
            vitality: stat("vitality")?,
            attunement: stat("attunement")?,
            strength: stat("strength")?,
            dexterity: stat("dexterity")?,
            adaptability: stat("adaptability")?,
            intelligence: stat("intelligence")?,
            faith: stat("faith")?,
        },
    })
}

/// The inside of `savedBuild = { ... }`, found after `<body>`.
fn saved_build_literal(html: &str) -> Result<&str, ParseError> {
    // AFTER `<body>`, because the bootstrap-only response has a body script too and the
    // distinguishing fact is which script carries the assignment. See the module docs.
    let body = html.find("<body").ok_or(ParseError::NoBody)?;
    let after_body = &html[body..];
    let at = after_body
        .find("savedBuild")
        .ok_or(ParseError::NoSavedBuild)?;

    let rest = after_body[at + "savedBuild".len()..].trim_start();
    let rest = rest.strip_prefix('=').ok_or(ParseError::MalformedLiteral)?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('{').ok_or(ParseError::MalformedLiteral)?;

    // Scan for the closing brace, skipping anything inside a single-quoted string -- the item and
    // spell names are barewords today, but a name with a brace in it must not end the object.
    let bytes = rest.as_bytes();
    let mut quoted = false;
    for (index, byte) in bytes.iter().enumerate() {
        match byte {
            b'\'' => quoted = !quoted,
            b'}' if !quoted => return Ok(&rest[..index]),
            _ => {}
        }
    }
    Err(ParseError::MalformedLiteral)
}

/// Every `key: value` pair in the literal, in the order they appear.
///
/// Anything that is not a bareword key followed by a quoted string or an integer is SKIPPED rather
/// than refused, so a field this parser does not know about cannot fail a build that is otherwise
/// complete.
fn parse_fields(literal: &str) -> Vec<(String, Value)> {
    let mut fields = Vec::new();
    let bytes = literal.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        // A key is a run of word bytes followed, after any spaces, by a colon.
        if !is_word_byte(bytes[at]) {
            at += 1;
            continue;
        }
        let key_start = at;
        while at < bytes.len() && is_word_byte(bytes[at]) {
            at += 1;
        }
        let key = &literal[key_start..at];
        let after_key = at;
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if at >= bytes.len() || bytes[at] != b':' {
            at = after_key;
            continue;
        }
        at += 1;
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if at >= bytes.len() {
            break;
        }
        if bytes[at] == b'\'' {
            at += 1;
            let value_start = at;
            while at < bytes.len() && bytes[at] != b'\'' {
                at += 1;
            }
            if at >= bytes.len() {
                break;
            }
            fields.push((
                key.to_owned(),
                Value::Text(literal[value_start..at].to_owned()),
            ));
            at += 1;
        } else if bytes[at] == b'-' || bytes[at].is_ascii_digit() {
            let value_start = at;
            at += 1;
            while at < bytes.len() && bytes[at].is_ascii_digit() {
                at += 1;
            }
            if let Ok(value) = literal[value_start..at].parse::<i64>() {
                fields.push((key.to_owned(), Value::Number(value)));
            }
        }
    }
    fields
}

/// Split a `;`-delimited list field the way the planner writes it.
///
/// An empty string is an empty list rather than one empty name -- `"".split(';')` yields one
/// element, and a build with no armour would otherwise arrive with a single nameless piece.
fn split_list(raw: &str) -> Vec<String> {
    if raw.is_empty() {
        return Vec::new();
    }
    raw.split(';').map(str::to_owned).collect()
}

/// Bytes a JavaScript bareword key is made of. `\w` in the python, and the same set.
const fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **THE REORDER, PINNED.** The planner's seventh stat is the game's ninth.
    ///
    /// Written with nine DISTINCT values so a wrong permutation cannot pass by coincidence -- with
    /// two stats equal, three of the six possible swaps still compare equal.
    #[test]
    fn the_game_stores_adaptability_last_and_the_planner_does_not() {
        let stats = Stats {
            vigor: 1,
            endurance: 2,
            vitality: 3,
            attunement: 4,
            strength: 5,
            dexterity: 6,
            adaptability: 7,
            intelligence: 8,
            faith: 9,
        };
        // The planner's order, adaptability seventh.
        assert_eq!(
            stats.each().map(|(_, value)| value),
            [1, 2, 3, 4, 5, 6, 7, 8, 9]
        );
        // The game's order, adaptability last. Only the tail of three differs.
        assert_eq!(stats.in_game_order(), [1, 2, 3, 4, 5, 6, 8, 9, 7]);
    }

    /// The two orders hold the same nine values, so the LEVEL is identical either way -- which is
    /// exactly why the level cannot be used to detect the wrong one.
    #[test]
    fn the_reorder_does_not_change_the_total() {
        let stats = Stats {
            vigor: 50,
            endurance: 20,
            vitality: 4,
            attunement: 16,
            strength: 25,
            dexterity: 16,
            adaptability: 16,
            intelligence: 28,
            faith: 28,
        };
        let planner: u32 = stats.each().iter().map(|(_, v)| u32::from(*v)).sum();
        let game: u32 = stats.in_game_order().iter().map(|v| u32::from(*v)).sum();
        assert_eq!(planner, game);
        assert_eq!(planner, 203, "build 253's total, which is level 150");
    }

    /// The shape of a real response, reduced to the parts this module reads. Values are build 253
    /// as fetched on 2026-08-28 via `scripts/ds2-soulsplanner.py 253`.
    const PAGE: &str = "<html><head><script>;var plannerId='darksouls2';</script></head>\
<body>  <script>savedBuild={class_:'swordsman',gender:0,covenant:'Brotherhood_of_Blood',\
armor:'Black_Hood;Armor_of_the_Forlorn;Shadow_Gauntlets;Leggings_of_the_Forlorn',\
weapons:'Caithas_Chime;Dark;Greatsword_of_the_Forlorn;Bleed',grip:0,\
rings:'Ring_of_Binding;Flynns_Ring;Crest_of_Blood;Third_Dragon_Ring',\
spells:'Dark_Weapon;Resonant_Soul',items:'Lifegem;No_Item',\
vigor:50,endurance:20,vitality:4,attunement:16,strength:25,dexterity:16,\
adaptability:16,intelligence:28,faith:28};</script></body></html>";

    /// A page with a build in it reads back exactly what the planner recorded.
    #[test]
    fn a_real_page_yields_the_build_it_carries() {
        let build = parse(PAGE, 253).expect("the fixture is a complete build");
        assert_eq!(build.id, 253);
        assert_eq!(build.class, "swordsman");
        assert_eq!(build.covenant, "Brotherhood_of_Blood");
        assert_eq!(build.gender, 0);
        assert_eq!(build.grip, 0);
        assert_eq!(build.armor.len(), 4);
        assert_eq!(build.armor[0], "Black_Hood");
        assert_eq!(build.rings[3], "Third_Dragon_Ring");
        assert_eq!(build.stats.vigor, 50);
        assert_eq!(build.stats.faith, 28);
        assert_eq!(build.stats.vitality, 4);
    }

    /// The BOOTSTRAP response is the failure this parser exists to name.
    ///
    /// It is what `/darksouls2/#253` returns every time, and what a nonexistent id returns. It is a
    /// perfectly valid page, so nothing upstream of here notices.
    #[test]
    fn the_bootstrap_only_page_is_refused_by_name() {
        let bootstrap =
            "<html><head></head><body><script>;var plannerId='darksouls2';</script></body></html>";
        assert_eq!(parse(bootstrap, 253), Err(ParseError::NoSavedBuild));
    }

    /// `savedBuild` in the HEAD is a page that changed shape, not a build.
    ///
    /// Accepting it would mean the one thing separating a real answer from an empty one had
    /// quietly stopped being checked.
    #[test]
    fn a_saved_build_before_the_body_is_not_accepted() {
        let head_only = "<html><head><script>savedBuild={class_:'swordsman'};</script></head><body></body></html>";
        assert_eq!(parse(head_only, 1), Err(ParseError::NoSavedBuild));
    }

    /// A page with no body at all is not a planner page.
    #[test]
    fn a_page_with_no_body_is_refused() {
        assert_eq!(parse("<html></html>", 1), Err(ParseError::NoBody));
    }

    /// A missing key names itself, so the log says which field the site dropped.
    #[test]
    fn a_missing_field_names_itself() {
        let page = "<body><script>savedBuild={gender:0};</script></body>";
        assert_eq!(parse(page, 1), Err(ParseError::MissingField(CLASS_KEY)));
    }

    /// A field of the wrong kind is not silently coerced.
    #[test]
    fn a_field_of_the_wrong_kind_is_refused() {
        let page = "<body><script>savedBuild={class_:7,gender:0};</script></body>";
        assert_eq!(parse(page, 1), Err(ParseError::WrongType(CLASS_KEY)));
    }

    /// A stat past 99 means the value did not come from where we think it did.
    #[test]
    fn a_stat_past_the_games_ceiling_is_refused() {
        let page = format!(
            "<body><script>savedBuild={{class_:'swordsman',gender:0,covenant:'x',grip:0,\
armor:'',weapons:'',rings:'',spells:'',items:'',vigor:{},endurance:1,vitality:1,attunement:1,\
strength:1,dexterity:1,adaptability:1,intelligence:1,faith:1}};</script></body>",
            MAX_STAT + 1
        );
        assert_eq!(
            parse(&page, 1),
            Err(ParseError::StatOutOfRange {
                field: "vigor",
                value: i64::from(MAX_STAT) + 1
            })
        );
    }

    /// An unknown key is dropped, not refused -- the site may add a column.
    #[test]
    fn an_unknown_field_does_not_fail_a_complete_build() {
        let page = PAGE.replace("grip:0", "grip:0,newThingNobodyHereKnowsAbout:'x'");
        assert_eq!(
            parse(&page, 253).map(|build| build.class),
            Ok("swordsman".to_owned())
        );
    }

    /// An empty list field is no items, not one nameless item.
    #[test]
    fn an_empty_list_is_empty_rather_than_one_blank() {
        assert_eq!(split_list(""), Vec::<String>::new());
        assert_eq!(split_list("Lifegem"), vec!["Lifegem".to_owned()]);
        assert_eq!(split_list("a;b"), vec!["a".to_owned(), "b".to_owned()]);
    }

    /// A brace inside a quoted name does not end the object early.
    #[test]
    fn a_brace_in_a_name_does_not_close_the_literal() {
        let page = "<body><script>savedBuild={class_:'sword}sman',gender:0};</script></body>";
        assert_eq!(parse(page, 1), Err(ParseError::MissingField("covenant")));
    }

    /// The nine stats are listed once each, in the planner's order.
    #[test]
    fn every_stat_is_named_exactly_once() {
        let stats = Stats::default();
        let names: Vec<&str> = stats.each().iter().map(|(name, _)| *name).collect();
        assert_eq!(names.len(), 9);
        for (index, name) in names.iter().enumerate() {
            assert!(!names[index + 1..].contains(name), "{name} listed twice");
        }
    }
}
