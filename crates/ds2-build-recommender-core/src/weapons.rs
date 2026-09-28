//! The weapon table the panel's two dropdowns and its class filter are drawn from.
//!
//! `data/weapons.tsv` is written by `scripts/ds2-recommender-weapons.py` from soulsplanner's and
//! mugenmonkey's own tables and compiled in whole. One row per weapon: the soulsplanner key (what a
//! build names and what `ds2_build_import_core::id_for` resolves), the display name, the class, the
//! kind, and one letter per infusion the weapon takes. The header line carries the letter legend.

use std::sync::OnceLock;

use ds2_build_import_core::Infusion;

/// The table, as the script wrote it.
pub const TABLE: &str = include_str!("../data/weapons.tsv");

/// What a row is, which decides whether the ranking considers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeaponKind {
    /// Something that is swung, thrust or shot. Ranked.
    Weapon,
    /// A shield. Listed so a build can name one, never ranked.
    Shield,
    /// A staff, chime, flame or other catalyst. Listed, never ranked.
    Catalyst,
}

/// One weapon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeaponRow {
    /// The soulsplanner key, `Moonlight_Greatsword`.
    pub key: &'static str,
    /// The display name, `Moonlight Greatsword`.
    pub name: &'static str,
    /// mugenmonkey's class, `Greatsword`; empty where mugenmonkey has no row for the weapon.
    pub class: &'static str,
    /// Weapon, shield or catalyst.
    pub kind: WeaponKind,
    /// One letter per infusion the weapon takes, in the table's order. See [`infusion_for_code`].
    pub infusions: &'static str,
}

impl WeaponRow {
    /// The infusions this weapon takes, in the table's order. Uninfused is always first.
    pub fn infusions(&self) -> Vec<Infusion> {
        self.infusions
            .chars()
            .filter_map(infusion_for_code)
            .collect()
    }

    /// Whether `infusion` is one this weapon takes.
    pub fn takes(&self, infusion: Infusion) -> bool {
        self.infusions().contains(&infusion)
    }
}

/// The infusion one letter of the table's `infusions` column stands for.
pub const fn infusion_for_code(code: char) -> Option<Infusion> {
    Some(match code {
        'N' => Infusion::None,
        'M' => Infusion::Magic,
        'F' => Infusion::Fire,
        'L' => Infusion::Lightning,
        'D' => Infusion::Dark,
        'P' => Infusion::Poison,
        'B' => Infusion::Bleed,
        'R' => Infusion::Raw,
        'E' => Infusion::Enchanted,
        'U' => Infusion::Mundane,
        _ => return None,
    })
}

/// What soulsplanner calls an infusion, which is what a build's weapon list carries after the name.
pub const fn planner_name(infusion: Infusion) -> &'static str {
    match infusion {
        Infusion::None => "No_Infusion",
        Infusion::Fire => "Fire",
        Infusion::Magic => "Magic",
        Infusion::Lightning => "Lightning",
        Infusion::Dark => "Dark",
        Infusion::Poison => "Poison",
        Infusion::Bleed => "Bleed",
        Infusion::Raw => "Raw",
        Infusion::Enchanted => "Enchanted",
        Infusion::Mundane => "Mundane",
    }
}

/// What the panel prints for an infusion.
pub const fn display_name(infusion: Infusion) -> &'static str {
    match infusion {
        Infusion::None => "Uninfused",
        other => planner_name(other),
    }
}

/// Every row, parsed once.
///
/// # Panics
///
/// Never on the shipped table: a malformed row is skipped, not fatal, and
/// `the_table_parses_whole` pins that no shipped row is.
pub fn all() -> &'static [WeaponRow] {
    static ROWS: OnceLock<Vec<WeaponRow>> = OnceLock::new();
    ROWS.get_or_init(|| parse(TABLE))
}

/// Parse a table in the script's format. Malformed rows and the header are skipped.
pub fn parse(text: &'static str) -> Vec<WeaponRow> {
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .filter_map(|line| {
            let mut columns = line.split('\t');
            let key = columns.next()?;
            let name = columns.next()?;
            let class = columns.next()?;
            let kind = match columns.next()? {
                "W" => WeaponKind::Weapon,
                "S" => WeaponKind::Shield,
                "C" => WeaponKind::Catalyst,
                _ => return None,
            };
            let infusions = columns.next()?;
            (!key.is_empty() && !name.is_empty()).then_some(WeaponRow {
                key,
                name,
                class,
                kind,
                infusions,
            })
        })
        .collect()
}

