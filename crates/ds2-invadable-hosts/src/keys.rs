//! The panel's two keys out of `[invadable_hosts]` in `ds2-mods.toml`, re-read while the game runs.
//!
//! Host-testable. The rules are `ds2_hotkey_config::binding`'s: a malformed value keeps the key in
//! force, a value naming the key already in force is no change, and a real change says from what
//! to what. The file is watched by its text (`ds2_hotkey_config::reload`), never its mtime.

use ds2_hotkey_config::binding::{Binding, BindingUpdate};
use ds2_hotkey_config::keys::{VirtualKey, parse_virtual_key, vk_name};
use ds2_hotkey_config::kv::KeyValues;

/// The section the panel's settings live in. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "invadable_hosts";
/// Whether the feature is installed at all.
pub const KEY_ENABLED: &str = "enabled";
/// The key that opens and closes the panel.
pub const KEY_MENU: &str = "key";
/// The key that switches the grid between hosts now and new hosts per minute.
pub const KEY_VIEW: &str = "view_key";

/// The panel key when the config names none: `F4`.
///
/// Every key the repo ships is taken by something else: F5 armour sync, F6 weapon sync, F7
/// inventory sort, F8 voice chat, F9 net effects, F10 the music panel, `;` the invasion path, and
/// the Lighting Engine's `dxgi.dll` polls F1, F2, F3 and F6 (`scripts/ds2-run.py`'s key notes).
/// `F4` is none of them. The test `the_defaults_collide_with_no_shipped_key` holds that.
pub const DEFAULT_MENU_KEY: &str = "F4";
/// The view key when the config names none: `R`. Read only while the panel is open, and the panel
/// takes the keyboard from the game while it is, so the letter reaches nothing else.
pub const DEFAULT_VIEW_KEY: &str = "R";

/// Every hotkey another feature of this repo or the Lighting Engine ships by default, by name.
pub const SHIPPED_KEYS: &[(&str, &str)] = &[
    ("Lighting Engine", "F1"),
    ("Lighting Engine", "F2"),
    ("Lighting Engine", "F3"),
    ("Lighting Engine", "F6"),
    ("armor_sync", "F5"),
    ("weapon_sync", "F6"),
    ("inventory_sort", "F7"),
    ("voice_chat", "F8"),
    ("net_effects", "F9"),
    ("music_probe", "F10"),
    ("music_probe", "F11"),
    ("invasion_path", "semicolon"),
];

/// The two keys in force.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keys {
    /// Opens and closes the panel.
    pub menu: Binding<VirtualKey>,
    /// Switches the view while the panel is open.
    pub view: Binding<VirtualKey>,
}

/// What one re-read of the file did to the keys, for the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeysUpdate {
    /// The panel key's update.
    pub menu: BindingUpdate<VirtualKey>,
    /// The view key's update.
    pub view: BindingUpdate<VirtualKey>,
}

impl KeysUpdate {
    /// Did either key move? The edge latches must be reset when one did.
    #[must_use]
    pub const fn changed(&self) -> bool {
        self.menu.changed() || self.view.changed()
    }
}

fn default_key(name: &str) -> VirtualKey {
    // Both defaults are names `parse_virtual_key` knows; the test below holds that, so 0 is never
    // what a player gets.
    parse_virtual_key(name).unwrap_or(0)
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            menu: Binding::new(default_key(DEFAULT_MENU_KEY)),
            view: Binding::new(default_key(DEFAULT_VIEW_KEY)),
        }
    }
}

fn unquoted(raw: &str) -> &str {
    raw.trim().trim_matches('"')
}

impl Keys {
    /// Apply the config file's text. An absent key keeps what is in force.
    pub fn apply(&mut self, text: &str) -> KeysUpdate {
        let parsed = KeyValues::parse(text);
        KeysUpdate {
            menu: self.menu.apply_optional(
                parsed.get(CONFIG_SECTION, KEY_MENU).map(unquoted),
                parse_virtual_key,
            ),
            view: self.view.apply_optional(
                parsed.get(CONFIG_SECTION, KEY_VIEW).map(unquoted),
                parse_virtual_key,
            ),
        }
    }

    /// `key=F4 view_key=R`, for the config line.
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "{KEY_MENU}={} {KEY_VIEW}={}",
            vk_name(self.menu.code()),
            vk_name(self.view.code())
        )
    }
}

