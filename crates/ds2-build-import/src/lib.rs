//! A pause-menu row that loads a DARK SOULS II build from a soulsplanner link.
//!
//! Press **Load from URL** on the quit tab and Steam draws a text field, prefilled with
//! `https://soulsplanner.com/darksouls2/`, that takes the build id after the slash. What comes back
//! is validated, fetched and read.
//!
//! # It borrows the game's Steam keyboard rather than drawing its own
//!
//! DARK SOULS II already asks Steam for a text field -- that is how character naming works -- and
//! this crate asks for the same thing at a newer interface version, because the game's own version
//! cannot prefill. See [`steam`] for the version bump and, more importantly, for the interlock: the
//! game's dismissal listener does not check whether the game asked for the keyboard, so a session
//! opened here is a session the game reacts to.
//!
//! The alternative was drawing a box in the `.flo` and collecting `WM_CHAR` from the message
//! enqueue at `0x140aef890`. That path is real and mapped, and it is what this becomes if the
//! overlay turns out to be unavailable -- `ds2_build_import_core::field` is its editing model,
//! already written and tested. It is not the first choice because it needs a second hook for
//! suppression that nobody has established yet: the message queue is **not** how this game reads
//! the keyboard (it uses DirectInput8), so refusing to enqueue a key does not stop it reaching the
//! player's character.
//!
//! # What it does not do
//!
//! **It does not apply the build.** It reads one and writes it to the log. Putting a build on a
//! character means writing equipment, inventory and nine stats, and none of those writers exist for
//! this game yet. Saying so in the log beats a row that looks like it worked.
//!
//! # The three failure modes worth knowing before reading the log
//!
//! | log line | what happened |
//! |---|---|
//! | `no field: the Steam overlay is disabled` | nothing to do with this mod -- no overlay, no field |
//! | `no field: the game's keyboard is busy` | the game owns a session; refused rather than stolen |
//! | `rejected "...": Drop the #` | the fragment form, refused before the request went out |

#![cfg_attr(not(windows), allow(unused))]

/// What every line this crate writes begins with.
pub const LOG_PREFIX: &str = "ds2-build-import:";

#[cfg(windows)]
mod clipboard;
#[cfg(windows)]
mod flow;
#[cfg(windows)]
mod game;
#[cfg(windows)]
mod save;
#[cfg(windows)]
mod steam;

/// Turn a build's named gear into the entries the game's grant function takes.
///
/// **Weapons arrive as PAIRS** -- soulsplanner emits `name, infusion, name, infusion, ...` across
/// the six weapon slots -- so they are walked two at a time and everything else one at a time.
/// Reading that list as a flat run of names puts `Dark` and `Bleed` through the item lookup, where
/// they resolve to nothing and read as a broken catalogue.
///
/// Anything that does not resolve is SKIPPED and logged rather than failing the whole build: one
/// unrecognised name should cost the player that item, not the other thirty.
#[cfg(windows)]
pub(crate) fn build_items(build: &ds2_build_import_core::Build) -> Vec<game::ItemSpawn> {
    use ds2_build_import_core::{Infusion, ItemError, id_for, is_empty_slot};

    /// `-1` is the game's own "maximum" for durability and reinforcement.
    const MAX: i32 = -1;

    let mut out = Vec::new();
    let mut push = |name: &str, infusion: Infusion| match id_for(name) {
        Ok(item_id) => out.push(game::ItemSpawn {
            unknown: 0,
            item_id,
            durability: f32::from_bits(MAX as u32),
            quantity: 1,
            reinforce: 0,
            infusion: infusion.byte(),
        }),
        Err(ItemError::EmptySlot) => {}
        Err(error) => log_line(format_args!("{LOG_PREFIX} skipping {name:?}: {error}")),
    };

    // Weapons: (name, infusion) pairs.
    for pair in build.weapons.chunks(2) {
        let [name, infusion] = pair else { continue };
        if is_empty_slot(name) {
            continue;
        }
        push(
            name,
            Infusion::from_name(infusion).unwrap_or(Infusion::None),
        );
    }
    // Everything else is one name per slot, uninfused.
    for name in build
        .armor
        .iter()
        .chain(&build.rings)
        .chain(&build.spells)
        .chain(&build.items)
    {
        push(name, Infusion::None);
    }
    out
}

#[cfg(windows)]
pub use install::{LogFn, register};

#[cfg(windows)]
mod install {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use ds2_build_import_core::ROW_CAPTION;

    use crate::LOG_PREFIX;

