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
//! # The marker settings
//!
//! `er-invasion-path`'s file carries `marker_fxr_id` and its spacing and budget siblings: they
//! place the game's OWN effect -- the lingering coloured stone -- along the route at intervals, so
//! the trail is made of real objects in the world rather than of lines drawn over it. The
//! equivalent keys are here, and `crate::trail` and `crate::sfx` act on them.
//!
//! **The item is the PRISM STONE here.** Elden Ring renamed it to Rainbow Stone; DARK SOULS II's
//! is `ItemParam` row `60450000`. Worth stating once, because searching this game for "rainbow"
//! finds nothing and reads as the feature being absent.
//!
//! Three things were in the way of this and none of them still is: the spawn is
//! `KatanaSfxSystem`'s at `0x140beb590`; it runs on the game's own tick (`crate::gametick`) rather
//! than in the `Present` detour, because its create path read-modify-writes a quality level, two
//! live vectors, an RNG and a red-black tree with no lock anywhere; and `crate::navquery` can now
//! ask for a route rather than only read one. `ds2-mods-rs-3al` and `ds2-mods-rs-4yd` carry the
//! derivations.
//!
//! **Still unseen on a screen.** Nobody has watched a stone appear. `ds2-mods-rs-zbo` lists the
//! log lines a live run should produce and what each missing one would rule out.
//!
//! A setting that is read, validated and then silently ignored is worse than a missing one -- it
//! is a promise the code does not keep. So these are not silent: [`PathConfig::markers_requested`]
//! is what the draw path asks, and asking for markers while they cannot be placed produces a log
//! line naming which of the two halves is missing. The setting does something observable from the
//! day it lands; what it does today is tell you the truth about why your trail is not there.

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

/// The effect placed at each marker along a route. `0` is off, and off is the default.
///
/// **`833` is the one to set**, and it is the Prism Stone's own: a seven-entry table at
/// `0x1410c7b58` holds `833..=839`, one colour each, reached from `ItemParam` row `60450000`
/// through an emevd instruction whose name is literally `七色石発射` -- "fire seven-colour stone".
/// Seven ids for seven colours is also the answer to a problem `er-invasion-path` could not solve:
/// it needed several different EFFECTS to tell players apart, because an Elden Ring FXR carries no
/// tint. Here the colours are the point of the item.
///
/// DARK SOULS II identifies an effect by a bare `int32` in the range 40..8557 -- no `SfxParam`, no
/// FXR -- so Elden Ring's six-digit `302022` is not merely a different number, it is a different
/// kind of number. Pass the BASE id: with the remaster's high-quality effects on, the engine tries
/// `id + 20000` itself.
///
/// The default is still `0`, because spawning anything is the only thing this crate does that
/// changes the game rather than drawing over it, and because no one has yet watched a stone appear
/// on a screen.
pub const PRISM_STONE_FIRST_EFFECT_ID: u32 = 833;

/// The Prism Stone's seven colours, `833..=839`. See [`PRISM_STONE_FIRST_EFFECT_ID`].
pub const PRISM_STONE_EFFECT_IDS: core::ops::RangeInclusive<u32> = 833..=839;

/// The effect placed at each marker along a route. `0` is off, and off is the default.
pub const DEFAULT_MARKER_EFFECT_ID: u32 = 0;

/// Metres between markers along the route.
///
/// Spaced along the PATH rather than placed at its corners, or a doorway collects six of them and
/// open ground gets none. Elden Ring's `2.7` is carried over as a starting point and recorded as
/// borrowed rather than measured -- DARK SOULS II's scale is close but nobody here has checked.
pub const DEFAULT_MARKER_SPACING_METERS: f32 = 2.7;

/// Most markers one route's trail may hold.
pub const DEFAULT_MAX_MARKERS: usize = 144;

/// Metres of already-walked trail kept behind you before those markers are torn down.
///
/// The ones you have passed are clutter, but tearing them down the instant you step past makes
/// the trail appear to end at your feet, which reads as the trail being broken.
pub const DEFAULT_MARKER_KEEP_BEHIND_METERS: f32 = 12.0;

/// Markers placed per pass, so a trail is laid outwards from your feet rather than all at once.
pub const DEFAULT_MARKERS_PER_PASS: usize = 3;

