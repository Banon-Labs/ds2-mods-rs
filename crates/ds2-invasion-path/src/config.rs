//! `[invasion_path]` in `<Game>/ds2-mods.toml`, and the rule that every setting here is live.
//!
//! Change the key, save the file, press the new key -- no restart. The file is re-read about once
//! a second and what changed is logged, which is how a player finds out their edit landed without
//! having to guess from the absence of an effect.
//!
//! # Why the file is compared by CONTENT and not by timestamp
//!
//! `ds2_hotkey_config::reload::HotFile` does the comparing, and it compares the text. `mtime` has
//! one-second granularity on several filesystems -- a Proton prefix among them -- so an edit saved
//! in the same second as the previous read is invisible to a timestamp check. That is not a rare
//! race: it is what happens every time someone tweaks a number and immediately tweaks it back.
//!
//! # Only settings that do something are here
//!
//! `er-invasion-path`'s file carries `marker_fxr_id`, `marker_spacing_meters`, `max_markers`,
//! `search_range_meters` and `search_budget`. None of them are here, because nothing in this
//! crate spawns an effect or asks for a navmesh search yet -- see `crate::navpath`. A setting
//! that is read, validated, logged and then ignored is worse than a missing one: it is a
//! documented promise the code does not keep.

// Parsed on Windows; the parser and its defaults are proven on the host.
#![cfg_attr(not(windows), allow(dead_code))]

use ds2_hotkey_config::keys::Chord;
use ds2_hotkey_config::kv::KeyValues;
use ds2_hotkey_config::parse_chord;

/// The section this crate reads. Mirrored in the loader.
pub const CONFIG_SECTION: &str = "invasion_path";

/// The key that switches the overlay on and off, by name.
///
/// `;` rather than a function key, and that default was bought with a live failure in the sibling
/// workspace: `er-invasion-path` shipped `F7`, a 15-DLL run found `er-invasion-warp` polling
/// `VK_F7` every frame in the same process, and the key warped the player instead of drawing
/// anything, with nothing warning about it. "The game binds nothing to it" is not the question --
/// the other mods loaded beside you are, and a default cannot know them. `;` is clear of
/// everything this workspace polls, which makes it a better default rather than a safe one.
pub const DEFAULT_TOGGLE_KEY_NAME: &str = "semicolon";

/// Closer than this, nothing is drawn, in metres.
///
/// **This number is ELDEN RING's**, and saying so is the point. There it is the game's own:
/// `MenuCommonParam` row 0's `compassEnemyHostInnerDistance` is `30.0`, the radius inside which
/// the game hides an invader's marker for the host. DARK SOULS II has no compass and therefore no
/// such param, so there is no DS2 measurement to take. Thirty metres is carried over as a
/// defensible starting point for a value a player can change, and it is recorded here as borrowed
/// rather than derived so nobody later cites it as a DS2 fact.
///
/// Unlike that compass, this compares in **3D**. A player five metres away and forty metres
/// straight down is five metres to a map-plane test, and losing the arrow there is losing it
/// exactly where the climb is the thing worth pointing at.
pub const DEFAULT_NEAR_SUPPRESS_METERS: f32 = 30.0;

/// At or inside this, full width and opacity.
pub const DEFAULT_BOLD_AT_METERS: f32 = crate::geometry::DEFAULT_BOLD_AT_METERS;

/// At this distance, the faintest a line is drawn.
pub const DEFAULT_FAINT_AT_METERS: f32 = crate::geometry::DEFAULT_FAINT_AT_METERS;

/// Most players drawn at once.
///
/// DARK SOULS II's own cap is lower than Elden Ring's -- a session holds the host, up to three
/// summons and up to two invaders -- so six covers every one of them with nothing left over, and
/// the setting exists for the bloodstain phantoms that share the player class.
pub const DEFAULT_MAX_TARGETS: usize = 6;

/// Length of the arrow, in metres.
pub const DEFAULT_ARROW_METERS: f32 = 3.0;

/// Begin with the overlay already on.
pub const DEFAULT_START_ENABLED: bool = false;

/// `[invasion_path]`, resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct PathConfig {
    /// The chord that toggles the overlay.
    pub toggle: Chord,
    /// What the file asked for, for the log line. Not re-derived from [`Self::toggle`], because
    /// the interesting case is a name that did NOT parse and the log has to be able to say so.
    pub toggle_text: String,
    /// True when [`Self::toggle_text`] could not be parsed and the built-in default is standing
    /// in. A typo must never leave the feature unbindable, and it must never do so silently.
    pub toggle_fell_back: bool,
    pub near_suppress_meters: f32,
    pub bold_at_meters: f32,
    pub faint_at_meters: f32,
    pub max_targets: usize,
    pub arrow_meters: f32,
    pub start_enabled: bool,
}

