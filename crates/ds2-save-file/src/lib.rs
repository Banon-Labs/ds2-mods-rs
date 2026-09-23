//! Two pause-menu rows that move a save between DARK SOULS II and a file the player picks.
//!
//! | row | what one press does | when it takes effect |
//! |---|---|---|
//! | **Load Character from File** | records the pick, saves your character, then QUITS the game | the next launch, which the row has already asked for |
//! | **Save Game to File** | asks the game to save, then copies the container out | a few frames later |
//!
//! Both open the OS file dialog and neither draws a menu of its own. That is the port decision, and
//! it is the whole reason this crate is small: `../er-mods-rs` has both an in-game save browser and a
//! comdlg32 one, and only the second has no game coupling. See [`dialog`].
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
//! # Why one of them ends the session, and does not ask the player to
//!
//! Because DS2 saves on the way out of a game, and a mid-session redirect therefore gets overwritten
//! by the player's own character before the load can read it. [`import`] has the argument in full.
//! The restart is real and it is NOT the player's job: the row records the pick, requests the save
//! that would otherwise clobber it, waits for that save to land, and takes the game down itself. A
//! row that relabelled itself `restart to load` and stopped there was the first attempt, and it was
//! wrong -- it left the mod's last step for the player to perform by hand.
//!
//! # These two rows are not the whole feature, and the missing half is filed
//!
//! Neither row lets you choose a CHARACTER. The load row swaps a whole container, so the ten slots
//! you get are the donor file's ten, and picking among them is the game's own character list
//! rather than anything here. `../er-mods-rs` has that half in `er-save-picker-core`; what DS2
//! lacks for it is a Rust reader that can name a character -- `ds2-sl2-core` walks the BND4 entry
//! table and rebinds Steam IDs, and cannot report a slot's name or level. `scripts/ds2-sl2.py
//! --slots` already does, which is where the port starts.
//!
//! An earlier version of this section refused that port on the grounds that it meant reproducing
//! `er-quit-menu-core` wholesale. That was a mis-measurement of the wrong crate: the menu chrome
//! is what lives there, and the picker is `er-save-picker-core`, which is mostly a host-testable
//! row model over the filesystem. Size the thing being asked for, not the crate it is linked into.

#![cfg_attr(not(windows), allow(unused))]

/// What every line this crate writes begins with, so its lines can be grepped out of the shared log.
pub const LOG_PREFIX: &str = "ds2-save-file:";

#[cfg(windows)]
mod dialog;
#[cfg(windows)]
pub mod export;
#[cfg(windows)]
mod game;
#[cfg(windows)]
pub mod import;

#[cfg(windows)]
pub use import::take_handoff;
#[cfg(windows)]
pub use install::{LogFn, register_export_row, register_import_row, set_logger};

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
