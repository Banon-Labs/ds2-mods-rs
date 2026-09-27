//! Apply a `SpEffect` to the local player from a key named in `ds2-mods.toml`.
//!
//! The port of `er-net-effects` to DARK SOULS II, first slice: one key, one effect id, applied
//! the way the game applies its own. Off unless `[net_effects] enabled = true`.
//!
//! # What a press does
//!
//! It toggles the effect, as `er-net-effects` does ([`Toggle`]). On: the effect is applied, and
//! every frame the local player's action list is searched for it ([`sp_effect_active`]); once it
//! has been seen there and then leaves -- it ran out -- it is applied again. The key switches the
//! whole feature, and it starts off. Off: every effect this crate applied since it went on (this
//! effect, the selector's kept effects and previews) is taken off the player at once with the
//! game's `removeSpEffect` ([`ds2_rva::SP_EFFECT_REMOVE`]), the selector bar is not drawn, its
//! keys do nothing, and nothing is applied or kept. On again: the effect and every kept effect are
//! applied again. An effect that never leaves the list is applied once. Each toggle, apply and expiry is one log line. Each toggle also says
//! "Net effects on." or "Net effects off." out loud, and while it is on a sparkle glyph sits at
//! the top centre of the screen ([`glyph`]).
//!
//! An apply builds the sixteen-byte request the game's own callers build ([`request_bytes`]) and calls
//! `applySpEffect` ([`ds2_rva::SP_EFFECT_APPLY`]) with the local player's `ChrSpEffectCtrl`, reached
//! as `GameManagerImp` ([`ds2_rva::GAME_MANAGER_IMP`]) -> `PlayerCtrl`
//! ([`GameManagerImp::player_ctrl`]) -> `ChrSpEffectCtrl`
//! ([`darksouls2::game::chr::CharacterCtrl::sp_effect_ctrl`]). Any link of that chain can be
//! null -- on the title screen, during loads -- and a press then does nothing and says so once in
//! the log
//! ([`sp_effect_ctrl`]). The default id is [`ds2_rva::SP_EFFECT_VISUAL_SFX`], an effect that
//! only draws an sfx, so a press changes nothing about the player. The apply call itself was
//! first measured live with [`ds2_rva::SP_EFFECT_BONFIRE_REST`], which heals.
//!
//! # Where the key is read, and why there
//!
//! Inside a consumer on `ds2-overlay`'s `Present` clock, which runs on the game thread. The
//! apply function reaches into the character's effect list and is not safe from any other thread,
//! so the key is not read from a thread of this crate's own.
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

pub mod catalog;
pub mod glyph;
pub mod marked;
pub mod selector;

#[cfg(windows)]
mod install;
#[cfg(windows)]
mod overlay;

#[cfg(windows)]
pub use install::{LogFn, Outcome, Request, install, set_logger};

use core::mem::offset_of;

use darksouls2::game::chr::{
    ChrSpEffectCtrl, PlayerCtrl, SP_EFFECT_ACTION_CAPACITY, SpEffectAction, SpEffectActionHolder,
    SpEffectActionList,
};
use darksouls2::game::game_manager::GameManagerImp;
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

/// The id applied when the file does not name one: an sfx and nothing else.
pub const DEFAULT_EFFECT: i32 = ds2_rva::SP_EFFECT_VISUAL_SFX;

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
/// is one toggle.
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
    let player = read(manager + offset_of!(GameManagerImp, player_ctrl))
        .filter(|p| *p != 0)
        .ok_or(MissingLink::PlayerCtrl)?;
    read(player + offset_of!(PlayerCtrl, base.sp_effect_ctrl))
        .filter(|p| *p != 0)
        .ok_or(MissingLink::SpEffectCtrl)
}

