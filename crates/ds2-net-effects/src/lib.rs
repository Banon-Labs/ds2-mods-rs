//! Apply a `SpEffect` to the local player from a key named in `ds2-mods.toml`.
//!
//! The port of `er-net-effects` to DARK SOULS II, first slice: one key, one effect id, applied
//! the way the game applies its own. Off unless `[net_effects] enabled = true`.
//!
//! # What a press does
//!
//! It builds the sixteen-byte request the game's own callers build ([`request_bytes`]) and calls
//! `applySpEffect` ([`ds2_rva::SP_EFFECT_APPLY`]) with the local player's `ChrSpEffectCtrl`, reached
//! as `GameManagerImp` ([`ds2_rva::GAME_MANAGER_IMP`]) -> `PlayerCtrl`
//! ([`ds2_rva::PLAYER_CTRL_OFFSET`]) -> `ChrSpEffectCtrl`
//! ([`ds2_rva::PLAYER_CTRL_SP_EFFECT_CTRL_OFFSET`]). Any link of that chain can be null -- on the
//! title screen, during loads -- and a press then does nothing and says so once in the log
//! ([`sp_effect_ctrl`]). The default id is [`ds2_rva::SP_EFFECT_BONFIRE_REST`], resting at a
//! bonfire, because that call has been made live and its result measured.
//!
//! # Where the key is read, and why there
//!
//! Inside a consumer on `ds2-invasion-path`'s `Present` clock, which runs on the game thread. The
//! apply function reaches into the character's effect list and is not safe from any other thread,
//! so the key is not read from a thread of this crate's own. The clock exists only while
//! `[invasion_path]` is on (its `Present` detour is what calls the consumers); the loader says so
//! when it is off.
//!
//! The apply function's entry is an Arxan redirect. Calling it is an ordinary call; this crate
//! checks those five bytes before it calls and never hooks the function.
//!
//! # What it does not do
//!
//! * Choose what other players see. Whether an effect applied this way reaches the other players
//!   in a session is not known yet; the crate adds no network code of its own.
//! * Validate the id. There is no `SpEffectParam` in this game: an id is an event in one of the
//!   regulation's `SpEffect*.emevd` files, and what the game does with an id that names no event
//!   is not known. [`effect_setting`] refuses only ids that cannot be one (zero and negatives).
//!
//! # The binding moves without restarting the game
//!
//! Same as `ds2-voice-chat`: a watcher thread re-reads the config about once a second and
//! publishes the chord and the id into atomics the frame consumer loads. Default key `F9`.

#[cfg(windows)]
mod install;

#[cfg(windows)]
pub use install::{LogFn, Outcome, Request, install, set_logger};

use ds2_hotkey_config::keys::{
    Chord, KeyParseError, MODIFIER_ALT, MODIFIER_CTRL, MODIFIER_SHIFT, parse_chord,
};
use ds2_hotkey_config::kv::KeyValues;

/// Prefix on every line this crate writes to the loader log.
pub const LOG_PREFIX: &str = "ds2-net-effects:";

/// The config section. Mirrored in `crates/ds2-loader/src/net_effects.rs`.
pub const CONFIG_SECTION: &str = "net_effects";

/// The keyboard binding key. A name from [`ds2_hotkey_config::keys`], e.g. `"F9"`, `"Ctrl+H"`.
pub const CONFIG_KEY_KEYBOARD: &str = "key";

/// The binding in force when the file does not name one. Clear of `F7` (`ds2-inventory-sort`),
/// `F8` (`ds2-voice-chat`) and `;` (`ds2-invasion-path`), the other keys this workspace polls by
/// default.
pub const DEFAULT_KEY: &str = "F9";

/// The effect id key.
pub const CONFIG_KEY_EFFECT: &str = "effect";

/// The id applied when the file does not name one: resting at a bonfire.
pub const DEFAULT_EFFECT: i32 = ds2_rva::SP_EFFECT_BONFIRE_REST;

/// `VK_CONTROL`, `VK_MENU`, `VK_SHIFT` -- the three modifiers a [`Chord`] can carry.
const VK_CONTROL: i32 = 0x11;
const VK_MENU: i32 = 0x12;
const VK_SHIFT: i32 = 0x10;

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

