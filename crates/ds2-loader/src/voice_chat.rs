//! Reading `[voice_chat] enabled` out of `<Game>/ds2-mods.toml`.
//!
//! The feature lives in `ds2-voice-chat`; this is the switch, kept here for the reason every other
//! feature's is -- the config file belongs to the loader. The key is not read here: it moves
//! without a restart, so the crate that reads the keyboard also watches the file for it.

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Same string as `ds2_voice_chat::CONFIG_SECTION`.
pub const CONFIG_SECTION: &str = "voice_chat";

/// Whether to install the detour at all.
pub const KEY_ENABLED: &str = "enabled";

/// `[voice_chat]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoiceChatConfig {
    /// Put the key on the game's Voice chat option.
    pub enabled: bool,
}

impl Default for VoiceChatConfig {
    /// Off, like every feature here (user directive 2026-09-25): a config that says nothing is the
    /// game as shipped. `enabled = true` binds `F8` unless `key` says otherwise.
    fn default() -> Self {
        Self { enabled: false }
    }
}

impl VoiceChatConfig {
    /// Read the section. A missing file or key means [`Default`]; an exact `true`/`false` is an
    /// answer, anything else falls back to the default and shows in [`Self::describe`].
    pub fn load() -> Self {
        let Some(path) = config_file_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        Self::parse(&text)
    }

    /// The same rule as [`Self::load`], on a text.
    pub fn parse(text: &str) -> Self {
        let parsed = KeyValues::parse(text);
        let enabled = match parsed.get(CONFIG_SECTION, KEY_ENABLED) {
            None => Self::default().enabled,
            Some(raw) => match raw.trim().trim_matches('"') {
                "false" => false,
                "true" => true,
                _ => Self::default().enabled,
            },
        };
        Self { enabled }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_ENABLED}={}",
            ds2_voice_chat::LOG_PREFIX,
            self.enabled
        )
    }
}
