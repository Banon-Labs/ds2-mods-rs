//! Resize and centre the HP bar and damage number the game floats over other characters.
//!
//! Off unless `<Game>/ds2-mods.toml` says `[hp_gauge] enabled = true`.
//!
//! `FeSceneEnemyHpGuage` draws up to twenty gauges a frame, one per visible character, and places
//! each with two calls that are handed the same scale (render height / 720): one for the bar, one
//! for the number. Both are detoured here, so the two are sized independently:
//!
//! | detour | what it changes |
//! |---|---|
//! | bar transform | moves the pivot by `bar_dx`/`bar_dy` and multiplies the scale by `bar_scale` |
//! | text transform | centres the number over the bar, lifts it by `text_lift`, multiplies the scale by `text_scale` |
//! | number setter | only counts the digits the game writes, so the number can be centred as a block |
//!
//! # Why the number needs a second write
//!
//! The game's own text call submits its matrix with the translate-only flag, so the element stores
//! the scaled matrix and then applies only its translation -- a scale argument of 50 draws the
//! number at its usual size. After the original returns, this writes the full-matrix mask and
//! clears that flag on the element, which is the state the bar's own submit leaves. Nothing is
//! written unless the element is the class those offsets were read from and already holds a
//! matrix.
//!
//! # Centring
//!
//! The number's pivot is the left edge of its text, and the bar's is its left edge too. So the
//! number is placed at the bar's midpoint minus half its own width, and its width is its digit
//! count times `digit_advance` layout units. That advance is not in the layout file; the default
//! was set by eye in a live run.
//!
//! `docs/DS2-HP-GAUGE.md` has the call chain, the layout records and the tuning runs.

// DEBT: ds2-mods-rs-24r -- not debt to be paid: this crate ships as a Windows DLL and the
// attribute is what keeps its Rust half parseable on the host, so the game-free tests below it
// can run at all. The issue is the standing record of that decision.
#![cfg_attr(not(windows), allow(unused))]

#[cfg(windows)]
mod install;

#[cfg(windows)]
pub use install::{LogFn, Outcome, install, set_logger, set_tune};

/// Prefix on every line this crate writes to the loader log.
pub const LOG_PREFIX: &str = "ds2-hp-gauge:";

/// Width of the bar frame in layout units. Mirrors `ds2_rva::HP_GAUGE_BAR_WIDTH`, which the host
/// build cannot reach; a test on Windows holds the two equal.
pub const BAR_WIDTH: f32 = 99.6;

/// The number the game clamps to. Mirrors `ds2_rva::HP_GAUGE_NUMBER_MAX`.
pub const NUMBER_MAX: i32 = 99_999;

/// Every tunable, in 720p layout units where it is a distance.
///
/// Distances are multiplied by the game's own height / 720 scale, so they are the same share of
/// the screen at any resolution. Screen y grows downward.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tune {
    /// Multiplier on the game's scale for the damage number.
    pub text_scale: f32,
    /// How far to raise the number above where the game puts it.
    pub text_lift: f32,
    /// Horizontal nudge for the number after centring, negative = left.
    pub text_dx: f32,
    /// Width of one digit of the number, before any scale.
    pub digit_advance: f32,
    /// Multiplier on the game's scale for the bar.
    pub bar_scale: f32,
    /// How far to move the bar's pivot right (negative = left).
    pub bar_dx: f32,
    /// How far to move the bar's pivot down.
    pub bar_dy: f32,
}

impl Default for Tune {
    /// The values the live run on 2026-09-25 ended on, when the user said it looked good.
    ///
    /// `text_scale` 1.25 on the game's 2.0 at 1440p is 2.5x what is normally drawn, because the
    /// normal draw ignores the scale entirely (see the crate docs). `bar_dx` -98.4 is where half
    /// the grown bar's width, and the corrections after it, landed the bar over the target.
    fn default() -> Self {
        Self {
            text_scale: 1.25,
            text_lift: 12.0,
            text_dx: 0.0,
            digit_advance: 8.0,
            bar_scale: 2.25,
            bar_dx: -98.4,
            bar_dy: 6.0,
        }
    }
}

