//! Reading `[save_redirect]` out of `<Game>/ds2-mods.toml`.
//!
//! The feature lives in `ds2-save-redirect`; this is the switch that decides whether to turn it on,
//! kept here for the same reason every other feature's config is -- the config file belongs to the
//! loader, and a feature crate should not have to know where the game directory is.
//!
//! # OFF by default, and this one is not a style choice
//!
//! Every other default in this loader is chosen so that a typo is harmless. This feature moves
//! where DARK SOULS II reads and writes the only copy of a character, so the harmless direction is
//! "did nothing". Only an exact `true` turns it on, and `enabled = true` with no `path` is refused
//! rather than guessed at.
//!
//! # `path` names a FILE, and that is deliberate
//!
//! A file manager's "copy full path" produces the path of a file, so that is what this takes.
//! Four shapes are accepted, told apart by extension:
//!
//! * `.sl2` -- the save itself
//! * `.zip`, `.7z`, `.rar` -- an archive with exactly one `DS2SOFS0000.sl2` anywhere inside it
//!
//! ```text
//! path = "Z:\\home\\you\\DS2\\Dark souls 2 Sotfs Mega Mule.zip"
//! ```
//!
//! Zero copies inside an archive, or more than one, is refused by name rather than resolved by
//! picking the first.
//!
//! # The Steam ID is NOT configured
//!
//! DS2 writes the owning account's SteamID64 into the save and refuses a mismatch. The DLL rebinds
//! it during staging using the ID the game hands the hooked function as its second argument, so
//! nobody has to look theirs up. The source file is never modified.
//!
//! # It is a WINDOWS path
//!
//! This DLL runs inside the Proton prefix, so the value is a path as the prefix sees it. Wine maps
//! `Z:` to `/`, which makes `/home/you/DS2` into `Z:\home\you\DS2`. The drive letter is not
//! something this can invent.

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "save_redirect";

/// Master switch. Only an exact `true` turns it on.
pub const KEY_ENABLED: &str = "enabled";

/// The save to load: a `.sl2` file, or a `.zip`/`.7z`/`.rar` containing one.
pub const KEY_PATH: &str = "path";

/// Directory beside the executable that the resolved save is written into.
///
/// It is REWRITTEN ON EVERY LAUNCH, which is the point rather than an oversight: `path` names a
/// read-only source, so "start from this save" is what pointing at one means. Progress made in a
/// redirected run lives here and does not survive the next launch.
pub const STAGING_DIR_NAME: &str = "ds2-save-staging";

/// `[save_redirect]`, resolved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveRedirectConfig {
    /// Whether to install the detour with a replacement armed.
    pub enabled: bool,
    /// The replacement directory, verbatim from the file. `None` when the key is absent or empty.
    pub path: Option<String>,
}

impl SaveRedirectConfig {
    /// Read the section. A missing file, a missing key, or anything but `true` means off.
    pub fn load() -> Self {
        let Some(path) = config_file_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let parsed = KeyValues::parse(&text);
        // Only an exact `true` turns it on. A feature that moves the save directory is one where a
        // typo must mean "did nothing", never "wrote somewhere else".
        let enabled = matches!(
            parsed
                .get(CONFIG_SECTION, KEY_ENABLED)
                .map(|raw| raw.trim().trim_matches('"')),
            Some("true")
        );
        let path = parsed
            .get(CONFIG_SECTION, KEY_PATH)
            .map(|raw| raw.trim().trim_matches('"').to_owned())
            // A TOML basic string escapes its backslashes, and a Windows path is mostly
            // backslashes. Undo that here rather than making the file's author choose between a
            // path this can read and a path TOML considers valid.
            .map(|raw| raw.replace("\\\\", "\\"))
            .filter(|raw| !raw.is_empty());
        Self { enabled, path }
    }

    /// Whether the pair is coherent enough to act on. `enabled` with no `path` is not.
    pub fn armable(&self) -> bool {
        self.enabled && self.path.is_some()
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_ENABLED}={} {KEY_PATH}={}",
            ds2_save_redirect::LOG_PREFIX,
            self.enabled,
            self.path.as_deref().unwrap_or("<unset>")
        )
    }
}
