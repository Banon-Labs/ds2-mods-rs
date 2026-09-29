//! Weapon flexibility: how many weapons a stat line wields, and how that compares with the builds
//! nearest it.
//!
//! The script's `flexibility`, `flex_line` and `flex_load_line`
//! (`scripts/ds2-builds-recommend.py`). [`crate::corpus::CorpusBackend`] computes a
//! [`Flexibility`]; this module holds what it is and the words the panel shows for it, so the panel
//! and the script say the same thing.

/// How many nearest-stat corpus builds a build is ranked among: the script's `FLEX_K`.
pub const FLEX_K: usize = 50;

/// The share of equip load the load line is measured under, as a percentage: the script's
/// `EQUIP_CAP`.
pub const LOAD_CAP_PERCENT: u32 = 70;

/// Flynn's Ring's physical bonus falls as equip load rises. The curve is not modelled, so the spare
/// load is shown and this is said beside it: the script's `FLYNN_NOTE`.
pub const FLYNN_NOTE: &str = "Flynn's Ring's bonus falls as load rises";

/// A stat line's weapon flexibility.
#[derive(Clone, Debug, PartialEq)]
pub struct Flexibility {
    /// Weapons the levelled stats wield one-handed.
    pub one_handed: u32,
    /// Weapons they wield two-handed (STR requirement halved); includes every one-handed one.
    pub two_handed: u32,
    /// Every weapon a hand can hold, `Bare_Fists` left out.
    pub total: u32,
    /// How many of the nearest builds wield fewer (one-handed plus two-handed).
    pub below: u32,
    /// How many wield the same number.
    pub equal: u32,
    /// How many nearest builds there were, at most [`FLEX_K`].
    pub neighbours: u32,
    /// `100 * (below + equal / 2) / neighbours`: the mid-rank percentile.
    pub percentile: f64,
    /// Equip load left under [`LOAD_CAP_PERCENT`] after the armour and rings: the weapon weight
    /// the build can still carry.
    pub spare_load: f64,
    /// Weapons wielded in either grip that weigh no more than [`Self::spare_load`] on their own.
    pub fits: u32,
}

/// `wields 28/324 1H, 110 2H -- 25th percentile of 50 similar builds`.
pub fn flex_line(flex: &Flexibility) -> String {
    format!(
        "wields {}/{} 1H, {} 2H -- {} percentile of {} similar builds",
        flex.one_handed,
        flex.total,
        flex.two_handed,
        ordinal(round_half_even(flex.percentile)),
        flex.neighbours
    )
}

/// `34.3 weight left for weapons under 70% load, 110 fit -- Flynn's Ring's bonus falls as load
/// rises`.
pub fn flex_load_line(flex: &Flexibility) -> String {
    format!(
        "{:.1} weight left for weapons under {LOAD_CAP_PERCENT}% load, {} fit -- {FLYNN_NOTE}",
        flex.spare_load, flex.fits
    )
}

/// Python's `round`, which rounds a half to the even neighbour: 12.5 is 12 and 13.5 is 14. Read
/// back through its decimal text rather than cast, so no float-to-integer cast needs allowing.
fn round_half_even(value: f64) -> i64 {
    format!("{:.0}", value.round_ties_even())
        .parse()
        .unwrap_or(0)
}

/// 1st, 2nd, 3rd, 4th, 11th, 12th, 13th, 21st.
pub fn ordinal(n: i64) -> String {
    let suffix = if (10..=20).contains(&(n % 100)) {
        "th"
    } else {
        match n % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    };
    format!("{n}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Flexibility {
        Flexibility {
            one_handed: 28,
            two_handed: 110,
            total: 324,
            below: 12,
            equal: 1,
            neighbours: 50,
            percentile: 25.0,
            spare_load: 34.2825,
            fits: 110,
        }
    }

    #[test]
    fn the_line_is_the_scripts() {
        assert_eq!(
            flex_line(&sample()),
            "wields 28/324 1H, 110 2H -- 25th percentile of 50 similar builds"
        );
        assert_eq!(
            flex_load_line(&sample()),
            "34.3 weight left for weapons under 70% load, 110 fit -- Flynn's Ring's bonus falls \
             as load rises"
        );
    }

    #[test]
    fn ordinals_read_as_english() {
        let got: Vec<String> = [1, 2, 3, 4, 11, 12, 13, 21, 22, 100, 0]
            .into_iter()
            .map(ordinal)
            .collect();
        assert_eq!(
            got,
            [
                "1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd", "100th", "0th"
            ]
        );
    }

    #[test]
    fn a_half_rounds_to_even_as_python_does() {
        assert_eq!(round_half_even(12.5), 12);
        assert_eq!(round_half_even(13.5), 14);
        assert_eq!(round_half_even(13.4), 13);
        assert_eq!(round_half_even(99.0), 99);
    }
}
