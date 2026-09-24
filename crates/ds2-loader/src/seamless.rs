//! Running beside DARK SOULS II Seamless Co-op, which this repo does not ship and does not load.
//!
//! The parsing, the paths and the save-name arithmetic live in [`ds2_seamless`], which is
//! host-testable and game-free. What is here is the half that needs the running process: the game
//! directory, and the container name every save feature has to follow.
//!
//! # `[seamless] enabled` means "that mod is in this run", not "load it"
//!
//! It used to mean the second thing. This crate is inside the game process before the entry point,
//! which is the position that mod's launcher injects from, so calling `LoadLibraryW` on its DLL
//! looked like the same act with one less moving part -- `steam -applaunch` kept working, no game
//! file was renamed, and the load order of two mods was decided here rather than by a race.
//!
//! It does not work, and the failure is silent. `LoadLibraryW` returns a base, the image maps with
//! its code and its packed section committed, and the mod does nothing whatsoever: no settings are
//! read, no hooks are installed, and the game goes on opening `DS2SOFS0000.sl2` rather than the
//! `.co2` its own settings rename the container to. Four slots were run against a live game:
//!
//! | who loads it | process init | loader lock | thread | result |
//! |---|---|---|---|---|
//! | this crate, post-Arxan callback | complete | not held | game's | inert |
//! | this crate, `attach`, inline | in progress | held here | game's | inert |
//! | this crate, `attach`, own thread, entry point held | complete | not held | its own | inert |
//! | `ds2sc_launcher.exe` | complete | not held | its own | works |
//!
//! The launcher run is the control, and it was made with `[seamless]` disarmed so that nothing
//! here loaded anything. Its mechanism is `CreateProcessA` with `dwCreationFlags = 4`
//! (`CREATE_SUSPENDED`; `movl $0x4,0x28(%rsp)` at `0x1400013b5`, immediately before the call at
//! `0x1400013d4`), then `VirtualAllocEx` / `WriteProcessMemory` / `CreateRemoteThread` on
//! `LoadLibraryA` with `SeamlessCoop//ds2sc.dll`, then `ResumeThread`. That run reached the mod's
//! own `Please enter a session password` dialog, and with a password set it went on to a session
//! whose container opens as `DS2SOFS0000.co2` -- with every hook in this crate installed beside it.
//!
//! So the mod initialises perfectly well next to these hooks and next to a neutered Arxan; both
//! were live in the run that worked. What it does not survive is being loaded into a process whose
//! main thread has already run `LdrInitializeThunk`. That condition cannot be reproduced from a
//! statically imported DLL, because such a DLL's `DllMain` *is* part of that initialisation. The
//! launcher's suspended-process injection is load-bearing, and `scripts/ds2-run.py --seamless`
//! runs the launcher.
//!
//! # What the section is still read for
//!
//! The save container's name. That mod keeps its own save file -- `save_file_extension = co2` in
//! `ds2sc_settings.ini` -- so under it the game opens `DS2SOFS0000.co2`, and a staged donor or an
//! imported build written to `DS2SOFS0000.sl2` is a file the game never asks for. The row that
//! wrote it would report success over it. See [`ds2_save_redirect::active_save_file_name`].
//!
//! `[offline]` and this remain mutually exclusive in `scripts/ds2-run.py`: that section fronts the
//! socket imports, and a co-op mod under it would run, report success and never connect.

pub use ds2_seamless::{
    CONFIG_SECTION, DEFAULT_DLL, KEY_DLL, KEY_ENABLED, LOG_PREFIX, SeamlessConfig,
    save_file_extension, save_file_name,
};

use crate::arxan_probe::CONFIG_FILE_NAME;

/// The game directory, or `None` when it could not be resolved.
pub fn game_directory() -> Option<std::path::PathBuf> {
    ds2_game_base::log::game_directory_path()
}

/// Read `[seamless]` out of this repo's own config file. A missing file means the default, which
/// is off.
pub fn config() -> SeamlessConfig {
    match game_directory() {
        Some(dir) => SeamlessConfig::from_game_dir(&dir, CONFIG_FILE_NAME),
        None => SeamlessConfig::default(),
    }
}