/// Route to the nearest NPC and narrate it. `false`, and off is the default.
///
/// # What it is for
///
/// **A solo player can never see this feature work.** Everything the route and the trail do
/// starts with another player in the session, so a solo session's log is all install lines --
/// "hooked", "requested", "armed" -- and not one execution line. A real session read
/// `characters=6 players=1 remotes=0`: six objects walked, one of them the player, and nothing
/// to point at. Waiting for a bloodstain phantom to wander past is not a test.
///
/// Those other five objects are the answer. An NPC is a live `CharacterCtrl` at a real world
/// position, standing on the navmesh, so routing to one runs the ENTIRE chain that a second
/// player would: snap both ends, request, poll, decode, space the stones along the path, spawn
/// each one. Turn this on and a solo player standing in Majula exercises every line of it.
///
/// # What the log will say, and why each line is there
///
/// Three failures look identical on the ground -- an effect that is not in this map, a spawn the
/// quality throttle discarded, and one that worked and is simply not where you are looking. The
/// self-check separates them at the moment of the attempt, because two of the three are invisible
/// afterwards:
///
/// | the log says | it means |
/// |---|---|
/// | `picked 0x..., N.Nm away` | which character, so a bad pick is visible rather than inferred |
/// | `READY -- N segment(s) decoded to M point(s)` | the route came back; both numbers, because they fail differently |
/// | `the planner said NO ROUTE` | a finding, not a shrug: that character IS on the navmesh |
/// | `id 833: spawned, quality 0, the id resolved` | it worked |
/// | `EMPTY -- ... AT OR ABOVE THE THRESHOLD` | [`ds2_rva::KATANA_SFX_QUALITY_DROP_THRESHOLD`] ate it; the id is not implicated |
/// | `THIS attempt's lookup failed` | the effect is not resident in this map |
/// | `t=3.0s -- 7/7 stone(s) still alive` | they LINGER, which is the property a trail needs |
///
/// It sweeps [`PRISM_STONE_SFX_IDS`] one id per stone, so a single run also says which of the
/// seven colours actually appear. Then it takes them down, through the same stand-down path the
/// real trail uses, so the check leaves nothing behind and the teardown gets tested too.
///
/// # Why it is off by default
///
/// It spawns effects and routes to a character you did not ask it to route to. That is the right
/// behaviour for a diagnostic and the wrong behaviour for a feature, and the distance between
/// those two is one line in a file.
pub const DEFAULT_NPC_SELF_CHECK: bool = false;

/// The most markers the parser will accept, whatever the file asks for.
///
/// Each one is an object the engine has to build, own and draw, inside a `Present` detour on the
/// render thread. A mistyped `max_markers = 1440000` must be a rejected setting rather than a
/// frozen game.
pub const HARD_MARKER_CAP: usize = 1024;

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
    /// The effect placed at each marker, or `0` for no markers. See [`DEFAULT_MARKER_EFFECT_ID`]
    /// for why no non-zero value is known yet.
    pub marker_effect_id: u32,
    pub marker_spacing_meters: f32,
    pub max_markers: usize,
    pub marker_keep_behind_meters: f32,
    pub markers_per_pass: usize,
    /// Route to the nearest NPC and report, in the log, everything that happened. See
    /// [`DEFAULT_NPC_SELF_CHECK`].
    pub npc_self_check: bool,
}

