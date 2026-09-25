//! What this process needs to know about DARK SOULS II Seamless Co-op, which it does not ship.
//!
//! # The two questions
//!
//! 1. Did the player ask for it, and where did they install it? That is `[seamless]` in this
//!    repo's own `ds2-mods.toml`, and it is opt-in by an exact `true`.
//! 2. What is the save file called while it is running? Seamless renames the container --
//!    `save_file_extension = co2` in its own `ds2sc_settings.ini` -- so a co-op session's
//!    character lives in `DS2SOFS0000.co2` and never touches `DS2SOFS0000.sl2`.
//!
//! The second question is the one that breaks things quietly. Every save feature in this repo was
//! written against the only name SOTFS builds, and under Seamless that name is wrong: a staged
//! donor file written as `.sl2` is invisible to a game asking for `.co2`, and the load-from-file
//! row would report success over a file the game never opens. So the name is resolved here, once,
//! from the two files that decide it.
//!
//! # Where the load happens, and why it is not here
//!
//! Loading the other mod's DLL is a `LoadLibraryW` in `ds2-loader`; this crate only says whether
//! and from where. Two facts about the timing are recorded here because they were measured from
//! the shipped launcher and they explain the call site:
//!
//! - `ds2sc_launcher.exe` calls `CreateProcessA` with `dwCreationFlags = 4`
//!   (`CREATE_SUSPENDED`; `movl $0x4,0x28(%rsp)` at `0x1400013b5`, immediately before the call at
//!   `0x1400013d4`), then `VirtualAllocEx` / `WriteProcessMemory` / `CreateRemoteThread` on
//!   `LoadLibraryA` with `SeamlessCoop//ds2sc.dll`, then `ResumeThread`. So the other mod is
//!   loaded before the game executes a single instruction.
//! - A suspended process's initial thread is parked in `LdrInitializeThunk`. The injected thread
//!   runs process init -- static imports and their `DllMain`s -- and only then its start routine.
//!   Static `DllMain`s run before the image's TLS callbacks, which run before its entry point. A
//!   mod that anchors its start on a TLS callback therefore gets one in the launcher's path and
//!   gets nothing at all if it is loaded after the entry point is already reached.
//!
//! Neither fact says what Seamless does with that window. Its DLL is packed -- one named
//! `kernel32` import, `GetModuleHandleA`, and padded junk branches through the entry thunk -- so
//! the question of what it anchors on has no cheap static answer, and this crate does not claim
//! one.

use std::path::Path;

use ds2_hotkey_config::kv::KeyValues;

/// The section this crate reads out of `ds2-mods.toml`. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "seamless";

/// Whether to load the second mod at all.
pub const KEY_ENABLED: &str = "enabled";

/// Where its DLL is, relative to the game directory.
pub const KEY_DLL: &str = "dll";

/// Log prefix, in the shape every other feature here uses.
pub const LOG_PREFIX: &str = "ds2-seamless:";

/// Where Seamless Co-op's own launcher looks for it, so this is where it is expected to be.
pub const DEFAULT_DLL: &str = "SeamlessCoop/ds2sc.dll";

/// The settings file Seamless keeps next to its DLL.
pub const SETTINGS_FILE_NAME: &str = "ds2sc_settings.ini";

/// The key in that file that renames the save container.
pub const KEY_SAVE_FILE_EXTENSION: &str = "save_file_extension";

/// The one container name SOTFS builds, without its extension.
pub const SAVE_FILE_STEM: &str = "DS2SOFS0000";

/// The extension the unmodded game uses.
pub const VANILLA_SAVE_EXTENSION: &str = "sl2";

/// Seamless's own default, and what its shipped settings file says.
pub const DEFAULT_SAVE_EXTENSION: &str = "co2";

/// The ceiling its settings file documents for the extension: "limit = 120".
pub const MAX_SAVE_EXTENSION_LEN: usize = 120;

/// `[seamless]`, resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeamlessConfig {
    /// Whether this workspace should expect the second mod to be present at all.
    pub enabled: bool,
    /// The DLL name the second mod ships under, as its own settings file spells it.
    pub dll: String,
}

impl Default for SeamlessConfig {
    /// Off. Every other feature in this repo is code it wrote and can answer for. This one loads
    /// a binary from somewhere else into the game, and a default that does that without being
    /// asked is a default nobody consented to.
    fn default() -> Self {
        Self {
            enabled: false,
            dll: DEFAULT_DLL.to_string(),
        }
    }
}

