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
//! offline with Piper, 16 kHz 16-bit mono -- English with `en_US-lessac-medium`, Polish with the CC0
//! `pl_PL-gosia-medium`, because a Polish speaker could not understand espeak-ng's Polish and the
//! English followed so both languages sound alike -- and compiled into the DLL from
//! `assets/`, so there is no speech engine and no file to go missing. `[voice_chat] announce` picks
//! the language; `""` silences it.
//!
//! # And the HUD shows it
//!
//! The HUD already has a voice chat icon (`FeScenePlayerVoiceChatIcon`), driven every frame from
//! the network's voice state through a four-entry table of show/hide bytes. A second detour
//! replaces that frame's choice with the Voice chat byte: on applies the game's own state 2, off
//! its state 0 (everything hidden). While the options block does not exist the original runs
//! untouched. See [`icon_visibility`] and `ds2_rva::FE_VOICE_CHAT_ICON_UPDATE`.
//!
//! The icon's animation plays at [`ICON_PLAYBACK_RATE`], a tenth of the shipped speed: the same
//! detour writes that rate into every sprite of the icon's own layout scene ([`icon_sprites`]),
//! and the game's sprite tick multiplies its delta by it. Other HUD elements keep their speed.

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

/// The HUD voice chat icon's state word for a Voice chat byte.
///
/// On is the game's own state 2 (root and the `3e0/3e2` shape shown); off is state 0, everything
/// hidden. The word is one of the game's own table, not a value composed here.
pub const fn icon_state_word(voice_chat: u8) -> u32 {
    let index = if voice_chat_on(voice_chat) {
        ds2_rva::FE_VOICE_CHAT_ICON_STATE_ON
    } else {
        ds2_rva::FE_VOICE_CHAT_ICON_STATE_HIDDEN
    };
    ds2_rva::FE_VOICE_CHAT_ICON_STATES[index]
}

/// The four show/hide bytes the icon's update applies, in its order: root, `3e0/3e0`, `3e0/3e1`,
/// `3e0/3e2`. Nonzero is shown.
pub const fn icon_visibility(voice_chat: u8) -> [u8; 4] {
    icon_state_word(voice_chat).to_le_bytes()
}

/// The HUD icon's playback rate: a tenth of the game's own, because the shipped animation is
/// distracting.
///
/// Written into every `FeComponentSprite` of the icon's layout (`ds2_rva::FE_SPRITE_RATE_OFFSET`),
/// which the sprite tick multiplies its delta by. Nothing outside the icon's own scene is touched.
pub const ICON_PLAYBACK_RATE: f32 = 0.1;

/// Most components [`icon_sprites`] visits. The icon's layout has fifteen records; a live tree past
/// this is not the icon's, and the walk stops rather than wander.
pub const ICON_WALK_MAX_NODES: usize = 64;

/// Deepest level [`icon_sprites`] descends to. The icon's layout nests six deep.
pub const ICON_WALK_MAX_DEPTH: usize = 12;

/// Fault-tolerant reads of game memory, so the walk can be tested against a fake tree.
pub trait ComponentMemory {
    /// A pointer-sized value, or `None` when the address is not readable.
    fn read_usize(&self, addr: usize) -> Option<usize>;
    /// A `u16`, or `None` when the address is not readable.
    fn read_u16(&self, addr: usize) -> Option<u16>;
}

/// How a component holds its children, decided by its vtable alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    /// `FeComponentSprite`: a clock of its own, children in its display list.
    Sprite,
    /// `FeComponentObject` / `FeComponentScene`: children on the `+0x38` list.
    Parent,
    /// Everything else. Its `+0x38` is some other field, and following it crashed a walk once.
    Leaf,
}

fn class_of(vtable: usize, base: usize) -> Class {
    let Some(rva) = vtable.checked_sub(base) else {
        return Class::Leaf;
    };
    if rva == ds2_rva::FE_COMPONENT_SPRITE_VTABLE as usize {
        Class::Sprite
    } else if rva == ds2_rva::FE_COMPONENT_OBJECT_VTABLE as usize
        || rva == ds2_rva::FE_COMPONENT_SCENE_VTABLE as usize
    {
        Class::Parent
    } else {
        Class::Leaf
    }
}

