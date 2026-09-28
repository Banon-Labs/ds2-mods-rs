//! Reading `[music_probe] enabled` out of `<Game>/ds2-mods.toml`.
//!
//! The feature lives in `ds2-music-probe`; this is only the switch, kept here for the same reason
//! every other feature's is -- the config file belongs to the loader.
//!
//! ```toml
//! [music_probe]
//! enabled = true
//! ```

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "music_probe";

/// Whether to front the FMOD event import slots at all.
pub const KEY_ENABLED: &str = "enabled";

/// `[music_probe]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MusicProbeConfig {
    /// Log the music events the game starts, pauses and stops.
    ///
    /// Off by default, like every feature here: a config that says nothing is the game as shipped.
    pub enabled: bool,
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
        let enabled = KeyValues::parse(text)
            .get(CONFIG_SECTION, KEY_ENABLED)
            .is_some_and(|raw| raw.trim().trim_matches('"') == "true");
        Self { enabled }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_ENABLED}={}",
            ds2_music_probe::LOG_PREFIX,
            self.enabled
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
}
