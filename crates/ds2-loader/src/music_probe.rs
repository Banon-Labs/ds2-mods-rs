//! Reading `[music_probe]` out of `<Game>/ds2-mods.toml`: whether to install the music player, and
//! which key opens its panel.
//!
//! The feature lives in `ds2-music-probe`; this is only the switch, kept here for the same reason
//! every other feature's is -- the config file belongs to the loader. The playlists are not here:
//! they live in `ds2-music-playlist.toml`, which the panel writes and `scripts/ds2-run.py` never
//! rewrites.
//!
//! ```toml
//! [music_probe]
//! enabled = true
//! key = "F10"
//! ```

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "music_probe";

/// Whether to front the FMOD event import slots at all.
pub const KEY_ENABLED: &str = "enabled";

/// The key that opens and closes the Music panel, a name from `ds2_hotkey_config::keys`.
pub const KEY_KEY: &str = "key";

/// `[music_probe]`, resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicProbeConfig {
    /// Log the music events and run the player.
    ///
    /// Off by default, like every feature here: a config that says nothing is the game as shipped.
    pub enabled: bool,
    /// The panel key, as written. `F10` when the file says nothing.
    pub key: String,
}

impl Default for MusicProbeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            key: ds2_music_probe::DEFAULT_KEY.to_owned(),
        }
    }
}

impl MusicProbeConfig {
    /// Read the section. A missing file or a missing key means [`Default`].
    pub fn load() -> Self {
        let Some(path) = config_file_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        Self::from_text(&text)
    }

    /// The same decision against a config file's text, so it can be tested without one on disk.
    pub fn from_text(text: &str) -> Self {
        let parsed = KeyValues::parse(text);
        let enabled = parsed
            .get(CONFIG_SECTION, KEY_ENABLED)
            .is_some_and(|raw| raw.trim().trim_matches('"') == "true");
        let key = parsed
            .get(CONFIG_SECTION, KEY_KEY)
            .map(|raw| raw.trim().trim_matches('"').to_owned())
            .filter(|raw| !raw.is_empty())
            .unwrap_or_else(|| ds2_music_probe::DEFAULT_KEY.to_owned());
        Self { enabled, key }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_ENABLED}={} {KEY_KEY}={}",
            ds2_music_probe::LOG_PREFIX,
            self.enabled,
            self.key
        )
    }
}

#[cfg(test)]
mod tests {
    use super::MusicProbeConfig;

    #[test]
    fn only_an_exact_true_turns_it_on() {
        assert!(!MusicProbeConfig::from_text("").enabled);
        assert!(!MusicProbeConfig::from_text("[music_probe]\nenabled = yes\n").enabled);
        assert!(MusicProbeConfig::from_text("[music_probe]\nenabled = true\n").enabled);
    }

    #[test]
    fn the_key_defaults_to_f10() {
        assert_eq!(MusicProbeConfig::from_text("").key, "F10");
        assert_eq!(
            MusicProbeConfig::from_text("[music_probe]\nkey = \"F11\"\n").key,
            "F11"
        );
    }
}
