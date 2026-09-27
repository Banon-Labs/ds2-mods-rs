//! Two pause-menu rows that move a save between DARK SOULS II and a file the player picks.
//!
//! | row | what one press does | when it takes effect |
//! |---|---|---|
//! | **Load Character from File** | stages the pick, returns to the title, and opens the game's own character list for it | the same session, once you choose a character |
//! | **Save Game to File** | asks the game to save, then copies the container out | a few frames later |
//!
//! Both open an in-game panel, `picker`: drives, folders, saves with their characters and levels, a
//! typed path with folder completion, and for loading a character stage that loads the one chosen.
//! It is `../er-mods-rs`'s in-game picker rebuilt on this engine -- an imgui panel through
//! `ds2-overlay` rather than a Scaleform movie -- over the host-tested `ds2-save-picker-core` model.
//! `[save_picker] os_native = true` puts the OS file dialog back (`dialog`), and so does a session
//! whose panel could not install.
//!
//! # The asymmetry is the point
//!
//! One of these rows can destroy something and the other cannot, and they are written accordingly.
//!
//! * [`import`] writes no save. It writes one text file and the staging it triggers next launch copies
//!   the picked file, so the player's container and the donor's file both survive untouched.
//! * [`export`] writes a file the player named, which may already exist. So it refuses to write onto
//!   the live container, the dialog asks before replacing anything, and the copy waits for the game to
//!   finish writing rather than racing it.
//!
//! # The load row no longer restarts the game, and [`swap`] is why
//!
//! It used to, and the reason it had to was a misreading rather than a limit of the engine: DS2
//! saves on the way out of a game, so a session that re-points its save directory and then leaves
//! writes the character it was playing over the staged copy. That is true, and it stops mattering
//! once the save side and the load side are separable -- which they are, because `SLSaveSession`
//! and `SLLoadSession` each override the directory virtual and each is reached only through its own
//! vtable. [`swap`] arms one side at a time, and the restart is gone.
//!
//! The restart route is still in [`import`], reached only when the title flow is not hooked. A row
//! that can load a save slowly is worth more than a row that reports a missing detour.
//!
//! # The character is chosen in the panel, and the game's list loads it
//!
//! The panel reads each container's ten slots itself, so the player picks a character before the
//! game is left. The swap then opens the title's own LOAD GAME list for the staged container and
//! `ds2-continue` loads that slot from it, past the game's own occupancy and ownership checks. An
//! archive's characters cannot be read until staging unwraps it, so picking one opens the game's
//! list for the player to choose from, as the OS dialog route always did.

// DEBT: ds2-mods-rs-24r -- not debt to be paid: this crate ships as a Windows DLL and the
// attribute is what keeps its Rust half parseable on the host, so the game-free tests below it
// can run at all. The issue is the standing record of that decision.
#![cfg_attr(not(windows), allow(unused))]

/// What every line this crate writes begins with, so its lines can be grepped out of the shared log.
pub const LOG_PREFIX: &str = "ds2-save-file:";

/// How the two rows choose a file: `[save_picker]` in `ds2-mods.toml`, read by the loader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerSettings {
    /// Open the Windows file dialog instead of the in-game panel.
    pub os_native: bool,
    /// Where the panel opens, as the config spelled it. `None` means `~/Downloads`.
    pub start_dir: Option<String>,
    /// Whether a pick moves where the panel opens next time.
    pub remember_dir: bool,
}

impl Default for PickerSettings {
    fn default() -> Self {
        Self {
            os_native: false,
            start_dir: None,
            remember_dir: true,
        }
    }
}

#[cfg(windows)]
mod dialog;
#[cfg(windows)]
pub mod export;
#[cfg(windows)]
mod game;
#[cfg(windows)]
pub mod import;
#[cfg(windows)]
mod picker;
#[cfg(windows)]
mod picker_input;
#[cfg(windows)]
pub mod swap;

#[cfg(windows)]
pub use import::take_handoff;
#[cfg(windows)]
pub use install::{LogFn, register_export_row, register_import_row, set_logger};
#[cfg(windows)]
pub use picker::{configure as configure_picker, install as install_picker_panel};

#[cfg(windows)]
pub(crate) use install::log_line;
#[cfg(windows)]
pub(crate) use install::registered_import_row;

#[cfg(windows)]
mod install {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use crate::LOG_PREFIX;