impl Default for PathConfig {
    fn default() -> Self {
        let toggle = parse_chord(DEFAULT_TOGGLE_KEY_NAME).unwrap_or(Chord {
            // Unreachable unless `ds2-hotkey-config`'s table loses the name, which would be a
            // bug there rather than in the file -- but a default that can fail to exist is a
            // feature that can ship unbindable, so it is spelled out instead.
            modifiers: 0,
            // `VK_OEM_1` is `;` on a US layout. Reached only if the key table ever loses the
            // name, which would be a bug in `ds2-hotkey-config` rather than in the file.
            vk: 0xBA,
            dik: None,
        });
        Self {
            toggle,
            toggle_text: DEFAULT_TOGGLE_KEY_NAME.to_string(),
            toggle_fell_back: false,
            near_suppress_meters: DEFAULT_NEAR_SUPPRESS_METERS,
            bold_at_meters: DEFAULT_BOLD_AT_METERS,
            faint_at_meters: DEFAULT_FAINT_AT_METERS,
            max_targets: DEFAULT_MAX_TARGETS,
            arrow_meters: DEFAULT_ARROW_METERS,
            start_enabled: DEFAULT_START_ENABLED,
        }
    }
}

/// Strip a trailing `# comment` and the quotes a TOML string carries.
///
/// The reader underneath is a `key = value` parser, not a TOML implementation, so `"F9"` arrives
/// with its quotes on. Accepting both spellings costs three lines and removes a class of
/// "I wrote it exactly as the comment showed and it did not work".
fn scalar(raw: &str) -> &str {
    let text = raw.split('#').next().unwrap_or(raw).trim();
    text.strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(text)
        .trim()
}