/// Every `FeComponentSprite` under `root`, `root` included.
///
/// Children are followed the way the sprite tick (`0x140b6ce80`) does: a sprite's display list
/// when it has one, else the `+0x38` list; `FeComponentObject`/`FeComponentScene` through `+0x38`
/// and `+0x28`; nothing else descends.
///
/// Bounded by [`ICON_WALK_MAX_NODES`] and [`ICON_WALK_MAX_DEPTH`], and a component seen twice is
/// not walked twice, so a corrupt link cannot loop it.
pub fn icon_sprites(root: usize, base: usize, memory: &impl ComponentMemory) -> Vec<usize> {
    let mut sprites = Vec::new();
    let mut seen = Vec::new();
    let mut stack = vec![(root, 0usize)];
    while let Some((node, depth)) = stack.pop() {
        if node == 0 || seen.contains(&node) || seen.len() >= ICON_WALK_MAX_NODES {
            continue;
        }
        seen.push(node);
        let Some(vtable) = memory.read_usize(node) else {
            continue;
        };
        let class = class_of(vtable, base);
        if class == Class::Sprite {
            sprites.push(node);
        }
        if class == Class::Leaf || depth >= ICON_WALK_MAX_DEPTH {
            continue;
        }
        let list = if class == Class::Sprite {
            memory
                .read_usize(node + ds2_rva::FE_COMPONENT_DISPLAY_LIST_OFFSET)
                .unwrap_or(0)
        } else {
            0
        };
        if list != 0 {
            let count = memory
                .read_u16(node + ds2_rva::FE_COMPONENT_DISPLAY_COUNT_OFFSET)
                .unwrap_or(0);
            for i in 0..usize::from(count).min(ICON_WALK_MAX_NODES) {
                let entry = list
                    + i * ds2_rva::FE_COMPONENT_DISPLAY_ENTRY_STRIDE
                    + ds2_rva::FE_COMPONENT_DISPLAY_ENTRY_CHILD_OFFSET;
                if let Some(child) = memory.read_usize(entry) {
                    stack.push((child, depth + 1));
                }
            }
        } else {
            let mut child = memory
                .read_usize(node + ds2_rva::FE_COMPONENT_FIRST_CHILD_OFFSET)
                .unwrap_or(0);
            let mut links = 0;
            while child != 0 && links < ICON_WALK_MAX_NODES {
                stack.push((child, depth + 1));
                child = memory
                    .read_usize(child + ds2_rva::FE_COMPONENT_NEXT_SIBLING_OFFSET)
                    .unwrap_or(0);
                links += 1;
            }
        }
    }
    sprites
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
    fn on_is_the_games_state_two_root_and_the_0x35_shape() {
        assert_eq!(icon_state_word(0), 0x0100_0001);
        assert_eq!(icon_visibility(0), [1, 0, 0, 1]);
    }

    #[test]
    fn off_hides_every_element() {
        assert_eq!(icon_state_word(1), 0);
        assert_eq!(icon_visibility(1), [0; 4]);
        assert_eq!(icon_visibility(0xff), [0; 4]);
    }

    #[test]
    fn a_press_moves_the_icon_with_the_byte() {
        assert_eq!(icon_visibility(toggled(1)), [1, 0, 0, 1]);
        assert_eq!(icon_visibility(toggled(0)), [0; 4]);
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

    /// A fake heap: address -> value, anything else unreadable.
    #[derive(Default)]
    struct Fake {
        words: std::collections::HashMap<usize, usize>,
        halves: std::collections::HashMap<usize, u16>,
    }

    impl ComponentMemory for Fake {
        fn read_usize(&self, addr: usize) -> Option<usize> {
            self.words.get(&addr).copied()
        }
        fn read_u16(&self, addr: usize) -> Option<u16> {
            self.halves.get(&addr).copied()
        }
    }

    const BASE: usize = 0x1_4000_0000;

    impl Fake {
        fn node(&mut self, at: usize, vtable_rva: u32) {
            self.words.insert(at, BASE + vtable_rva as usize);
        }
        fn display_list(&mut self, sprite: usize, list: usize, children: &[usize]) {
            self.words
                .insert(sprite + ds2_rva::FE_COMPONENT_DISPLAY_LIST_OFFSET, list);
            self.halves.insert(
                sprite + ds2_rva::FE_COMPONENT_DISPLAY_COUNT_OFFSET,
                children.len() as u16,
            );
            for (i, child) in children.iter().enumerate() {
                self.words.insert(
                    list + i * ds2_rva::FE_COMPONENT_DISPLAY_ENTRY_STRIDE,
                    *child,
                );
            }
        }
        fn child_list(&mut self, parent: usize, children: &[usize]) {
            let mut link = parent + ds2_rva::FE_COMPONENT_FIRST_CHILD_OFFSET;
            for child in children {
                self.words.insert(link, *child);
                link = child + ds2_rva::FE_COMPONENT_NEXT_SIBLING_OFFSET;
            }
        }
    }

    const SPRITE: u32 = ds2_rva::FE_COMPONENT_SPRITE_VTABLE;
    const OBJECT: u32 = ds2_rva::FE_COMPONENT_OBJECT_VTABLE;
    const SHAPE: u32 = ds2_rva::FE_COMPONENT_TEXTURE_SHAPE_VTABLE;

    #[test]
    fn the_rate_is_a_tenth() {
        assert_eq!(ICON_PLAYBACK_RATE, 0.1);
    }

    /// The icon's own shape: root sprite (def 0x3b) -> object -> sprite (def 0x3a) -> a shape and
    /// an object holding a nested sprite (def 0x39).
    #[test]
    fn every_sprite_in_the_icon_is_found_through_objects_and_display_lists() {
        let mut m = Fake::default();
        m.node(0x1000, SPRITE);
        m.display_list(0x1000, 0x9000, &[0x2000]);
        m.node(0x2000, OBJECT);
        m.child_list(0x2000, &[0x3000]);
        m.node(0x3000, SPRITE);
        m.display_list(0x3000, 0x9100, &[0x4000, 0x5000]);
        m.node(0x4000, SHAPE);
        m.node(0x5000, OBJECT);
        m.child_list(0x5000, &[0x6000]);
        m.node(0x6000, SPRITE);
        let mut found = icon_sprites(0x1000, BASE, &m);
        found.sort_unstable();
        assert_eq!(found, vec![0x1000, 0x3000, 0x6000]);
    }

    #[test]
    fn a_leafs_0x38_is_never_followed() {
        let mut m = Fake::default();
        m.node(0x1000, SHAPE);
        m.words
            .insert(0x1000 + ds2_rva::FE_COMPONENT_FIRST_CHILD_OFFSET, 0x2000);
        m.node(0x2000, SPRITE);
        assert!(icon_sprites(0x1000, BASE, &m).is_empty());
    }

    #[test]
    fn a_sprite_without_a_display_list_uses_its_child_list() {
        let mut m = Fake::default();
        m.node(0x1000, SPRITE);
        m.child_list(0x1000, &[0x2000, 0x3000]);
        m.node(0x2000, SPRITE);
        m.node(0x3000, SPRITE);
        let mut found = icon_sprites(0x1000, BASE, &m);
        found.sort_unstable();
        assert_eq!(found, vec![0x1000, 0x2000, 0x3000]);
    }

    #[test]
    fn a_cycle_or_unreadable_node_ends_the_walk() {
        let mut m = Fake::default();
        m.node(0x1000, OBJECT);
        m.child_list(0x1000, &[0x2000]);
        m.node(0x2000, OBJECT);
        m.child_list(0x2000, &[0x1000, 0xdead_0000]);
        assert!(icon_sprites(0x1000, BASE, &m).is_empty());
        assert!(icon_sprites(0, BASE, &m).is_empty());
    }

    #[test]
    fn the_walk_stops_at_its_node_cap() {
        let mut m = Fake::default();
        let children: Vec<usize> = (1..=200).map(|i| 0x10_0000 + i * 0x100).collect();
        m.node(0x1000, SPRITE);
        m.display_list(0x1000, 0x9000, &children);
        for child in &children {
            m.node(*child, SPRITE);
        }
        assert!(icon_sprites(0x1000, BASE, &m).len() <= ICON_WALK_MAX_NODES);
    }

    #[test]
    fn another_sections_key_is_not_ours() {
        assert_eq!(
            key_setting("[voice_chat]\nenabled = true\n[inventory_sort]\nkey = \"F9\"\n"),
            KeySetting::NotSet
        );
    }
}