/// Read `[net_effects] key` out of a config text.
pub fn key_setting(text: &str) -> KeySetting {
    let parsed = KeyValues::parse(text);
    let Some(raw) = parsed.get(CONFIG_SECTION, CONFIG_KEY_KEYBOARD) else {
        return KeySetting::NotSet;
    };
    let value = scalar(raw);
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

/// What one config text says about the effect id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectSetting {
    /// The key is not in the file. [`DEFAULT_EFFECT`] stands.
    NotSet,
    /// A positive id.
    Set(i32),
    /// Not a positive `i32`. The id already in force stays in force.
    Invalid(String),
}

/// Read `[net_effects] effect` out of a config text.
pub fn effect_setting(text: &str) -> EffectSetting {
    let parsed = KeyValues::parse(text);
    let Some(raw) = parsed.get(CONFIG_SECTION, CONFIG_KEY_EFFECT) else {
        return EffectSetting::NotSet;
    };
    let value = scalar(raw);
    match value.replace('_', "").parse::<i32>() {
        Ok(id) if id > 0 => EffectSetting::Set(id),
        _ => EffectSetting::Invalid(value.to_string()),
    }
}

/// Whether an effect this crate applies may be sent to the other players in the session.
pub const CONFIG_KEY_NETWORK: &str = "network";

/// Read `[net_effects] network` out of a config text. `false` when absent or not exactly `true`:
/// sending is opt-in, because the game sends a local player's applied effect to the whole session.
pub fn network_setting(text: &str) -> bool {
    KeyValues::parse(text)
        .get(CONFIG_SECTION, CONFIG_KEY_NETWORK)
        .is_some_and(|raw| scalar(raw) == "true")
}

/// Strip a trailing `# comment` and the quotes a TOML string carries.
fn scalar(raw: &str) -> &str {
    let text = raw.split('#').next().unwrap_or(raw).trim();
    text.trim_matches('"').trim()
}

/// The chord [`DEFAULT_KEY`] names, or `None` if the key table ever stops knowing it -- which
/// would be a bug here, and leaves the feature unbound rather than bound to a guess.
pub fn default_chord() -> Option<Chord> {
    parse_chord(DEFAULT_KEY).ok()
}

/// Whether every modifier and the trigger of `chord` are held, asking `vk_down` about each
/// virtual key. An unbound chord (`vk == 0`) is never held.
pub fn chord_held(chord: Chord, vk_down: impl Fn(i32) -> bool) -> bool {
    if chord.vk == 0 {
        return false;
    }
    if chord.modifiers & MODIFIER_CTRL != 0 && !vk_down(VK_CONTROL) {
        return false;
    }
    if chord.modifiers & MODIFIER_ALT != 0 && !vk_down(VK_MENU) {
        return false;
    }
    if chord.modifiers & MODIFIER_SHIFT != 0 && !vk_down(VK_SHIFT) {
        return false;
    }
    vk_down(chord.vk as i32)
}

/// A press is the frame the chord goes down: held now, not held the frame before. Holding it
/// applies the effect once.
pub const fn is_press(was_down: bool, down: bool) -> bool {
    down && !was_down
}

/// The request the game's own callers build, for `id`.
///
/// `{id, 1, -1.0, slot 25, 1, 0, 0}`, laid out at the `ds2_rva` offsets: the bytes the live call
/// used.
pub fn request_bytes(id: i32) -> [u8; ds2_rva::SP_EFFECT_REQUEST_SIZE] {
    let mut bytes = [0u8; ds2_rva::SP_EFFECT_REQUEST_SIZE];
    bytes[ds2_rva::SP_EFFECT_REQUEST_ID_OFFSET..][..4].copy_from_slice(&id.to_le_bytes());
    bytes[ds2_rva::SP_EFFECT_REQUEST_COUNT_OFFSET..][..4]
        .copy_from_slice(&ds2_rva::SP_EFFECT_REQUEST_COUNT.to_le_bytes());
    bytes[ds2_rva::SP_EFFECT_REQUEST_DURATION_OFFSET..][..4]
        .copy_from_slice(&ds2_rva::SP_EFFECT_REQUEST_DURATION.to_le_bytes());
    bytes[ds2_rva::SP_EFFECT_REQUEST_SLOT_OFFSET] = ds2_rva::SP_EFFECT_REQUEST_SLOT;
    bytes[ds2_rva::SP_EFFECT_REQUEST_KIND_OFFSET] = ds2_rva::SP_EFFECT_REQUEST_KIND;
    bytes[ds2_rva::SP_EFFECT_REQUEST_RESERVED_OFFSET] = ds2_rva::SP_EFFECT_REQUEST_RESERVED;
    bytes[ds2_rva::SP_EFFECT_REQUEST_FLAGS_OFFSET] = ds2_rva::SP_EFFECT_REQUEST_FLAGS;
    bytes
}