impl PathConfig {
    /// Has the file asked for a trail of markers?
    ///
    /// The one question the draw path asks about all five marker settings, so that "the player
    /// wants markers" is a single fact rather than five fields that have to be read together and
    /// could be read inconsistently. Nothing can place one yet -- see this module's header -- and
    /// the caller's job on `true` is to say WHY not, once, rather than to fail silently.
    #[must_use]
    pub fn markers_requested(&self) -> bool {
        self.marker_effect_id != 0 && self.max_markers > 0 && self.markers_per_pass > 0
    }
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
            marker_effect_id: DEFAULT_MARKER_EFFECT_ID,
            marker_spacing_meters: DEFAULT_MARKER_SPACING_METERS,
            max_markers: DEFAULT_MAX_MARKERS,
            marker_keep_behind_meters: DEFAULT_MARKER_KEEP_BEHIND_METERS,
            markers_per_pass: DEFAULT_MARKERS_PER_PASS,
            npc_self_check: DEFAULT_NPC_SELF_CHECK,
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
            // ZERO IS MEANINGFUL HERE, unlike every distance above: it is how the file spells
            // "no markers", and it is the default. So an unparseable id falls back to the
            // default rather than being filtered out as invalid -- the two are the same value
            // and both mean off.
            marker_effect_id: values
                .get(CONFIG_SECTION, "marker_effect_id")
                .map(scalar)
                .and_then(|text| text.parse::<u32>().ok())
                .unwrap_or(defaults.marker_effect_id),
            marker_spacing_meters: positive_float(
                &values,
                "marker_spacing_meters",
                defaults.marker_spacing_meters,
            ),
            max_markers: values
                .get(CONFIG_SECTION, "max_markers")
                .map(scalar)
                .and_then(|text| text.parse::<usize>().ok())
                .filter(|value| *value > 0 && *value <= HARD_MARKER_CAP)
                .unwrap_or(defaults.max_markers),
            marker_keep_behind_meters: non_negative_float(
                &values,
                "marker_keep_behind_meters",
                defaults.marker_keep_behind_meters,
            ),
            markers_per_pass: values
                .get(CONFIG_SECTION, "markers_per_pass")
                .map(scalar)
                .and_then(|text| text.parse::<usize>().ok())
                .filter(|value| *value > 0 && *value <= HARD_MARKER_CAP)
                .unwrap_or(defaults.markers_per_pass),
            npc_self_check: values
                .get(CONFIG_SECTION, "npc_self_check")
                .map(scalar)
                .map(|text| text.eq_ignore_ascii_case("true"))
                .unwrap_or(defaults.npc_self_check),
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

#[cfg(test)]
mod marker_scaffolding {
    use super::*;

    /// The shipped default must place nothing. Spawning an effect is the only thing this crate
    /// would do that changes the game rather than drawing over it, and no DARK SOULS II effect id
    /// is known -- so a non-zero default could only be a guess dressed as a setting.
    #[test]
    fn markers_are_off_until_someone_asks() {
        assert_eq!(PathConfig::default().marker_effect_id, 0);
        assert!(!PathConfig::default().markers_requested());
    }

    #[test]
    fn an_id_in_the_file_is_a_request() {
        let parsed = PathConfig::parse("[invasion_path]\nmarker_effect_id = 302022\n");
        assert_eq!(parsed.marker_effect_id, 302_022);
        assert!(parsed.markers_requested());
    }

    /// Zero is how the file spells "off", so it is a value rather than a rejected one.
    #[test]
    fn zero_is_off_rather_than_invalid() {
        let parsed = PathConfig::parse("[invasion_path]\nmarker_effect_id = 0\n");
        assert_eq!(parsed.marker_effect_id, 0);
        assert!(!parsed.markers_requested());
    }

    /// A budget of zero would be a request for markers that places none, which reads as the
    /// feature being broken rather than as the number being wrong.
    #[test]
    fn a_zero_budget_is_refused_rather_than_honoured() {
        let parsed = PathConfig::parse("[invasion_path]\nmarker_effect_id = 1\nmax_markers = 0\n");
        assert_eq!(parsed.max_markers, DEFAULT_MAX_MARKERS);
        assert!(parsed.markers_requested());
    }

    /// Every marker is an object the engine builds and draws from inside a `Present` detour, so a
    /// mistyped budget has to be a rejected setting and not a frozen game.
    #[test]
    fn an_absurd_budget_falls_back_instead_of_freezing_the_render_thread() {
        let parsed = PathConfig::parse("[invasion_path]\nmax_markers = 1440000\n");
        assert_eq!(parsed.max_markers, DEFAULT_MAX_MARKERS);
    }

    #[test]
    fn spacing_and_trail_length_come_through() {
        let parsed = PathConfig::parse(
            "[invasion_path]\nmarker_spacing_meters = 1.5\nmarker_keep_behind_meters = 0\n",
        );
        assert!((parsed.marker_spacing_meters - 1.5).abs() < f32::EPSILON);
        // Zero behind you is a legitimate choice -- tear them down as you pass -- unlike a zero
        // spacing, which would ask for infinitely many markers.
        assert!((parsed.marker_keep_behind_meters - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_zero_spacing_is_refused() {
        let parsed = PathConfig::parse("[invasion_path]\nmarker_spacing_meters = 0\n");
        assert!((parsed.marker_spacing_meters - DEFAULT_MARKER_SPACING_METERS).abs() < 0.001);
    }
}
