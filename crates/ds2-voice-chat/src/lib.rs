//! Flip DARK SOULS II's own Voice Chat option from a key named in `ds2-mods.toml`.
//!
//! # This adds no feature. It moves one that already exists onto a key.
//!
//! The Game tab of the options menu has a Voice chat row. It is byte `0x0b` of the options block
//! (`GameManagerImp -> +0xa8 -> +0xc8`, see [`ds2_rva`]'s `GAME_OPTION_VOICE_CHAT_OFFSET` on
//! Windows builds), `0` meaning on. This crate changes that byte the way the menu does and nothing
//! else: it copies the Game tab's sixteen bytes, inverts the one, and hands the copy to the tab's
//! own commit routine -- the function the menu calls when the player confirms. The game then does
//! the rest by itself: its net session update reads the byte every frame and, when it changes,
//! mutes or unmutes every voice peer in the session.
//!
//! # Where the key is read, and why there
//!
//! In a detour on that same net session update, before the original runs. So the press lands on
//! the game thread (the commit calls into the sound and input managers and is not safe from
//! anywhere else), and the original sees the new byte in the same frame the key went down.
//!
//! # What it does not do
//!
//! * Save. The byte is part of the options block the game writes with its own saves; this crate
//!   does not trigger one. A toggle outlives the session only if the game saves after it.
//! * Win against an open options menu. The menu edits a private copy and commits it when the
//!   player confirms, so a toggle made while the menu is open is overwritten by that confirm.
//!
//! # The binding moves without restarting the game
//!
//! House rule, same as `ds2-inventory-sort`: a watcher thread re-reads the config file about once a
//! second and publishes the chord into an atomic the detour loads. Default `F8`.
//!
//! # It says which way it went
//!
//! After a press the crate reads the byte back and plays a short spoken clip for the state the game
//! actually holds: "Voice chat on" / "Voice chat off", or the Polish pair. The clips are rendered
//! offline with espeak-ng as telephone-quality mono WAVs in `assets/` and compiled into the DLL, so
//! there is no speech engine and no file to go missing. `[voice_chat] announce` picks the language;
//! `""` silences it.

// DEBT: ds2-mods-rs-24r -- not debt to be paid: this crate ships as a Windows DLL and the
// attribute is what keeps its Rust half parseable on the host, so the game-free tests below it
// can run at all. The issue is the standing record of that decision.
#![cfg_attr(not(windows), allow(unused))]

#[cfg(windows)]
mod install;

#[cfg(windows)]
pub use install::{LogFn, Outcome, Request, install, set_logger};

use ds2_hotkey_config::keys::{Chord, KeyParseError, parse_chord};
use ds2_hotkey_config::kv::KeyValues;

/// Prefix on every line this crate writes to the loader log.
pub const LOG_PREFIX: &str = "ds2-voice-chat:";

/// The config section. Mirrored in `crates/ds2-loader/src/voice_chat.rs` and in the shipped
/// `.github/dist-ds2-mods.toml`.
pub const CONFIG_SECTION: &str = "voice_chat";

/// The keyboard binding key. A name from [`ds2_hotkey_config::keys`], e.g. `"F8"`, `"Ctrl+V"`.
pub const CONFIG_KEY_KEYBOARD: &str = "key";

/// The binding in force when the file does not name one. Clear of `F7` (`ds2-inventory-sort`) and
/// `;` (`ds2-invasion-path`), the two other keys this workspace polls by default.
pub const DEFAULT_KEY: &str = "F8";

/// The announcement language key. `"en"`, `"pl"`, or `""` for none.
pub const CONFIG_KEY_ANNOUNCE: &str = "announce";

/// The language spoken when the file does not name one.
pub const DEFAULT_ANNOUNCE: Announce = Announce::English;

/// Which clip pair a press plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Announce {
    /// No sound.
    Silent = 0,
    /// "Voice chat on" / "Voice chat off".
    English = 1,
    /// "O kurwa, działa!" / "O kurwa, nie działa!".
    Polish = 2,
}