/// Whether any action in the controller's list belongs to `SpEffect` `id`.
///
/// `Some(true)` or `Some(false)` when the list was read, `None` when a hop was null or unreadable
/// or the count is not one the list can hold -- a load screen, or a controller being torn down --
/// which says nothing about the effect.
///
/// The walk is [`ChrSpEffectCtrl::action_holder`] -> [`SpEffectActionHolder::list`] ->
/// [`SpEffectActionList::actions`] (the first [`SpEffectActionList::count`]) ->
/// [`SpEffectAction::sp_effect_id`]. `read` answers a pointer-sized value, `read_words` `n`
/// consecutive ones (the action array in one read), `read_i32` a 32-bit one. A null slot is
/// skipped; an action whose id cannot be read is not a match.
pub fn sp_effect_active(
    ctrl: usize,
    id: i32,
    read: impl Fn(usize) -> Option<usize>,
    read_words: impl Fn(usize, usize) -> Option<Vec<usize>>,
    read_i32: impl Fn(usize) -> Option<i32>,
) -> Option<bool> {
    let holder = read(ctrl + offset_of!(ChrSpEffectCtrl, action_holder)).filter(|p| *p != 0)?;
    let list = read(holder + offset_of!(SpEffectActionHolder, list)).filter(|p| *p != 0)?;
    let count = read(list + offset_of!(SpEffectActionList, count))?;
    if count > SP_EFFECT_ACTION_CAPACITY {
        return None;
    }
    if count == 0 {
        return Some(false);
    }
    let actions = read_words(list + offset_of!(SpEffectActionList, actions), count)?;
    Some(
        actions
            .into_iter()
            .filter(|a| *a != 0)
            .any(|action| read_i32(action + offset_of!(SpEffectAction, sp_effect_id)) == Some(id)),
    )
}

/// Every `SpEffect` id in the controller's action list, in list order.
///
/// The same walk and the same refusals as [`sp_effect_active`] (`None` for a broken hop or an
/// impossible count), but one read of the list answers every id the selector and the kept
/// effects ask about. A null slot is skipped, and so is an action whose id cannot be read.
pub fn sp_effect_ids(
    ctrl: usize,
    read: impl Fn(usize) -> Option<usize>,
    read_words: impl Fn(usize, usize) -> Option<Vec<usize>>,
    read_i32: impl Fn(usize) -> Option<i32>,
) -> Option<Vec<i32>> {
    let holder = read(ctrl + offset_of!(ChrSpEffectCtrl, action_holder)).filter(|p| *p != 0)?;
    let list = read(holder + offset_of!(SpEffectActionHolder, list)).filter(|p| *p != 0)?;
    let count = read(list + offset_of!(SpEffectActionList, count))?;
    if count > SP_EFFECT_ACTION_CAPACITY {
        return None;
    }
    if count == 0 {
        return Some(Vec::new());
    }
    let actions = read_words(list + offset_of!(SpEffectActionList, actions), count)?;
    Some(
        actions
            .into_iter()
            .filter(|a| *a != 0)
            .filter_map(|action| read_i32(action + offset_of!(SpEffectAction, sp_effect_id)))
            .collect(),
    )
}

/// Frames the effect has to show up on the player after an apply before the toggle stops waiting
/// for it. About two seconds at 60 fps; the live apply was in the list by the first 50 ms sample.
pub const SIGHTING_FRAMES: u32 = 120;

/// Why an apply is asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyReason {
    /// The key turned the toggle on.
    Enabled,
    /// The effect was seen on the player and has since ended.
    Expired,
}

/// What one frame of [`Toggle::frame`] decided. Every field is something the caller logs or does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// `Some(on)` on the frame a press flipped the toggle.
    pub toggled: Option<bool>,
    /// The effect was seen on the player for the first time since the last apply.
    pub seen: bool,
    /// The effect was on the player last frame and is gone now.
    pub expired: bool,
    /// Apply now, for this reason. The caller reports back with [`Toggle::applied`].
    pub apply: Option<ApplyReason>,
    /// [`SIGHTING_FRAMES`] passed after an apply without the effect showing up. Nothing is
    /// re-applied until the key is pressed again, so an id the game ignores is not applied every
    /// frame.
    pub never_seen: bool,
}

/// The on/off state of the effect, ported from `er-net-effects`.
///
/// A press enables it; while it is enabled, an effect that has been seen on the player and then ends is applied again; a press
/// disables it and nothing more is applied. An effect that never ends is applied once.
///
/// The ER mod's `refresh_call_status` and `reapply_expired_enabled_calls` are the two halves of
/// [`Toggle::frame`]. One difference: after a re-apply ER keeps its "seen" mark, so an apply the
/// game ignores is retried every frame; here a re-apply clears it and waits
/// [`SIGHTING_FRAMES`] for the effect to show up again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Toggle {
    enabled: bool,
    seen: bool,
    pending: Option<ApplyReason>,
    waiting: u32,
}