impl SeamlessConfig {
    /// Parse the section out of `ds2-mods.toml`'s text.
    ///
    /// Only an exact `true` arms it, which is the opposite of how the shipped features read their
    /// switch and deliberately so: a typo here leaves a foreign DLL unloaded rather than loaded.
    pub fn parse(text: &str) -> Self {
        let parsed = KeyValues::parse(text);
        let enabled =
            matches!(parsed.get(CONFIG_SECTION, KEY_ENABLED).map(unquote), Some(v) if v == "true");
        let dll = parsed
            .get(CONFIG_SECTION, KEY_DLL)
            .map(unquote)
            .filter(|raw| !raw.is_empty())
            .unwrap_or_else(|| DEFAULT_DLL.to_string());
        Self { enabled, dll }
    }

    /// Read `<game_dir>/ds2-mods.toml`. A missing file means [`Default`].
    pub fn from_game_dir(game_dir: &Path, config_file_name: &str) -> Self {
        std::fs::read_to_string(game_dir.join(config_file_name))
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    /// The directory the other mod keeps its own files in, derived from the DLL path it was given.
    ///
    /// Relative to the game directory, and empty when the DLL was configured at the top level.
    pub fn mod_directory(&self) -> String {
        let normalised = self.dll.replace('\\', "/");
        match normalised.rsplit_once('/') {
            Some((parent, _)) => parent.to_string(),
            None => String::new(),
        }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{LOG_PREFIX} config [{CONFIG_SECTION}] {KEY_ENABLED}={} {KEY_DLL}={}",
            self.enabled, self.dll
        )
    }
}

/// Strip the whitespace and the optional quotes a `toml` value may carry.
fn unquote(raw: &str) -> String {
    raw.trim().trim_matches('"').to_string()
}

/// Pull `save_file_extension` out of Seamless's settings file.
///
/// Its own parser is not this one, so this is deliberately forgiving in the same directions the
/// shipped file needs and strict in the one that matters. Comments are `;` (its style) or `#`;
/// section headers are ignored, because the key is unique in the file and matching `[SAVE]`
/// case-exactly would break on a player who retyped it.
///
/// Returns `None` when the key is absent, empty, or names something that is not an extension --
/// anything with a path separator, a dot, or a character the game cannot put in a file name. A
/// refused value is not a small mistake: it would be pasted into a path and opened.
pub fn parse_save_file_extension(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with(';') || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case(KEY_SAVE_FILE_EXTENSION) {
            continue;
        }
        let value = value.trim().trim_matches('"');
        if value.is_empty() || value.len() > MAX_SAVE_EXTENSION_LEN {
            return None;
        }
        if !value.chars().all(|c| c.is_ascii_alphanumeric()) {
            return None;
        }
        return Some(value.to_string());
    }
    None
}

/// The extension the running game will actually use for its save container.
///
/// [`VANILLA_SAVE_EXTENSION`] whenever Seamless is not armed, its settings file is missing, or
/// that file names nothing usable. The fallback is not a guess: with the other mod absent or
/// silent, the game builds the name it has always built.
pub fn save_file_extension(game_dir: &Path, config: &SeamlessConfig) -> String {
    if !config.enabled {
        return VANILLA_SAVE_EXTENSION.to_string();
    }
    let directory = config.mod_directory();
    let settings = if directory.is_empty() {
        game_dir.join(SETTINGS_FILE_NAME)
    } else {
        game_dir
            .join(directory.replace('/', std::path::MAIN_SEPARATOR_STR))
            .join(SETTINGS_FILE_NAME)
    };
    std::fs::read_to_string(settings)
        .ok()
        .and_then(|text| parse_save_file_extension(&text))
        .unwrap_or_else(|| VANILLA_SAVE_EXTENSION.to_string())
}

/// `DS2SOFS0000.<ext>` -- the file name the game is opening on this run.
pub fn save_file_name_for_extension(extension: &str) -> String {
    format!("{SAVE_FILE_STEM}.{extension}")
}

/// [`save_file_name_for_extension`] over [`save_file_extension`], for a caller that has the game
/// directory and wants the answer.
pub fn save_file_name(game_dir: &Path, config: &SeamlessConfig) -> String {
    save_file_name_for_extension(&save_file_extension(game_dir, config))
}