    /// The loader's log sink. Same shape as `ds2-menu-row`'s.
    pub type LogFn = fn(std::fmt::Arguments<'_>);

    static LOGGER: AtomicUsize = AtomicUsize::new(0);

    /// The import row's registry index, so it can relabel itself once a pick is recorded.
    ///
    /// `usize::MAX` until registration succeeds, which is a value no registry index can be -- a
    /// caption written before registration would otherwise land on row zero, which belongs to
    /// somebody else.
    static IMPORT_ROW: AtomicUsize = AtomicUsize::new(usize::MAX);

    /// Whether the export row's per-frame tick has been registered.
    ///
    /// ONE PER ROW, because both rows now have a second phase: the export waits for the save before
    /// it copies, and the import waits for the save before it quits. Latched so a row registered
    /// twice does not spend two of the tick registry's slots.
    static EXPORT_TICK_ADDED: AtomicBool = AtomicBool::new(false);

    /// Point this crate's logging at the loader's log file.
    ///
    /// Separate from the `register_*` calls, which also set it, because [`crate::take_handoff`] runs
    /// at ATTACH -- long before any row is registered -- and a handoff consumed with no sink is a
    /// save swapped with nothing in the log to say so.
    pub fn set_logger(logger: LogFn) {
        LOGGER.store(logger as usize, Ordering::Release);
    }

    pub(crate) fn log_line(args: std::fmt::Arguments<'_>) {
        let raw = LOGGER.load(Ordering::Acquire);
        if raw != 0 {
            // SAFETY: `raw` is only ever a `LogFn` stored by a `register_*` call below.
            let logger: LogFn = unsafe { std::mem::transmute::<usize, LogFn>(raw) };
            logger(args);
        }
    }

    /// The import row, if one was registered.
    pub(crate) fn registered_import_row() -> Option<ds2_menu_row::RowId> {
        let slot = IMPORT_ROW.load(Ordering::Acquire);
        (slot != usize::MAX).then_some(ds2_menu_row::RowId(slot))
    }

    /// Green, because the two rows next to it are red (quit) and blue (load from URL).
    ///
    /// Every added row wears the same glyph -- [`ds2_rva::FLO_QUIT_ICON_DEFINITION`], the Quit Game
    /// icon on its own -- because there are ten icons in this document and all ten already mean
    /// something. The tint is the only thing telling them apart, so no two may share one.
    const IMPORT_HUE: [u8; 3] = [0x78, 0xff, 0x96];

    /// Amber, and the fourth distinct hue among the added rows.
    const EXPORT_HUE: [u8; 3] = [0xff, 0xc8, 0x50];

    /// Register **Load Character from File**. Call BEFORE `ds2_menu_row::install`, which seals the
    /// registry.
    ///
    /// Returns whatever [`ds2_menu_row::add_row`] said, so the caller can log a refusal in its own
    /// voice -- including the `TabFull` that comes back when the tab's five item slots are spoken for.
    /// # Errors
    ///
    /// Whatever [`ds2_menu_row::add_row`] said -- `TabFull` when the tab is spoken for,
    /// `AlreadyInstalled` when the registry is sealed.
    pub fn register_import_row(
        logger: LogFn,
    ) -> Result<ds2_menu_row::RowId, ds2_menu_row::AddRowError> {
        LOGGER.store(logger as usize, Ordering::Release);
        let registered = ds2_menu_row::add_row(ds2_menu_row::RowSpec {
            tab: ds2_menu_row::Tab::Quit,
            caption: crate::import::ROW_CAPTION,
            icon: ds2_rva::FLO_QUIT_ICON_DEFINITION,
            tint: Some(ds2_menu_row::Tint {
                rgb: IMPORT_HUE,
                strength: ds2_rva::FLO_ADDED_ROW_TINT_STRENGTH,
            }),
            on_confirm: crate::import::load_from_file,
        });
        if let Ok(id) = registered {
            IMPORT_ROW.store(id.0, Ordering::Release);
            // THE GAME-THREAD HALF. The press records the pick and asks the game to save; the quit
            // that follows has to wait for that save to land, and waiting is what a tick is for.
            // Without it the row would record a pick and never quit, which is the old behaviour it
            // exists to replace.
            if !ds2_menu_row::add_tick(crate::import::tick) {
                log_line(format_args!(
                    "{LOG_PREFIX} NO TICK -- a pick will be recorded and the game will never quit"
                ));
            }
        }
        registered
    }

    /// Register **Save Game to File**. Call BEFORE `ds2_menu_row::install`.
    ///
    /// Also claims the per-frame tick this row's second phase needs. A row registered without a tick
    /// would ask the game to save and then never copy anything, so the failure is logged rather than
    /// left for someone to find in an empty destination folder.
    /// # Errors
    ///
    /// Whatever [`ds2_menu_row::add_row`] said, or the same refusal for the per-frame tick this
    /// row also needs.
    pub fn register_export_row(
        logger: LogFn,
    ) -> Result<ds2_menu_row::RowId, ds2_menu_row::AddRowError> {
        LOGGER.store(logger as usize, Ordering::Release);
        let registered = ds2_menu_row::add_row(ds2_menu_row::RowSpec {
            tab: ds2_menu_row::Tab::Quit,
            caption: crate::export::ROW_CAPTION,
            icon: ds2_rva::FLO_QUIT_ICON_DEFINITION,
            tint: Some(ds2_menu_row::Tint {
                rgb: EXPORT_HUE,
                strength: ds2_rva::FLO_ADDED_ROW_TINT_STRENGTH,
            }),
            on_confirm: crate::export::save_to_file,
        });
        if registered.is_ok() && !EXPORT_TICK_ADDED.swap(true, Ordering::AcqRel) {
            // THE GAME-THREAD HALF. `RequestSave` only asks; the copy happens once the game has
            // actually written the file, which is frames later and therefore here.
            if !ds2_menu_row::add_tick(crate::export::tick) {
                log_line(format_args!(
                    "{LOG_PREFIX} NO TICK -- an export will be requested and never written"
                ));
            }
        }
        registered
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// No two added rows may share a tint, because every one of them wears the same glyph.
        #[test]
        fn the_two_hues_differ_from_each_other_and_from_the_shipped_rows() {
            assert_ne!(IMPORT_HUE, EXPORT_HUE);
            // The quit-to-desktop row's hue, from `ds2-rva`, and `ds2-build-import`'s blue.
            assert_ne!(IMPORT_HUE, ds2_rva::FLO_ADDED_ROW_HUE);
            assert_ne!(EXPORT_HUE, ds2_rva::FLO_ADDED_ROW_HUE);
            assert_ne!(IMPORT_HUE, [0x64, 0xb4, 0xff]);
            assert_ne!(EXPORT_HUE, [0x64, 0xb4, 0xff]);
        }

        /// A row id of `usize::MAX` is "not registered" and must never be handed out as row zero.
        #[test]
        fn an_unregistered_row_has_no_id() {
            assert_eq!(IMPORT_ROW.load(Ordering::Acquire), usize::MAX);
            assert!(registered_import_row().is_none());
        }
    }
}