impl Announce {
    /// The inverse of `as u8`, for the atomic the detour loads. Unknown values are silent.
    pub const fn from_u8(raw: u8) -> Self {
        match raw {
            1 => Self::English,
            2 => Self::Polish,
            _ => Self::Silent,
        }
    }

    /// The WAV to play for a voice chat state, or `None` when silent.
    pub const fn clip(self, on: bool) -> Option<&'static [u8]> {
        match (self, on) {
            (Self::Silent, _) => None,
            (Self::English, true) => Some(include_bytes!("../assets/en-on.wav")),
            (Self::English, false) => Some(include_bytes!("../assets/en-off.wav")),
            (Self::Polish, true) => Some(include_bytes!("../assets/pl-on.wav")),
            (Self::Polish, false) => Some(include_bytes!("../assets/pl-off.wav")),
        }
    }
}

/// What one config text says about the announcement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnnounceSetting {
    /// The key is not in the file. [`DEFAULT_ANNOUNCE`] stands.
    NotSet,
    /// A value this crate knows.
    Set(Announce),
    /// A value it does not. The language already in force stays in force.
    Invalid(String),
}

/// Read `[voice_chat] announce` out of a config text.
pub fn announce_setting(text: &str) -> AnnounceSetting {
    let parsed = KeyValues::parse(text);
    let Some(raw) = parsed.get(CONFIG_SECTION, CONFIG_KEY_ANNOUNCE) else {
        return AnnounceSetting::NotSet;
    };
    let value = raw.trim().trim_matches('"').trim();
    match value.to_ascii_lowercase().as_str() {
        "" => AnnounceSetting::Set(Announce::Silent),
        "en" => AnnounceSetting::Set(Announce::English),
        "pl" => AnnounceSetting::Set(Announce::Polish),
        _ => AnnounceSetting::Invalid(value.to_string()),
    }
}

/// What one config text says about the binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeySetting {
    /// The key is not in the file. The default stands.
    NotSet,
    /// `key = ""`: the player unbound it on purpose.
    Unbound,
    /// A key this crate can read.
    Bound(Chord),
    /// A value that does not parse. The binding already in force stays in force.
    Invalid {
        /// What the file said.
        value: String,
        /// Why it was refused.
        error: KeyParseError,
    },
}

/// Read `[voice_chat] key` out of a config text.
pub fn key_setting(text: &str) -> KeySetting {
    let parsed = KeyValues::parse(text);
    let Some(raw) = parsed.get(CONFIG_SECTION, CONFIG_KEY_KEYBOARD) else {
        return KeySetting::NotSet;
    };
    let value = raw.trim().trim_matches('"').trim();
    if value.is_empty() {
        return KeySetting::Unbound;
    }
    match parse_chord(value) {
        Ok(chord) => KeySetting::Bound(chord),
        Err(error) => KeySetting::Invalid {
            value: value.to_string(),
            error,
        },
    }
}

/// The chord [`DEFAULT_KEY`] names, or `None` if the key table ever stops knowing it -- which
/// would be a bug here, and leaves the feature unbound rather than bound to a guess.
pub fn default_chord() -> Option<Chord> {
    parse_chord(DEFAULT_KEY).ok()
}

/// The Voice chat byte after one press: `0` (on) becomes `1` (off), anything else becomes `0`.
///
/// The game's own readers test the byte against zero, so any nonzero value means off; a press on
/// an off value always turns voice chat on, and a press on an on value writes the `1` the menu
/// writes.
pub const fn toggled(voice_chat: u8) -> u8 {
    if voice_chat == 0 { 1 } else { 0 }
}