/// The row a soulsplanner key names.
pub fn by_key(key: &str) -> Option<&'static WeaponRow> {
    all().iter().find(|row| row.key == key)
}

/// The infusions the weapon `key` names takes; empty for a key the table does not have.
pub fn infusions_for(key: &str) -> Vec<Infusion> {
    by_key(key).map(WeaponRow::infusions).unwrap_or_default()
}

/// Letters and digits only, lowercased, so `butcher's` finds `Butcher's Knife`.
fn fold(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The rows whose name or class contains every word of `filter`, in table order.
///
/// An empty filter matches everything. Case, spaces and punctuation are ignored within a word, and
/// the words may appear in any order: `sword moon` finds the Moonlight Greatsword.
pub fn search(filter: &str) -> Vec<&'static WeaponRow> {
    let words: Vec<String> = filter
        .split_whitespace()
        .map(fold)
        .filter(|word| !word.is_empty())
        .collect();
    all()
        .iter()
        .filter(|row| {
            let haystack = format!("{}{}", fold(row.name), fold(row.class));
            words.iter().all(|word| haystack.contains(word.as_str()))
        })
        .collect()
}

/// Every weapon class the ranked kind carries, sorted, for the class filter. Empty classes are
/// left out.
pub fn weapon_classes() -> Vec<&'static str> {
    let mut classes: Vec<&'static str> = all()
        .iter()
        .filter(|row| row.kind == WeaponKind::Weapon && !row.class.is_empty())
        .map(|row| row.class)
        .collect();
    classes.sort_unstable();
    classes.dedup();
    classes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every line after the header is a row: nothing in the shipped table is silently dropped.
    #[test]
    fn the_table_parses_whole() {
        let lines = TABLE
            .lines()
            .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
            .count();
        assert_eq!(all().len(), lines);
        assert!(lines > 300, "{lines} rows -- the table is truncated");
    }

    /// Every infusion letter in the table is one the legend defines.
    #[test]
    fn every_infusion_letter_is_known() {
        for row in all() {
            assert_eq!(
                row.infusions().len(),
                row.infusions.chars().count(),
                "{} carries a letter with no infusion: {}",
                row.key,
                row.infusions
            );
        }
    }

    #[test]
    fn the_moonlight_greatsword_takes_every_infusion() {
        let row = by_key("Moonlight_Greatsword").expect("in the table");
        assert_eq!(row.name, "Moonlight Greatsword");
        assert_eq!(row.class, "Greatsword");
        assert_eq!(row.kind, WeaponKind::Weapon);
        assert_eq!(infusions_for("Moonlight_Greatsword").len(), 10);
        assert_eq!(infusions_for("Moonlight_Greatsword")[0], Infusion::None);
    }

    /// The infusion filter: a weapon offers exactly the letters its row carries, and a key the
    /// table does not have offers none.
    #[test]
    fn the_infusion_filter_follows_the_row() {
        let limited = all()
            .iter()
            .find(|row| row.infusions.len() < 10)
            .expect("some weapon takes fewer than every infusion");
        let offered = infusions_for(limited.key);
        assert_eq!(offered.len(), limited.infusions.len());
        let refused = [
            Infusion::None,
            Infusion::Magic,
            Infusion::Fire,
            Infusion::Lightning,
            Infusion::Dark,
            Infusion::Poison,
            Infusion::Bleed,
            Infusion::Raw,
            Infusion::Enchanted,
            Infusion::Mundane,
        ]
        .into_iter()
        .find(|infusion| !offered.contains(infusion))
        .expect("a missing infusion");
        assert!(!limited.takes(refused));
        assert!(infusions_for("Not_A_Weapon").is_empty());
    }

    #[test]
    fn a_planner_name_round_trips_through_the_import_crate() {
        for code in "NMFLDPBREU".chars() {
            let infusion = infusion_for_code(code).expect("a legend letter");
            assert_eq!(Infusion::from_name(planner_name(infusion)), Some(infusion));
        }
    }

    #[test]
    fn search_ignores_case_order_and_punctuation() {
        let found = search("sword MOON");
        assert!(found.iter().any(|row| row.key == "Moonlight_Greatsword"));
        assert!(
            search("butchers")
                .iter()
                .any(|row| row.name == "Butcher's Knife")
        );
        assert_eq!(search("").len(), all().len());
        assert!(search("zzzz-no-such-weapon").is_empty());
    }

    #[test]
    fn the_classes_are_the_ranked_kinds_only() {
        let classes = weapon_classes();
        assert!(classes.contains(&"Greatsword"));
        assert!(!classes.contains(&"Normal Shield"));
        assert!(classes.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