/// Whether a file name is the save container under either the vanilla or the active extension.
///
/// Input is tolerant on purpose and output is not. A donor save exported from an unmodded game is
/// `DS2SOFS0000.sl2`, its bytes are the same bytes, and refusing to load it into a co-op session
/// because of its extension would be a rule with nothing behind it. What gets written is always
/// the active name.
pub fn is_save_container_name(name: &str, active_extension: &str) -> bool {
    let vanilla = save_file_name_for_extension(VANILLA_SAVE_EXTENSION);
    name.eq_ignore_ascii_case(&vanilla)
        || name.eq_ignore_ascii_case(&save_file_name_for_extension(active_extension))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The switch is off unless the file says `true`, and a typo does not load a foreign binary.
    #[test]
    fn only_an_exact_true_arms_it() {
        assert!(!SeamlessConfig::default().enabled);
        for raw in ["", "yes", "True", "1", "ture", "false"] {
            let text = format!("[{CONFIG_SECTION}]\n{KEY_ENABLED} = {raw}\n");
            assert!(
                !SeamlessConfig::parse(&text).enabled,
                "{raw:?} must not arm a foreign DLL load"
            );
        }
        let armed = SeamlessConfig::parse(&format!("[{CONFIG_SECTION}]\n{KEY_ENABLED} = true\n"));
        assert!(armed.enabled);
        assert_eq!(armed.dll, DEFAULT_DLL);
    }

    /// The default path is the one Seamless's own launcher injects.
    #[test]
    fn the_default_path_is_the_launchers() {
        assert_eq!(SeamlessConfig::default().dll, DEFAULT_DLL);
        assert!(
            DEFAULT_DLL.ends_with("ds2sc.dll"),
            "the launcher's string is `SeamlessCoop//ds2sc.dll`"
        );
    }

    /// The settings file sits next to the DLL, wherever the player put it.
    #[test]
    fn the_mod_directory_follows_the_dll() {
        let config = SeamlessConfig::parse(&format!(
            "[{CONFIG_SECTION}]\n{KEY_DLL} = \"mods/coop/ds2sc.dll\"\n"
        ));
        assert_eq!(config.mod_directory(), "mods/coop");
        let backslashes = SeamlessConfig::parse(&format!(
            "[{CONFIG_SECTION}]\n{KEY_DLL} = mods\\coop\\x.dll\n"
        ));
        assert_eq!(backslashes.mod_directory(), "mods/coop");
        let top_level = SeamlessConfig::parse(&format!("[{CONFIG_SECTION}]\n{KEY_DLL} = x.dll\n"));
        assert_eq!(top_level.mod_directory(), "");
    }

    /// The shipped settings file, verbatim in the shape that matters: `;` comments, a `[SAVE]`
    /// header, and lines above it that contain an `=` inside their prose.
    #[test]
    fn the_extension_is_read_from_the_shipped_file() {
        let text = "\
[PASSWORD]

; Your session password. You must have the same password as people you want to connect to
cooppassword =

[GAMEPLAY]

; Whether to allow invaders (hostile players) into your world. 1 = ON | 0 = OFF
allow_invaders = 1

[SAVE]

; Your save file extension (in the vanilla game this is .sl2). Use any alphanumeric characters (limit = 120)
save_file_extension = co2
";
        assert_eq!(
            parse_save_file_extension(text).as_deref(),
            Some(DEFAULT_SAVE_EXTENSION)
        );
    }

    /// A value that is not an extension is refused rather than pasted into a path.
    #[test]
    fn a_value_that_is_not_an_extension_is_refused() {
        for raw in [
            "",
            "   ",
            "../../etc",
            "co2/../sl2",
            "co 2",
            ".co2",
            "co2.bak",
            "co\\2",
        ] {
            let text = format!("{KEY_SAVE_FILE_EXTENSION} = {raw}\n");
            assert_eq!(
                parse_save_file_extension(&text),
                None,
                "{raw:?} must not reach a file path"
            );
        }
        let long = "a".repeat(MAX_SAVE_EXTENSION_LEN + 1);
        assert_eq!(
            parse_save_file_extension(&format!("{KEY_SAVE_FILE_EXTENSION} = {long}\n")),
            None
        );
    }

    /// With the mod disarmed the name is the one the game has always built, whatever the other
    /// mod's settings file happens to say.
    #[test]
    fn a_disarmed_mod_leaves_the_vanilla_name() {
        let config = SeamlessConfig::default();
        let extension = save_file_extension(Path::new("/nonexistent/game"), &config);
        assert_eq!(extension, VANILLA_SAVE_EXTENSION);
        assert_eq!(
            save_file_name_for_extension(&extension),
            "DS2SOFS0000.sl2",
            "the only container name SOTFS builds"
        );
    }

    /// Armed but with no settings file to read is still the vanilla name -- not a guess at `co2`.
    #[test]
    fn an_armed_mod_with_no_settings_file_does_not_guess() {
        let config = SeamlessConfig {
            enabled: true,
            dll: DEFAULT_DLL.to_string(),
        };
        assert_eq!(
            save_file_extension(Path::new("/nonexistent/game"), &config),
            VANILLA_SAVE_EXTENSION
        );
    }

    /// A donor exported from an unmodded game still counts as a save inside a co-op session.
    #[test]
    fn a_vanilla_donor_is_accepted_under_the_active_extension() {
        assert!(is_save_container_name("DS2SOFS0000.sl2", "co2"));
        assert!(is_save_container_name("DS2SOFS0000.co2", "co2"));
        assert!(is_save_container_name("ds2sofs0000.CO2", "co2"));
        assert!(!is_save_container_name("DS2SOFS0001.co2", "co2"));
        assert!(!is_save_container_name("DS2SOFS0000.bak", "co2"));
    }
}