/// Whether the file turns the feature on: an exact `true`, like every feature here.
#[must_use]
pub fn enabled(text: &str) -> bool {
    KeyValues::parse(text)
        .get(CONFIG_SECTION, KEY_ENABLED)
        .is_some_and(|raw| unquoted(raw) == "true")
}

/// One line describing what a re-read did, or `None` when it did nothing worth a line.
#[must_use]
pub fn describe_update(update: &KeysUpdate) -> Option<String> {
    let one = |name: &str, change: &BindingUpdate<VirtualKey>| match change {
        BindingUpdate::Unchanged => None,
        BindingUpdate::Changed { from, to } => {
            Some(format!("{name} {} -> {}", vk_name(*from), vk_name(*to)))
        }
        BindingUpdate::Rejected { value, error, kept } => Some(format!(
            "{name} {value:?} is not a key ({error}); keeping {}",
            vk_name(*kept)
        )),
    };
    let parts: Vec<String> = [one(KEY_MENU, &update.menu), one(KEY_VIEW, &update.view)]
        .into_iter()
        .flatten()
        .collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// One key's press edge. The latch remembers which key it is about, because a rebind would
/// otherwise inherit the old key's held state and swallow the new key's first press.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Edge {
    key: VirtualKey,
    down: bool,
}

impl Edge {
    /// Feed this frame's state of `key`; true on the frame it went down.
    pub fn pressed(&mut self, key: VirtualKey, down: bool) -> bool {
        if self.key != key {
            self.key = key;
            self.down = false;
        }
        let was = std::mem::replace(&mut self.down, down);
        down && !was
    }

    /// Forget the held state, after a rebind.
    pub fn reset(&mut self) {
        self.down = false;
    }

    /// Is the key this latch watches down as of the last frame?
    #[must_use]
    pub const fn held(&self) -> bool {
        self.down
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_keys_and_collide_with_no_shipped_key() {
        let keys = Keys::default();
        assert_ne!(keys.menu.code(), 0);
        assert_ne!(keys.view.code(), 0);
        for (owner, name) in SHIPPED_KEYS {
            let shipped = parse_virtual_key(name).expect("a shipped key is a key");
            assert_ne!(
                shipped,
                keys.menu.code(),
                "the panel key collides with {owner}"
            );
        }
        assert_eq!(keys.describe(), "key=F4 view_key=R");
    }

    #[test]
    fn a_rebind_is_a_change_and_a_typo_keeps_the_last_working_key() {
        let mut keys = Keys::default();
        let update = keys.apply("[invadable_hosts]\nkey = \"F12\"\n");
        assert!(update.changed());
        assert_eq!(vk_name(keys.menu.code()), "F12");
        assert_eq!(describe_update(&update).as_deref(), Some("key F4 -> F12"));
        let update = keys.apply("[invadable_hosts]\nkey = \"F99x\"\nview_key = \"Tab\"\n");
        assert_eq!(
            vk_name(keys.menu.code()),
            "F12",
            "a malformed key keeps F12"
        );
        assert!(update.changed(), "the view key moved");
        let line = describe_update(&update).expect("a line");
        assert!(line.contains("keeping F12"), "{line}");
        assert!(
            line.contains("view_key R -> TAB") || line.contains("view_key R -> Tab"),
            "{line}"
        );
        // Same keys written differently: nothing to do.
        let update = keys.apply("[invadable_hosts]\nkey = \"f12\"\nview_key = \"tab\"\n");
        assert!(!update.changed());
        assert_eq!(describe_update(&update), None);
        // A file that no longer names them keeps them.
        assert!(!keys.apply("").changed());
        assert_eq!(vk_name(keys.menu.code()), "F12");
    }

    #[test]
    fn only_an_exact_true_turns_it_on() {
        assert!(!enabled(""));
        assert!(!enabled("[invadable_hosts]\nenabled = yes\n"));
        assert!(enabled("[invadable_hosts]\nenabled = true\n"));
    }

    #[test]
    fn the_edge_fires_once_and_a_rebind_resets_it() {
        let mut edge = Edge::default();
        assert!(edge.pressed(0x73, true));
        assert!(!edge.pressed(0x73, true), "held is not a new press");
        assert!(!edge.pressed(0x73, false));
        assert!(edge.pressed(0x73, true));
        // Rebound while the old key was held: the new key's first press still counts.
        assert!(edge.pressed(0x7b, true));
    }
}
