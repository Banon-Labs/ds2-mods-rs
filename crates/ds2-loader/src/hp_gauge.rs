//! Reading `[hp_gauge]` out of `<Game>/ds2-mods.toml`.
//!
//! The feature lives in `ds2-hp-gauge`; this is the switch and its tuning, kept here for the
//! reason every other feature's config is -- the config file belongs to the loader.

use ds2_hotkey_config::kv::KeyValues;
use ds2_hp_gauge::Tune;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "hp_gauge";

/// Whether to resize and centre the floating HP bar and damage number.
pub const KEY_ENABLED: &str = "enabled";

/// The tuning keys, each an `f32`, each falling back to [`Tune::default`] when absent or unparsable.
pub const TUNE_KEYS: [&str; 7] = [
    "text_scale",
    "text_lift",
    "text_dx",
    "digit_advance",
    "bar_scale",
    "bar_dx",
    "bar_dy",
];

/// `[hp_gauge]`, resolved.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HpGaugeConfig {
    /// Install the three detours.
    pub enabled: bool,
    /// The sizes and offsets they apply.
    pub tune: Tune,
}

impl HpGaugeConfig {
    /// Read the section. A missing file or a missing key means [`Default`]: off, default tuning.
    pub fn load() -> Self {
        let Some(path) = config_file_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        Self::parse(&text)
    }

    /// Resolve the section out of a config file's text.
    pub fn parse(text: &str) -> Self {
        let parsed = KeyValues::parse(text);
        // Only an exact `true` turns it on, so a typo leaves the game as shipped.
        let enabled = parsed
            .get(CONFIG_SECTION, KEY_ENABLED)
            .is_some_and(|raw| raw.trim().trim_matches('"') == "true");
        let mut tune = Tune::default();
        let fields: [&mut f32; 7] = [
            &mut tune.text_scale,
            &mut tune.text_lift,
            &mut tune.text_dx,
            &mut tune.digit_advance,
            &mut tune.bar_scale,
            &mut tune.bar_dx,
            &mut tune.bar_dy,
        ];
        for (key, field) in TUNE_KEYS.iter().zip(fields) {
            if let Some(value) = parsed
                .get(CONFIG_SECTION, key)
                .and_then(|raw| raw.trim().trim_matches('"').parse::<f32>().ok())
                .filter(|value| value.is_finite())
            {
                *field = value;
            }
        }
        Self { enabled, tune }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_ENABLED}={} {:?}",
            ds2_hp_gauge::LOG_PREFIX,
            self.enabled,
            self.tune
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_off_with_the_tuned_values() {
        let config = HpGaugeConfig::parse("");
        assert!(!config.enabled);
        assert_eq!(config.tune, Tune::default());
    }

    #[test]
    fn keys_override_one_at_a_time() {
        let config = HpGaugeConfig::parse(
            "[hp_gauge]\nenabled = true\ntext_scale = 1.5\nbar_dx = \"-10\"\ndigit_advance = oops\n",
        );
        assert!(config.enabled);
        assert_eq!(config.tune.text_scale, 1.5);
        assert_eq!(config.tune.bar_dx, -10.0);
        assert_eq!(config.tune.digit_advance, Tune::default().digit_advance);
    }

    #[test]
    fn the_line_names_the_section_and_the_value() {
        let line = HpGaugeConfig::default().describe();
        assert!(line.starts_with(ds2_hp_gauge::LOG_PREFIX));
        assert!(line.contains("[hp_gauge] enabled=false"));
    }
}
