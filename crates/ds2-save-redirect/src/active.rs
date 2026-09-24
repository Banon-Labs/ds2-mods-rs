//! The container name the running game is actually opening.
//!
//! [`SAVE_FILE_NAME`](crate::SAVE_FILE_NAME) is the only name unmodded SOTFS builds, and every
//! save feature in this repo was written against it. With DARK SOULS II Seamless Co-op loaded that
//! name is wrong: its `ds2sc_settings.ini` carries `save_file_extension = co2`, the game opens
//! `DS2SOFS0000.co2`, and a donor staged under the `.sl2` name is a file it never asks for. The
//! load-from-file row would then report success over a save nobody read.
//!
//! Resolved once and cached. The two files that decide it -- this repo's `ds2-mods.toml` and the
//! other mod's settings -- are read at startup and cannot change under a running game, because the
//! other mod's DLL is loaded once, from `ds2-loader`'s attach, and its own settings are read then
//! too.

use std::sync::OnceLock;

use ds2_seamless::{SeamlessConfig, VANILLA_SAVE_EXTENSION, save_file_name_for_extension};

/// This repo's own config file, which is where `[seamless]` lives.
const CONFIG_FILE_NAME: &str = "ds2-mods.toml";

/// Resolved on first call, then handed back.
static ACTIVE: OnceLock<String> = OnceLock::new();

/// `DS2SOFS0000.sl2`, or `DS2SOFS0000.<ext>` when Seamless Co-op is armed and names one.
///
/// Falls back to the vanilla name whenever anything is missing or unusable -- no game directory,
/// no config file, the mod disarmed, no settings file, or a value that is not an extension. The
/// fallback is not a guess: with the other mod absent or silent, the game builds the name it has
/// always built.
pub fn active_save_file_name() -> &'static str {
    ACTIVE.get_or_init(|| {
        let Some(game_dir) = ds2_game_base::log::game_directory_path() else {
            return save_file_name_for_extension(VANILLA_SAVE_EXTENSION);
        };
        let config = SeamlessConfig::from_game_dir(&game_dir, CONFIG_FILE_NAME);
        ds2_seamless::save_file_name(&game_dir, &config)
    })
}

/// Whether a name inside an archive is the save container, under either extension.
///
/// Tolerant on input and exact on output, which is the rule this module exists to keep: a donor
/// exported from an unmodded game is `DS2SOFS0000.sl2`, its bytes are the same bytes, and refusing
/// it inside a co-op session would be a rule with nothing behind it. What gets written is always
/// [`active_save_file_name`].
pub fn is_save_container_name(name: &str) -> bool {
    let active = active_save_file_name();
    let extension = active
        .rsplit_once('.')
        .map_or(VANILLA_SAVE_EXTENSION, |(_, ext)| ext);
    ds2_seamless::is_save_container_name(name, extension)
}