    /// The loader's log sink. Same shape as `ds2-menu-row`'s.
    pub type LogFn = fn(std::fmt::Arguments<'_>);

    static LOGGER: AtomicUsize = AtomicUsize::new(0);

    /// A session is open. The row is inert until it finishes.
    ///
    /// Not because two fields would crash -- [`crate::steam::KeyboardClaim`] would refuse the
    /// second one anyway -- but because the refusal would be logged as "the game's keyboard is
    /// busy", which points at the wrong culprit when the thing holding it is us.
    static SESSION_OPEN: AtomicBool = AtomicBool::new(false);

    /// The row this crate registered, so it can be told to change its own caption.
    ///
    /// `usize::MAX` until [`register`] succeeds, which is a value no registry index can be -- a
    /// caption written before registration would otherwise land on row zero, which belongs to
    /// somebody else.
    static ROW: AtomicUsize = AtomicUsize::new(usize::MAX);

    /// The row to write captions to.
    pub(crate) fn row_id() -> ds2_menu_row::RowId {
        ds2_menu_row::RowId(ROW.load(Ordering::Acquire))
    }

    /// Register the row and point this crate's logging at the loader's file.
    ///
    /// Call BEFORE `ds2_menu_row::install`, which seals the registry. Returns whatever
    /// [`ds2_menu_row::add_row`] said, so the caller can log a refusal in its own voice.
    pub fn register(logger: LogFn) -> Result<ds2_menu_row::RowId, ds2_menu_row::AddRowError> {
        LOGGER.store(logger as usize, Ordering::Release);
        let registered = ds2_menu_row::add_row(ds2_menu_row::RowSpec {
            tab: ds2_menu_row::Tab::Quit,
            caption: ROW_CAPTION,
            icon: ds2_rva::FLO_QUIT_ICON_DEFINITION,
            // A different hue from the quit row's, because the two rows sit next to each other
            // wearing the same glyph -- there are ten icons in this document and all ten already
            // mean something. The strength is the one the ramp settled on; see
            // `ds2_rva::FLO_ADDED_ROW_TINT_STRENGTH`.
            tint: Some(ds2_menu_row::Tint {
                rgb: [0x64, 0xb4, 0xff],
                strength: ds2_rva::FLO_ADDED_ROW_TINT_STRENGTH,
            }),
            on_confirm: open_field,
        });
        if let Ok(id) = registered {
            ROW.store(id.0, Ordering::Release);
            // THE GAME-THREAD HALF. Everything this row does that touches the game happens here,
            // because the row's own confirm happens once and the work answers later.
            if !ds2_menu_row::add_tick(crate::flow::apply_tick) {
                log_line(format_args!(
                    "{LOG_PREFIX} NO TICK -- a fetched build will be recorded but never applied"
                ));
            }
        }
        registered
    }

    /// What pressing the row does. **Runs on the game thread, with the menu still up.**
    ///
    /// THE FIELD IS OPENED HERE, not on the worker. The game makes its own `ShowGamepadTextInput`
    /// call from its own frame loop, and the first run of this crate called it from a worker and got
    /// `false` back -- so the call moved to the thread the game uses, to take that difference off
    /// the table. Only the WAIT is handed off, because the wait is unbounded and the fetch that
    /// follows it is a blocking TLS handshake.
    fn open_field() {
        if SESSION_OPEN.swap(true, Ordering::AcqRel) {
            return log_line(format_args!("{LOG_PREFIX} a field is already open"));
        }
        let Some(job) = crate::flow::begin_session() else {
            SESSION_OPEN.store(false, Ordering::Release);
            return;
        };
        // A failed spawn must clear the flag AND drop the job, or the row is dead for the rest of
        // the process and any keyboard claim inside the job stays wedged with it.
        if std::thread::Builder::new()
            .name("ds2-build-import".to_owned())
            .spawn(move || {
                crate::flow::finish_session(job);
                SESSION_OPEN.store(false, Ordering::Release);
            })
            .is_err()
        {
            SESSION_OPEN.store(false, Ordering::Release);
            log_line(format_args!("{LOG_PREFIX} could not start a worker thread"));
        }
    }

    /// Write one line to the loader's log, if a logger was installed.
    pub(crate) fn log_line(args: std::fmt::Arguments<'_>) {
        let logger = LOGGER.load(Ordering::Acquire);
        if logger != 0 {
            // SAFETY: the only writer is `register`, which stores a `LogFn` cast from a real
            // function item, and it is stored before any row can be pressed.
            let logger: LogFn = unsafe { std::mem::transmute(logger) };
            logger(args);
        }
    }
}

#[cfg(windows)]
pub(crate) use install::{log_line, row_id};