impl Toggle {
    /// Off, nothing seen, nothing waiting.
    pub const fn new() -> Self {
        Self {
            enabled: false,
            seen: false,
            pending: None,
            waiting: 0,
        }
    }

    /// Whether the key has the effect on.
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    /// One frame. `pressed` is the key's down edge; `active` is [`sp_effect_active`] for the
    /// effect, `None` when the list could not be read, which neither sees nor expires anything and
    /// holds a wanted apply until it can be read.
    pub fn frame(&mut self, pressed: bool, active: Option<bool>) -> Frame {
        let mut out = Frame::default();
        if pressed {
            self.enabled = !self.enabled;
            self.seen = false;
            self.waiting = 0;
            self.pending = self.enabled.then_some(ApplyReason::Enabled);
            out.toggled = Some(self.enabled);
        }
        if !self.enabled {
            return out;
        }
        let Some(active) = active else {
            return out;
        };
        if active {
            out.seen = !self.seen;
            self.seen = true;
            self.waiting = 0;
        } else if self.seen {
            self.seen = false;
            out.expired = true;
            self.pending = Some(ApplyReason::Expired);
        } else if self.waiting > 0 {
            self.waiting -= 1;
            out.never_seen = self.waiting == 0;
        }
        out.apply = self.pending;
        out
    }