/// A strictly positive, finite float, or `default`.
///
/// Zero is rejected along with the negatives on purpose: every distance this reads is a threshold
/// or a length, and zero for any of them is a setting that silently disables the thing it
/// configures. Someone who wants the overlay off has a switch for that.
fn positive_float(values: &KeyValues, key: &str, default: f32) -> f32 {
    values
        .get(CONFIG_SECTION, key)
        .map(scalar)
        .and_then(|text| text.parse::<f32>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(default)
}

/// A distance that is allowed to be zero, because zero means "never suppress".
fn non_negative_float(values: &KeyValues, key: &str, default: f32) -> f32 {
    values
        .get(CONFIG_SECTION, key)
        .map(scalar)
        .and_then(|text| text.parse::<f32>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(default)
}

impl PathConfig {
    /// Parse the section out of a whole `ds2-mods.toml`.
    ///
    /// Every unreadable value falls back to its default rather than failing the parse. A config
    /// file is edited by hand while a game is running; refusing the whole section over one typo
    /// would turn a mistyped number into a feature that vanished.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let values = KeyValues::parse(text);
        let defaults = Self::default();

        let requested = values
            .get(CONFIG_SECTION, "toggle_key")
            .map(scalar)
            .filter(|text| !text.is_empty());
        let (toggle, toggle_text, toggle_fell_back) = match requested {
            Some(name) => match parse_chord(name) {
                Ok(chord) => (chord, name.to_string(), false),
                Err(_) => (defaults.toggle, name.to_string(), true),
            },
            None => (defaults.toggle, defaults.toggle_text.clone(), false),
        };

        // Read before the pair is repaired, so the repair below sees what the file asked for.
        let bold_at = positive_float(&values, "bold_at_meters", defaults.bold_at_meters);
        let faint_at = positive_float(&values, "faint_at_meters", defaults.faint_at_meters);
        // A `faint_at` at or below `bold_at` collapses the ramp. `geometry::boldness` already
        // refuses to produce NaN from it, but the resulting overlay is every line at its faintest
        // regardless of distance, which reads as "the fade is broken" rather than as a bad
        // config. Swapping in the defaults keeps the ramp meaningful and the log says nothing,
        // because the pair is a single setting and one half of it being larger is not an error a
        // player can act on separately.
        let (bold_at, faint_at) = if faint_at > bold_at {
            (bold_at, faint_at)
        } else {
            (defaults.bold_at_meters, defaults.faint_at_meters)
        };

        Self {
            toggle,
            toggle_text,
            toggle_fell_back,
            near_suppress_meters: non_negative_float(
                &values,
                "near_suppress_meters",
                defaults.near_suppress_meters,
            ),
            bold_at_meters: bold_at,
            faint_at_meters: faint_at,
            max_targets: values
                .get(CONFIG_SECTION, "max_targets")
                .map(scalar)
                .and_then(|text| text.parse::<usize>().ok())
                .filter(|value| *value > 0 && *value <= HARD_TARGET_CAP)
                .unwrap_or(defaults.max_targets),
            arrow_meters: positive_float(&values, "arrow_meters", defaults.arrow_meters),
            start_enabled: values
                .get(CONFIG_SECTION, "start_enabled")
                .map(scalar)
                .map(|text| text.eq_ignore_ascii_case("true"))
                .unwrap_or(defaults.start_enabled),
        }
    }
}

/// The largest `max_targets` accepted from a file.
///
/// Not a taste judgement. Every target costs a roster scan entry and a handful of projected
/// segments on the render thread, and the roster is walked while the game is waiting. A file
/// asking for thousands would be a frame-time problem nobody could diagnose from the picture.
pub const HARD_TARGET_CAP: usize = 32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_is_the_defaults() {
        assert_eq!(PathConfig::parse(""), PathConfig::default());
    }

    #[test]
    fn a_named_key_is_honoured() {
        let parsed = PathConfig::parse("[invasion_path]\ntoggle_key = \"F9\"\n");
        assert_eq!(parsed.toggle_text, "F9");
        assert!(!parsed.toggle_fell_back);
        assert_ne!(
            parsed.toggle,
            PathConfig::default().toggle,
            "F9 resolved to the same chord as the default"
        );
    }

    #[test]
    fn an_unparseable_key_falls_back_and_says_so() {
        let parsed = PathConfig::parse("[invasion_path]\ntoggle_key = \"Fnord\"\n");
        assert_eq!(
            parsed.toggle,
            PathConfig::default().toggle,
            "a typo left the feature unbindable"
        );
        assert!(parsed.toggle_fell_back);
        assert_eq!(parsed.toggle_text, "Fnord", "the log cannot name the typo");
    }

    #[test]
    fn a_quoted_and_an_unquoted_value_parse_the_same() {
        let quoted = PathConfig::parse("[invasion_path]\narrow_meters = \"4.5\"\n");
        let bare = PathConfig::parse("[invasion_path]\narrow_meters = 4.5\n");
        assert!((quoted.arrow_meters - 4.5).abs() < 1.0e-6);
        assert_eq!(quoted.arrow_meters, bare.arrow_meters);
    }

    #[test]
    fn a_trailing_comment_is_not_part_of_the_value() {
        let parsed = PathConfig::parse("[invasion_path]\nmax_targets = 3 # up to three\n");
        assert_eq!(parsed.max_targets, 3);
    }

    #[test]
    fn a_nonsense_number_keeps_the_default_rather_than_failing_the_section() {
        let parsed = PathConfig::parse(
            "[invasion_path]\narrow_meters = banana\nmax_targets = 4\nstart_enabled = true\n",
        );
        assert_eq!(parsed.arrow_meters, DEFAULT_ARROW_METERS);
        assert_eq!(parsed.max_targets, 4, "one bad key discarded the others");
        assert!(parsed.start_enabled);
    }

    #[test]
    fn a_zero_length_arrow_is_refused() {
        let parsed = PathConfig::parse("[invasion_path]\narrow_meters = 0\n");
        assert_eq!(
            parsed.arrow_meters, DEFAULT_ARROW_METERS,
            "zero would draw nothing while looking configured"
        );
    }

    #[test]
    fn suppression_may_be_switched_off_with_zero() {
        let parsed = PathConfig::parse("[invasion_path]\nnear_suppress_meters = 0\n");
        assert_eq!(parsed.near_suppress_meters, 0.0);
    }

    #[test]
    fn an_inverted_fade_pair_is_repaired_to_the_defaults() {
        let parsed =
            PathConfig::parse("[invasion_path]\nbold_at_meters = 150\nfaint_at_meters = 20\n");
        assert_eq!(parsed.bold_at_meters, DEFAULT_BOLD_AT_METERS);
        assert_eq!(parsed.faint_at_meters, DEFAULT_FAINT_AT_METERS);
    }

    #[test]
    fn a_valid_fade_pair_survives() {
        let parsed =
            PathConfig::parse("[invasion_path]\nbold_at_meters = 5\nfaint_at_meters = 60\n");
        assert!((parsed.bold_at_meters - 5.0).abs() < 1.0e-6);
        assert!((parsed.faint_at_meters - 60.0).abs() < 1.0e-6);
    }

    #[test]
    fn an_absurd_target_count_is_capped_back_to_the_default() {
        let parsed = PathConfig::parse("[invasion_path]\nmax_targets = 100000\n");
        assert_eq!(parsed.max_targets, DEFAULT_MAX_TARGETS);
    }

    #[test]
    fn another_features_section_is_not_read_as_ours() {
        let parsed = PathConfig::parse(
            "[inventory_sort]\nmax_targets = 1\narrow_meters = 99\n\n[invasion_path]\nmax_targets = 2\n",
        );
        assert_eq!(parsed.max_targets, 2);
        assert_eq!(parsed.arrow_meters, DEFAULT_ARROW_METERS);
    }
}