/// How many characters the game writes for `value`: none below 1, otherwise its decimal digits
/// after the game's own clamp.
pub fn digit_count(value: i32) -> u8 {
    if value < 1 {
        return 0;
    }
    let mut value = value.min(NUMBER_MAX);
    let mut digits = 0;
    while value > 0 {
        digits += 1;
        value /= 10;
    }
    digits
}

/// Where the bar's pivot goes: the game's pivot moved by the tuned offset.
pub fn bar_pivot(game: [f32; 2], scale: f32, tune: &Tune) -> [f32; 2] {
    [game[0] + tune.bar_dx * scale, game[1] + tune.bar_dy * scale]
}

/// The x of the bar's centre, given its (moved) pivot. The frame is left-anchored at the pivot
/// and scales about it.
pub fn bar_centre_x(pivot_x: f32, scale: f32, tune: &Tune) -> f32 {
    pivot_x + BAR_WIDTH / 2.0 * scale * tune.bar_scale
}

/// Where the number's pivot goes.
///
/// With the bar's centre known, the number is centred on it by its width; without it (a text
/// call with no bar before it this frame) the game's x is kept. Either way it is lifted.
pub fn text_pivot(
    game: [f32; 2],
    bar_centre: Option<f32>,
    digits: u8,
    scale: f32,
    tune: &Tune,
) -> [f32; 2] {
    let applied = scale * tune.text_scale;
    let x = match bar_centre {
        Some(centre) => centre - f32::from(digits) * tune.digit_advance * applied / 2.0,
        None => game[0],
    };
    [x + tune.text_dx * scale, game[1] - tune.text_lift * scale]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_follow_the_games_formatting() {
        assert_eq!(digit_count(0), 0);
        assert_eq!(digit_count(-5), 0);
        assert_eq!(digit_count(7), 1);
        assert_eq!(digit_count(10), 2);
        assert_eq!(digit_count(1234), 4);
        assert_eq!(digit_count(99_999), 5);
        assert_eq!(
            digit_count(i32::MAX),
            5,
            "the game clamps before formatting"
        );
    }

    #[test]
    fn the_bar_centre_is_half_its_grown_width_right_of_the_pivot() {
        let tune = Tune::default();
        let centre = bar_centre_x(100.0, 2.0, &tune);
        assert!((centre - (100.0 + 49.8 * 2.0 * 2.25)).abs() < 1e-3);
    }

    #[test]
    fn a_number_is_centred_by_its_own_width() {
        let tune = Tune {
            text_dx: 0.0,
            ..Tune::default()
        };
        let one = text_pivot([0.0, 0.0], Some(500.0), 1, 2.0, &tune)[0];
        let three = text_pivot([0.0, 0.0], Some(500.0), 3, 2.0, &tune)[0];
        // Each extra digit moves the left edge half a digit further left.
        let per_digit = tune.digit_advance * 2.0 * tune.text_scale;
        assert!((one - (500.0 - per_digit / 2.0)).abs() < 1e-3);
        assert!((one - three - per_digit).abs() < 1e-3);
    }

    #[test]
    fn without_a_bar_the_games_x_is_kept_and_the_number_still_lifts() {
        let tune = Tune::default();
        let pivot = text_pivot([321.0, 200.0], None, 3, 2.0, &tune);
        assert_eq!(pivot[0], 321.0 + tune.text_dx * 2.0);
        assert_eq!(pivot[1], 200.0 - tune.text_lift * 2.0);
    }

    #[test]
    fn the_bar_moves_by_scaled_offsets() {
        let tune = Tune::default();
        let pivot = bar_pivot([10.0, 20.0], 2.0, &tune);
        assert_eq!(pivot, [10.0 + tune.bar_dx * 2.0, 20.0 + tune.bar_dy * 2.0]);
    }
}