/// The link of the controller chain that was null or unreadable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissingLink {
    /// `GameManagerImp` itself.
    GameManager,
    /// `GameManagerImp -> PlayerCtrl`: no character is loaded.
    PlayerCtrl,
    /// `PlayerCtrl -> ChrSpEffectCtrl`: the character has no effect controller.
    SpEffectCtrl,
}

/// The local player's `ChrSpEffectCtrl`, walked from the address of `GameManagerImp`'s global.
///
/// `read` answers a pointer-sized value at an address, or `None` when it is unreadable; a null and
/// an unreadable link are both [`MissingLink`], named by the link that failed.
///
/// # Errors
///
/// The first link that was null or unreadable.
pub fn sp_effect_ctrl(
    game_manager_global: usize,
    read: impl Fn(usize) -> Option<usize>,
) -> Result<usize, MissingLink> {
    let manager = read(game_manager_global)
        .filter(|p| *p != 0)
        .ok_or(MissingLink::GameManager)?;
    let player = read(manager + ds2_rva::PLAYER_CTRL_OFFSET)
        .filter(|p| *p != 0)
        .ok_or(MissingLink::PlayerCtrl)?;
    read(player + ds2_rva::PLAYER_CTRL_SP_EFFECT_CTRL_OFFSET)
        .filter(|p| *p != 0)
        .ok_or(MissingLink::SpEffectCtrl)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Sending is off unless the file says exactly `true`.
    #[test]
    fn network_is_off_unless_asked_for() {
        assert!(!network_setting(""));
        assert!(!network_setting("[net_effects]\nnetwork = false\n"));
        assert!(!network_setting("[net_effects]\nnetwork = yes\n"));
        assert!(network_setting("[net_effects]\nnetwork = true # send\n"));
        assert!(!network_setting("[other]\nnetwork = true\n"));
    }

    fn chord(name: &str) -> Chord {
        parse_chord(name).expect("a name the key table knows")
    }

    #[test]
    fn the_default_is_f9_and_the_bonfire() {
        assert_eq!(default_chord(), Some(chord("F9")));
        assert_eq!(DEFAULT_EFFECT, 110_000_010);
    }

    #[test]
    fn no_section_means_the_defaults_stand() {
        assert_eq!(key_setting(""), KeySetting::NotSet);
        assert_eq!(effect_setting(""), EffectSetting::NotSet);
        let other = "[voice_chat]\nkey = \"F10\"\n[inventory_sort]\nkey = \"F11\"\n";
        assert_eq!(key_setting(other), KeySetting::NotSet);
    }

    #[test]
    fn a_named_key_is_read_quoted_or_not() {
        let want = KeySetting::Bound(chord("F10"));
        assert_eq!(key_setting("[net_effects]\nkey = \"F10\"\n"), want);
        assert_eq!(key_setting("[net_effects]\nkey = F10\n"), want);
        assert_eq!(
            key_setting("[net_effects]\nkey = \"Ctrl+H\"\n"),
            KeySetting::Bound(chord("Ctrl+H"))
        );
    }

    #[test]
    fn an_empty_key_unbinds_and_junk_is_named() {
        assert_eq!(
            key_setting("[net_effects]\nkey = \"\"\n"),
            KeySetting::Unbound
        );
        match key_setting("[net_effects]\nkey = \"NotAKey\"\n") {
            KeySetting::Invalid { value, .. } => assert_eq!(value, "NotAKey"),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn an_effect_id_is_read_with_or_without_quotes_and_separators() {
        let want = EffectSetting::Set(62_170_010);
        assert_eq!(effect_setting("[net_effects]\neffect = 62170010\n"), want);
        assert_eq!(
            effect_setting("[net_effects]\neffect = \"62170010\"\n"),
            want
        );
        assert_eq!(effect_setting("[net_effects]\neffect = 62_170_010\n"), want);
        assert_eq!(
            effect_setting("[net_effects]\neffect = 110000010 # bonfire\n"),
            EffectSetting::Set(110_000_010)
        );
    }

    #[test]
    fn an_effect_that_cannot_be_an_id_is_refused() {
        for bad in ["0", "-5", "lifegem", "", "99999999999"] {
            assert_eq!(
                effect_setting(&format!("[net_effects]\neffect = {bad}\n")),
                EffectSetting::Invalid(bad.to_string()),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_press_is_the_down_edge_only() {
        assert!(is_press(false, true));
        assert!(
            !is_press(true, true),
            "holding the key is not a second press"
        );
        assert!(!is_press(true, false));
        assert!(!is_press(false, false));
    }

    #[test]
    fn a_chord_needs_its_modifiers_and_an_unbound_one_is_never_held() {
        let ctrl_h = chord("Ctrl+H");
        let only_h = |vk: i32| vk == ctrl_h.vk as i32;
        assert!(!chord_held(ctrl_h, only_h));
        assert!(chord_held(ctrl_h, |vk| vk == ctrl_h.vk as i32 || vk == VK_CONTROL));
        assert!(chord_held(chord("F9"), |vk| vk == 0x78));
        let unbound = Chord {
            modifiers: 0,
            vk: 0,
            dik: None,
        };
        assert!(!chord_held(unbound, |_| true));
    }

    #[test]
    fn the_request_is_the_bytes_the_live_call_used() {
        let bytes = request_bytes(110_000_010);
        let mut want = Vec::new();
        want.extend_from_slice(&110_000_010i32.to_le_bytes());
        want.extend_from_slice(&1i32.to_le_bytes());
        want.extend_from_slice(&(-1.0f32).to_le_bytes());
        want.extend_from_slice(&[25, 1, 0, 0]);
        assert_eq!(bytes.as_slice(), want.as_slice());
        assert_eq!(&bytes[8..12], &[0x00, 0x00, 0x80, 0xbf]);
    }

    #[test]
    fn only_the_id_changes_between_requests() {
        let a = request_bytes(110_000_010);
        let b = request_bytes(62_170_010);
        assert_eq!(&a[4..], &b[4..]);
        assert_eq!(&b[..4], &62_170_010i32.to_le_bytes());
    }

    const GLOBAL: usize = 0x1_4161_48f0;
    const MANAGER: usize = 0x1000;
    const PLAYER: usize = 0x2000;
    const CTRL: usize = 0x3000;

    fn heap(links: &[(usize, usize)]) -> impl Fn(usize) -> Option<usize> {
        let map: HashMap<usize, usize> = links.iter().copied().collect();
        move |addr| map.get(&addr).copied()
    }

    #[test]
    fn the_chain_reaches_the_controller() {
        let read = heap(&[
            (GLOBAL, MANAGER),
            (MANAGER + 0xd0, PLAYER),
            (PLAYER + 0x3e0, CTRL),
        ]);
        assert_eq!(sp_effect_ctrl(GLOBAL, read), Ok(CTRL));
    }

    #[test]
    fn each_null_or_unreadable_link_is_named() {
        assert_eq!(
            sp_effect_ctrl(GLOBAL, heap(&[(GLOBAL, 0)])),
            Err(MissingLink::GameManager)
        );
        assert_eq!(
            sp_effect_ctrl(GLOBAL, heap(&[])),
            Err(MissingLink::GameManager)
        );
        assert_eq!(
            sp_effect_ctrl(GLOBAL, heap(&[(GLOBAL, MANAGER), (MANAGER + 0xd0, 0)])),
            Err(MissingLink::PlayerCtrl)
        );
        assert_eq!(
            sp_effect_ctrl(GLOBAL, heap(&[(GLOBAL, MANAGER), (MANAGER + 0xd0, PLAYER)])),
            Err(MissingLink::SpEffectCtrl)
        );
        assert_eq!(
            sp_effect_ctrl(
                GLOBAL,
                heap(&[
                    (GLOBAL, MANAGER),
                    (MANAGER + 0xd0, PLAYER),
                    (PLAYER + 0x3e0, 0)
                ])
            ),
            Err(MissingLink::SpEffectCtrl)
        );
    }
}
