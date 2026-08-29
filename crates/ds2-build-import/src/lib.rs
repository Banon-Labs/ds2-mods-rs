//! A pause-menu row that loads a DARK SOULS II build from a soulsplanner link.
//!
//! Press **Load from URL** on the quit tab, and the row takes a build id from whichever of three
//! sources can supply one, fetches that build and puts it on the live character.
//!
//! # Three ways in, tried in that order
//!
//! 1. **Steam's own text field**, prefilled with `https://soulsplanner.com/darksouls2/`. Correct,
//!    and absent on a desktop Steam outside Big Picture -- measured, not assumed.
//! 2. **The clipboard**, if it holds a soulsplanner link. Copy a link in a browser, press the row.
//! 3. **Typing the id on the row itself**, which is what happens when neither of the others can
//!    supply one. See [`typed`] for why it reads ten keys and not a keyboard.
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
//! # What one press changes, and the one thing it cannot undo
//!
//! Soul memory is raised to the floor the build's level needs, THEN the nine stats are written, and
//! then the items are granted -- every one of those through a function the game already has. The
//! order is not cosmetic: DS2 matches players on soul memory, so a level that arrives before the
//! soul memory to support it is a character matched against the wrong opponents.
//!
//! **RAISING SOUL MEMORY CANNOT BE UNDONE.** The engine has no path that lowers it. A character
//! already past the floor is left alone -- nothing here ever lowers or overwrites -- but one below
//! it is moved up permanently.
//!
//! Then everything the build named is worn, held, attuned and quick-slotted through
//! `ItemInventory2::SetEquip` -- the direct sibling of the grant function, taking the same
//! receiver -- and the covenant is joined.
//!
//! **THE COVENANT ALSO CANNOT BE UNDONE**, and it is the second such thing here. Joining one marks
//! it permanently discovered on that character; leaving later changes which is current and leaves
//! the mark. Nothing is announced to the network.
//!
//! **What is still not applied**: the hand `grip` the planner records, and the `class` and `gender`
//! it names -- a character already exists by the time this runs, so those are read and ignored.
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
#[cfg(windows)]
mod typed;

/// Turn a build's named gear into the entries the game's grant function takes.
///
/// **Weapons arrive as PAIRS** -- soulsplanner emits `name, infusion, name, infusion, ...` across
/// the six weapon slots -- so they are walked two at a time and everything else one at a time.
/// Reading that list as a flat run of names puts `Dark` and `Bleed` through the item lookup, where
/// they resolve to nothing and read as a broken catalogue.
///
/// Anything that does not resolve is SKIPPED and logged rather than failing the whole build: one
/// unrecognised name should cost the player that item, not the other thirty. A name carried by
/// SEVERAL ids is not in that category -- see the comment on the collision arm below.
#[cfg(windows)]
pub(crate) fn build_items(build: &ds2_build_import_core::Build) -> Vec<game::ItemSpawn> {
    use ds2_build_import_core::{Infusion, ItemError, id_for, is_empty_slot};

    let mut out = Vec::new();
    let mut push = |name: &str, infusion: Infusion| {
        let item_id = match id_for(name) {
            Ok(item_id) => item_id,
            Err(ItemError::EmptySlot) => return,
            // A NAME CARRIED BY SEVERAL IDS STILL GETS THE PLAYER AN ITEM.
            //
            // `id_for` refuses, and it is right to: the question "which id is this name" has no
            // single answer. But the question the BUILD asks is different. soulsplanner only ever
            // names a display name, and the seven collisions in the catalogue are all one item
            // appearing more than once -- three Estus Flasks, and two complete armour sets whose
            // ids differ by 1000 while every piece keeps its name. So refusing means a player who
            // asked for an Estus Flask gets nothing, which is a worse answer than getting one of
            // the three things called Estus Flask.
            //
            // The LOWEST id, because the two variant sets are laid out base-then-variant and the
            // lower run is the one the game's own item lists start from. That is a reading of the
            // catalogue, not a fact about the game -- so the line below names every candidate,
            // which makes a wrong pick visible instead of silent.
            Err(ItemError::Ambiguous { ref ids, .. }) => {
                let Some(chosen) = ids.iter().copied().min() else {
                    return;
                };
                log_line(format_args!(
                    "{LOG_PREFIX} {name:?} names {} items {ids:?} -- granting {chosen}",
                    ids.len()
                ));
                chosen
            }
            Err(error) => {
                return log_line(format_args!("{LOG_PREFIX} skipping {name:?}: {error}"));
            }
        };
        out.push(game::ItemSpawn {
            mode: ds2_rva::ITEM_SPAWN_MODE_NORMAL,
            item_id,
            // ZERO, which is what the game's own caller writes. This used to pass `-1`
            // reinterpreted as a float -- a NaN -- on the strength of a community table's
            // "-1 means max" convention that the disassembly supports nowhere. It never caused a
            // refusal (the check path overwrites this field before deciding) but the ADD path
            // stores the caller's bytes verbatim, so a NaN would have been written onto the item.
            durability: ds2_rva::ITEM_SPAWN_DURABILITY_DEFAULT,
            quantity: 1,
            reinforce: 0,
            infusion: infusion.byte(),
        });
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

/// The one thing this crate can check without a game: that the two stat orders line up.
///
/// `ds2-build-import-core` knows the planner's order and `ds2-rva` knows the game's, and neither
/// can see the other -- core has no dependencies at all. This crate depends on both, so it is the
/// only place the claim can be tested. It runs on the host, with no game and no Windows.
#[cfg(test)]
mod stat_order {
    /// `Stats::in_game_order` must produce values in the order `PLAYER_PARAM_STAT_NAMES` names.
    ///
    /// Built by giving each stat the value of its own index in the GAME's list, then asserting the
    /// reordered array is `[0, 1, 2, ...]`. If either order changes, this stops being the identity.
    #[test]
    fn the_planners_order_maps_onto_the_games() {
        use ds2_build_import_core::saved_build::Stats;

        let index_of = |name: &str| {
            ds2_rva::PLAYER_PARAM_STAT_NAMES
                .iter()
                .position(|candidate| *candidate == name)
                .unwrap_or_else(|| panic!("ds2-rva does not name {name:?}")) as u16
        };
        let stats = Stats {
            vigor: index_of("vigor"),
            endurance: index_of("endurance"),
            vitality: index_of("vitality"),
            attunement: index_of("attunement"),
            strength: index_of("strength"),
            dexterity: index_of("dexterity"),
            adaptability: index_of("adaptability"),
            intelligence: index_of("intelligence"),
            faith: index_of("faith"),
        };
        assert_eq!(
            stats.in_game_order(),
            [0, 1, 2, 3, 4, 5, 6, 7, 8],
            "the two crates disagree about where a stat lives"
        );
        // And the thing that makes the bug silent: the TOTAL is the same either way, so a level
        // computed from a wrong permutation still looks right.
        let scrambled: u16 = stats.each().iter().map(|(_, value)| value).sum();
        assert_eq!(scrambled, stats.in_game_order().iter().sum::<u16>());
    }
}