    /// The apply [`Frame::apply`] asked for was made. Start waiting for the effect to show up.
    pub fn applied(&mut self) {
        self.pending = None;
        self.seen = false;
        self.waiting = SIGHTING_FRAMES;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// The frames the live 140001010 run would produce: press, apply, seen, expired, re-apply,
    /// seen, expired, re-apply, press off, and then nothing however long the effect is gone.
    #[test]
    fn a_toggle_reapplies_after_expiry_until_it_is_turned_off() {
        let mut t = Toggle::default();
        let f = t.frame(true, Some(false));
        assert_eq!(f.toggled, Some(true));
        assert_eq!(f.apply, Some(ApplyReason::Enabled));
        t.applied();
        for _ in 0..2 {
            let f = t.frame(false, Some(true));
            assert!(f.seen);
            assert_eq!(f.apply, None);
            assert_eq!(
                t.frame(false, Some(true)),
                Frame::default(),
                "still on, nothing to do"
            );
            let f = t.frame(false, Some(false));
            assert!(f.expired);
            assert_eq!(f.apply, Some(ApplyReason::Expired));
            t.applied();
        }
        let f = t.frame(true, Some(true));
        assert_eq!(f.toggled, Some(false));
        assert_eq!(f.apply, None);
        assert!(!t.enabled());
        for _ in 0..500 {
            assert_eq!(t.frame(false, Some(false)), Frame::default());
        }
    }

    /// An effect that never leaves the list is applied once and never again.
    #[test]
    fn an_effect_that_never_ends_is_applied_once() {
        let mut t = Toggle::default();
        assert_eq!(t.frame(true, Some(false)).apply, Some(ApplyReason::Enabled));
        t.applied();
        assert!(t.frame(false, Some(true)).seen);
        for _ in 0..10_000 {
            assert_eq!(t.frame(false, Some(true)), Frame::default());
        }
    }

    /// Not seen yet is not expired: the frames between the apply and the first sighting apply
    /// nothing, and an apply the game never shows gives up once instead of repeating.
    #[test]
    fn an_apply_that_never_shows_up_is_not_repeated() {
        let mut t = Toggle::default();
        t.frame(true, Some(false));
        t.applied();
        let mut gave_up = 0;
        for _ in 0..SIGHTING_FRAMES * 3 {
            let f = t.frame(false, Some(false));
            assert_eq!(f.apply, None);
            gave_up += usize::from(f.never_seen);
        }
        assert_eq!(gave_up, 1);
        assert!(t.enabled(), "still on: a press turns it off, not the game");
    }

    /// An unreadable list (a load screen) neither expires the effect nor loses the wanted apply.
    #[test]
    fn an_unreadable_list_holds_everything() {
        let mut t = Toggle::default();
        let f = t.frame(true, None);
        assert_eq!(f.toggled, Some(true));
        assert_eq!(f.apply, None, "no controller to apply to");
        assert_eq!(t.frame(false, None).apply, None);
        assert_eq!(
            t.frame(false, Some(false)).apply,
            Some(ApplyReason::Enabled)
        );
        t.applied();
        t.frame(false, Some(true));
        assert_eq!(
            t.frame(false, None),
            Frame::default(),
            "a load is not an expiry"
        );
        assert!(t.frame(false, Some(false)).expired);
    }

    /// A press while the effect is off and a press while it is on both flip it; turning it off
    /// drops an apply that was still waiting for the controller.
    #[test]
    fn turning_it_off_drops_a_waiting_apply() {
        let mut t = Toggle::default();
        t.frame(true, None);
        assert_eq!(t.frame(true, None).toggled, Some(false));
        assert_eq!(t.frame(false, Some(false)).apply, None);
    }

    const LIST_CTRL: usize = 0x10_000;
    const HOLDER: usize = 0x20_000;
    const LIST: usize = 0x30_000;

    fn list_heap(count: usize, ids: &[i32]) -> HashMap<usize, u64> {
        let mut heap = HashMap::new();
        heap.insert(LIST_CTRL + 0x10, HOLDER as u64);
        heap.insert(HOLDER + 0x20, LIST as u64);
        heap.insert(LIST + 0x418, count as u64);
        for (i, id) in ids.iter().enumerate() {
            let action = 0x40_000 + i * 0x100;
            heap.insert(
                LIST + 0x10 + i * 8,
                if *id == 0 { 0 } else { action as u64 },
            );
            heap.insert(action + 0x10, *id as u32 as u64);
        }
        heap
    }

    fn active(heap: &HashMap<usize, u64>, id: i32) -> Option<bool> {
        let word = |a: usize| heap.get(&a).map(|v| *v as usize);
        sp_effect_active(
            LIST_CTRL,
            id,
            word,
            |a, n| (0..n).map(|i| word(a + i * 8)).collect(),
            |a| heap.get(&a).map(|v| *v as u32 as i32),
        )
    }

    /// The resident actions the live read found, and the sfx effect among them or not.
    #[test]
    fn the_list_is_searched_by_id() {
        let resident = [40_040_002, 41_110_000, 0, 21_660_100];
        assert_eq!(active(&list_heap(4, &resident), 140_001_010), Some(false));
        let with = [40_040_002, 41_110_000, 140_001_010];
        assert_eq!(active(&list_heap(3, &with), 140_001_010), Some(true));
        assert_eq!(active(&list_heap(0, &[]), 140_001_010), Some(false));
    }

    fn ids(heap: &HashMap<usize, u64>) -> Option<Vec<i32>> {
        let word = |a: usize| heap.get(&a).map(|v| *v as usize);
        sp_effect_ids(
            LIST_CTRL,
            word,
            |a, n| (0..n).map(|i| word(a + i * 8)).collect(),
            |a| heap.get(&a).map(|v| *v as u32 as i32),
        )
    }

    /// One read answers every id: nulls skipped, the count respected, a broken hop unknown.
    #[test]
    fn the_list_reads_as_all_its_ids() {
        let heap = list_heap(4, &[40_040_002, 0, 140_001_010, 21_660_100, 7]);
        assert_eq!(ids(&heap), Some(vec![40_040_002, 140_001_010, 21_660_100]));
        assert_eq!(ids(&list_heap(0, &[])), Some(vec![]));
        assert_eq!(ids(&list_heap(129, &[1])), None);
        assert_eq!(ids(&HashMap::new()), None);
    }

    /// Only the first `count` slots are live: a stale pointer past it is not a match.
    #[test]
    fn slots_past_the_count_are_not_read() {
        let heap = list_heap(2, &[40_040_002, 41_110_000, 140_001_010]);
        assert_eq!(active(&heap, 140_001_010), Some(false));
    }

    /// A missing hop or an impossible count says nothing about the effect.
    #[test]
    fn a_broken_list_is_unknown() {
        assert_eq!(active(&HashMap::new(), 1), None);
        let mut heap = list_heap(1, &[1]);
        heap.insert(HOLDER + 0x20, 0);
        assert_eq!(active(&heap, 1), None);
        let heap = list_heap(129, &[1]);
        assert_eq!(active(&heap, 1), None);
    }

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
    fn the_default_is_f9_and_the_visual_sfx() {
        assert_eq!(default_chord(), Some(chord("F9")));
        assert_eq!(DEFAULT_EFFECT, 140_001_010);
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