/// What the game will do with this byte: `true` when voice chat is on.
pub const fn voice_chat_on(voice_chat: u8) -> bool {
    voice_chat == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIPPED_CONFIG: &str = include_str!("../../../.github/dist-ds2-mods.toml");

    fn chord(name: &str) -> Chord {
        parse_chord(name).expect("a name the key table knows")
    }

    #[test]
    fn a_press_flips_on_to_off_and_off_to_on() {
        assert_eq!(toggled(0), 1);
        assert_eq!(toggled(1), 0);
        assert!(!voice_chat_on(toggled(0)));
        assert!(voice_chat_on(toggled(1)));
    }

    #[test]
    fn any_nonzero_byte_is_off_and_a_press_turns_it_on() {
        assert!(!voice_chat_on(2));
        assert_eq!(toggled(0xff), 0);
    }

    #[test]
    fn two_presses_put_the_menus_values_back() {
        assert_eq!(toggled(toggled(0)), 0);
        assert_eq!(toggled(toggled(1)), 1);
    }

    #[test]
    fn the_default_is_f8() {
        assert_eq!(default_chord(), Some(chord("F8")));
    }

    #[test]
    fn the_download_binds_f8() {
        assert_eq!(key_setting(SHIPPED_CONFIG), KeySetting::Bound(chord("F8")));
    }

    #[test]
    fn no_section_means_the_default_stands() {
        assert_eq!(key_setting(""), KeySetting::NotSet);
        assert_eq!(
            key_setting("[inventory_sort]\nkey = \"F9\"\n"),
            KeySetting::NotSet
        );
    }

    #[test]
    fn a_named_key_is_read_quoted_or_not() {
        let want = KeySetting::Bound(chord("F10"));
        assert_eq!(key_setting("[voice_chat]\nkey = \"F10\"\n"), want);
        assert_eq!(key_setting("[voice_chat]\nkey = F10\n"), want);
    }

    #[test]
    fn a_chord_with_a_modifier_is_read() {
        assert_eq!(
            key_setting("[voice_chat]\nkey = \"Ctrl+V\"\n"),
            KeySetting::Bound(chord("Ctrl+V"))
        );
    }

    #[test]
    fn an_empty_value_unbinds() {
        assert_eq!(
            key_setting("[voice_chat]\nkey = \"\"\n"),
            KeySetting::Unbound
        );
    }

    #[test]
    fn junk_is_refused_and_named() {
        match key_setting("[voice_chat]\nkey = \"NotAKey\"\n") {
            KeySetting::Invalid { value, .. } => assert_eq!(value, "NotAKey"),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn the_download_announces_in_english() {
        assert_eq!(
            announce_setting(SHIPPED_CONFIG),
            AnnounceSetting::Set(Announce::English)
        );
    }

    #[test]
    fn announce_reads_each_language_and_empty_is_silent() {
        let read = |v: &str| announce_setting(&format!("[voice_chat]\nannounce = {v}\n"));
        assert_eq!(read("\"en\""), AnnounceSetting::Set(Announce::English));
        assert_eq!(read("PL"), AnnounceSetting::Set(Announce::Polish));
        assert_eq!(read("\"\""), AnnounceSetting::Set(Announce::Silent));
        assert_eq!(read("\"de\""), AnnounceSetting::Invalid("de".to_string()));
        assert_eq!(announce_setting(""), AnnounceSetting::NotSet);
    }

    #[test]
    fn every_language_has_two_distinct_riff_wave_clips() {
        for lang in [Announce::English, Announce::Polish] {
            let on = lang.clip(true).expect("on clip");
            let off = lang.clip(false).expect("off clip");
            for clip in [on, off] {
                assert_eq!(&clip[..4], b"RIFF");
                assert_eq!(&clip[8..12], b"WAVE");
            }
            assert_ne!(on, off);
        }
        assert_eq!(Announce::Silent.clip(true), None);
    }

    #[test]
    fn the_atomic_round_trips_every_language() {
        for lang in [Announce::Silent, Announce::English, Announce::Polish] {
            assert_eq!(Announce::from_u8(lang as u8), lang);
        }
        assert_eq!(Announce::from_u8(0xff), Announce::Silent);
    }

    #[test]
    fn another_sections_key_is_not_ours() {
        assert_eq!(
            key_setting("[voice_chat]\nenabled = true\n[inventory_sort]\nkey = \"F9\"\n"),
            KeySetting::NotSet
        );
    }
}
