//! Swap the character you are playing for one out of a file you picked, without ending the process.
//!
//! # What the player sees
//!
//! ```text
//! press the row -> pick a file -> the game's own "return to title?" confirm
//!               -> the title's own LOAD GAME list, showing the picked file's characters
//!               -> choose one -> you are playing it
//! ```
//!
//! Every screen in that sequence is the game's. Nothing here draws a menu, and the character list
//! is the one the title screen has always had -- which is the whole reason this is a few hundred
//! lines rather than a port of Elden Ring's picker.
//!
//! # The four facts it is built on, all read out of the binary
//!
//! **1. The save side and the load side ask for their directory separately.** `SLSaveSession` and
//! `SLLoadSession` each override the same base virtual, at [`ds2_rva::SL_SAVE_SESSION_DIRECTORY`]
//! and [`ds2_rva::SL_LOAD_SESSION_DIRECTORY`], and neither override has a call site -- both are
//! reached only through their vtable. So a redirect is a pointer write per side, and the two sides
//! can be pointed at different folders at the same time. That is what kills the restart this row
//! used to perform: the character being left is written to the player's own folder by a save side
//! that was never touched, while the load side answers the staged copy.
//!
//! **2. Leaving a game is a shipped action.** The pause menu's own Quit Game row is action
//! [`ds2_rva::FE_INGAME_MENU_ACTION_RETURN_TITLE`], which opens `FeGroupInGameReturnTitleCheck` --
//! the confirm that offers to save on the way to the title. `ds2_menu_row::return_to_title` fires
//! that action, with that row's own gate applied. The character is unloaded by the game, on the
//! game's terms, and the save that goes with it is the game's own.
//!
//! **3. The character list is built from memory, not from the file.** `FUN_1400f0f60` walks
//! `GameManagerImp->GameDataManager->savedata__` -- ten `0x1f0`-byte records -- and keeps the ones
//! flagged occupied. Nothing in it reads a container. So pointing the load side somewhere else
//! changes nothing about what the list says until that block is filled again, which is what
//! [`ds2_rva::SAVE_LOAD_SYSTEM_LOAD_SYSTEM_DATA`] does: it asks for container entry 7, the section
//! those records come from. Three steps in one order -- arm, re-read, open the list.
//!
//! **4. The list's load branch is where a character becomes committed.** Phase
//! [`ds2_rva::FE_DATA_LIST_PHASE_LOAD`] is reached only after the game's own occupancy, exclusion
//! and ownership checks have passed. `ds2-continue` reports it, and that report is what arms the
//! save side -- because at that instant, and not before, the character about to be played is one
//! that came out of the staged container.
//!
//! # The ordering is the safety, and it only works in one direction
//!
//! | when | load side | save side | why |
//! |---|---|---|---|
//! | the press | the game's own | the game's own | the character being left has not been saved yet |
//! | at the title | **staged** | the game's own | the exit save has already gone to the player's folder |
//! | a character is confirmed | staged | **staged** | what is about to be played came from there |
//!
//! Arming the save side any earlier writes the player's own character into somebody else's
//! container. Arming it any later lets the donor character's first bonfire overwrite a slot of the
//! player's own. There is one correct moment and fact 4 is how this finds it.
//!
//! # Nothing here is a substitute for the game leaving the game
//!
//! An earlier design for this swapped the container while a character was loaded and never left the
//! session. It cannot work, and the reason is not a missing address: the world, the player object
//! and the save block are built during the load and torn down by the return to title, and none of
//! that machinery has an entry point that means "unload the character but stay". The return to
//! title is itself the unload. What this crate removes is the restart of the process, which was
//! never the game's requirement -- it was the price of not having read fact 1.
//!
//! # What has not been run
//!
//! Any of it. Every claim above is a claim about the disassembly, and the flow logs each step with
//! the value it acted on so that one live run can say which of them is wrong.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ds2_save_redirect::session_dir;

use crate::game;
use crate::{LOG_PREFIX, log_line};

/// The directory, beside `DarkSoulsII.exe`, a picked container is staged into.
///
/// Deliberately not the launch redirect's staging directory. That one is filled during attach and
/// is the container a `[save_redirect] path = ...` launch is reading from for the whole session;
/// writing this flow's pick over it would pull the file out from under a game already playing it.
pub const STAGING_DIR_NAME: &str = "ds2-swapped-save";

/// Pause-menu frames to wait for the player to answer the game's own confirm.
///
/// The pause menu's update is what runs this crate's tick, so a tick that is still running is
/// itself the evidence that the game was never left -- there is no other signal saying the confirm
/// was declined. Thirty seconds at 60fps: long enough to read a dialog, short enough that a
/// declined swap does not refuse the next press for the rest of the session.
const LEAVE_DEADLINE_TICKS: u32 = 1800;

const _: () = assert!(
    LEAVE_DEADLINE_TICKS >= 600,
    "under ten seconds is a misread dialog"
);

/// Title frames to wait for a container re-read before giving up on it.
///
/// The boot path's own measurement of this work is 88ms (`docs/DS2-BOOT-WORK.md`), so fifteen
/// seconds is not a tuning parameter -- it is the point at which the request is not slow but stuck,
/// and the flow should say so rather than sit on a title screen forever.
const REREAD_DEADLINE_FRAMES: u32 = 900;

/// Top-menu frames to ignore after firing LOAD GAME before a call means the player came back.
///
/// The menu keeps updating for a frame or two after its phase is written, because the transition
/// search runs when the update returns. Without this grace those frames read as "the player backed
/// out of the list" and the flow would put everything back before the list had even opened.
const HANDOVER_GRACE_FRAMES: u32 = 30;

/// Where the flow is. Each variant is a thing being waited for, not a thing being done.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// The confirm is up, or the game is on its way out. Waiting to be called at the title.
    Leaving,
    /// At the title. Arming a side and asking for the container re-read that follows it.
    Asking {
        /// Putting the player's own container back, rather than installing the staged one.
        restoring: bool,
    },
    /// The re-read is in flight. Waiting for the interlock to go idle.
    Waiting {
        /// As above.
        restoring: bool,
    },
    /// The list is open and the choice is the player's.
    Choosing,
}

/// One swap in progress. There is never more than one.
struct Swap {
    /// The staged directory, in the Windows form the game appends a filename to.
    staged: String,
    /// The file the player picked, for the log only.
    picked: PathBuf,
    /// What is being waited for.
    phase: Phase,
    /// Frames spent in the current phase, counted by whichever tick owns that phase.
    frames: u32,
}

/// The swap in progress, if any.
///
/// A lock taken from a per-frame detour, which this repo is otherwise careful about -- but the gate
/// that takes it is only registered while a swap is pending and only called while the title's top
/// menu is up. With no swap running the cost at the title is the single atomic load
/// `ds2_continue`'s own reader makes before it looks at anything here.
static SWAP: Mutex<Option<Swap>> = Mutex::new(None);

/// Why the in-session route was not taken, so the caller can say what it is falling back to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NotBegun {
    /// Another swap is already in flight.
    AlreadyPending,
    /// The game directory could not be found, so there is nowhere to stage into.
    NoGameDirectory,
    /// The running account's save directory is not known, so the rebind has no Steam ID.
    NoSteamId,
    /// Staging the picked file failed. The message is the staging error.
    Staging(String),
    /// The title flow is not hooked, so nothing can drive the character list.
    NoTitleGate,
    /// The game refused to leave -- the shipped Quit Game row is refused right now too.
    CannotLeave,
}

impl std::fmt::Display for NotBegun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyPending => write!(f, "already-pending"),
            Self::NoGameDirectory => write!(f, "no-game-directory"),
            Self::NoSteamId => write!(f, "no-steam-id"),
            Self::Staging(error) => write!(f, "staging-failed: {error}"),
            Self::NoTitleGate => write!(f, "no-title-gate"),
            Self::CannotLeave => write!(f, "the-game-refused-to-leave"),
        }
    }
}

/// The Steam ID the picked container has to be rebound to.
///
/// Asked of `ds2-save-redirect`, which records what the game passed its own directory builder.
///
/// # Not the live directory's last component
///
/// That was the first version of this, and one live run killed it. With a launch-time redirect
/// armed the directory is `...\Game\ds2-save-staging\`, whose last component is a folder name this
/// repo invented -- so the rebind would have bound the donor container to an account called
/// `ds2-save-staging`, and the character list would have come up empty. An empty list is also what
/// a failed stage looks like, so the wrong answer would have been indistinguishable from the
/// failure it caused.
fn steam_id() -> Option<String> {
    ds2_save_redirect::live_steam_id()
}

/// Start a swap: stage the pick, take the title flow, and ask the game to leave the game.
///
/// **Game thread, inside the menu's confirm path.** Returns the reason on refusal so the caller can
/// fall back to the route that ends in a restart rather than leaving the player with nothing.
pub fn begin(picked: &Path) -> Result<(), NotBegun> {
    {
        let guard = SWAP.lock().map_err(|_| NotBegun::AlreadyPending)?;
        if guard.is_some() {
            return Err(NotBegun::AlreadyPending);
        }
    }
    let root = ds2_game_base::log::game_directory_path()
        .map(|dir| dir.join(STAGING_DIR_NAME))
        .ok_or(NotBegun::NoGameDirectory)?;
    let id = steam_id().ok_or(NotBegun::NoSteamId)?;
    // A COPY, not a pointer at the player's file. The pick is staged and its Steam ID rebound to
    // the running account's, so the file the player chose is never opened for writing and a
    // container bound to somebody else's account is not handed to a game that would refuse it.
    let staged = ds2_save_redirect::stage::stage(picked, &id, &root)
        .map_err(|error| NotBegun::Staging(error.to_string()))?;
    let directory = staged.directory.to_string_lossy().into_owned();
    log_line(format_args!(
        "{LOG_PREFIX} swap staged kind={} bytes={} steam-id={id} from={} into={directory}",
        staged.kind,
        staged.bytes,
        picked.display()
    ));

    // Both sides are told where the staged copy is and neither is armed. Setting a directory is
    // inert on its own, which is what makes this safe to do here: the arming is what changes
    // behaviour, and each side is armed at the one moment it is correct.
    session_dir::LOAD.set_directory(&directory);
    session_dir::SAVE.set_directory(&directory);

    if !ds2_continue::set_title_gate(title_gate) {
        return Err(NotBegun::NoTitleGate);
    }
    ds2_continue::set_load_confirmed(load_confirmed);

    // Last, because it is the irreversible half. Everything above can be abandoned by dropping a
    // registration; once the game has been asked to leave, the player is watching a confirm dialog
    // and something had better be waiting for them at the title.
    if !ds2_menu_row::return_to_title() {
        ds2_continue::clear_title_gate();
        return Err(NotBegun::CannotLeave);
    }
    if let Ok(mut guard) = SWAP.lock() {
        *guard = Some(Swap {
            staged: directory,
            picked: picked.to_path_buf(),
            phase: Phase::Leaving,
            frames: 0,
        });
    }
    Ok(())
}

/// One pause-menu frame. Registered through the import row's own tick.
///
/// The only phase it can observe is [`Phase::Leaving`], and what it watches for is its own frame
/// count rising -- because the pause menu updating at all means the game was not left.
pub fn pause_tick() {
    let Ok(mut guard) = SWAP.lock() else {
        return;
    };
    let Some(swap) = guard.as_mut() else {
        return;
    };
    if swap.phase != Phase::Leaving {
        return;
    }
    swap.frames += 1;
    if swap.frames < LEAVE_DEADLINE_TICKS {
        return;
    }
    let picked = swap.picked.display().to_string();
    *guard = None;
    drop(guard);
    abandon(&format!(
        "the game was never left after {LEAVE_DEADLINE_TICKS} menu frames -- the confirm was \
         declined or left open. Nothing was changed; {picked} is still staged and the row can be \
         pressed again"
    ));
}

/// One frame of the title's top menu. Registered with `ds2-continue`, which owns that detour.
fn title_gate() -> ds2_continue::TitleStep {
    use ds2_continue::TitleStep;

    let Ok(mut guard) = SWAP.lock() else {
        return TitleStep::Finished;
    };
    let Some(swap) = guard.as_mut() else {
        // The gate outliving its flow is not an error -- a cancelled swap drops the state and lets
        // the next call retire the gate, which is one place for that to happen instead of two.
        return TitleStep::Finished;
    };
    swap.frames += 1;
    match swap.phase {
        Phase::Leaving => {
            // Being called at all means the title's top menu is up, which means the game was left
            // and the character that was being played has already been written to the player's own
            // folder by a save side nothing has touched.
            log_line(format_args!(
                "{LOG_PREFIX} swap at the title -- the character you left is saved in your own \
                 folder; pointing the loads at {}",
                swap.staged
            ));
            swap.phase = Phase::Asking { restoring: false };
            swap.frames = 0;
            TitleStep::Wait
        }
        Phase::Asking { restoring } => {
            // SAFETY: the game is mapped and past `DllMain`; this is its own thread at the title.
            let side_ready = if restoring {
                unsafe { session_dir::LOAD.disarm() }
            } else {
                unsafe { session_dir::LOAD.arm() }
            };
            if !side_ready {
                *guard = None;
                drop(guard);
                abandon(
                    "the load-side redirect could not be armed, so the character list would \
                     describe the wrong container",
                );
                return TitleStep::Finished;
            }
            let Some(system) = game::save_load_system() else {
                return expire_or_wait(guard, "the save system could not be reached");
            };
            if game::load_system_data(system) {
                swap.phase = Phase::Waiting { restoring };
                swap.frames = 0;
                return TitleStep::Wait;
            }
            // Refused means a request is already in flight, which is a reason to ask again next
            // frame rather than a failure. The deadline is what separates the two.
            expire_or_wait(guard, "the container re-read was never accepted")
        }
        Phase::Waiting { restoring } => {
            let idle = game::save_load_system().is_some_and(game::interlock_idle);
            if !idle {
                return expire_or_wait(guard, "the container re-read never finished");
            }
            if restoring {
                log_line(format_args!(
                    "{LOG_PREFIX} swap put back -- the character list describes your own container \
                     again"
                ));
                *guard = None;
                return TitleStep::Finished;
            }
            log_line(format_args!(
                "{LOG_PREFIX} swap ready -- the character list now describes {}; opening it",
                swap.staged
            ));
            swap.phase = Phase::Choosing;
            swap.frames = 0;
            TitleStep::TakeLoadGame
        }
        Phase::Choosing => {
            // Being called here, after the grace, means the player is back at the top menu without
            // having chosen a character -- they backed out of the list. Put the container they
            // actually own back in front of them before letting go of the menu.
            if swap.frames <= HANDOVER_GRACE_FRAMES {
                return TitleStep::Wait;
            }
            log_line(format_args!(
                "{LOG_PREFIX} swap backed out -- no character was chosen, so the staged container \
                 is being taken back out"
            ));
            swap.phase = Phase::Asking { restoring: true };
            swap.frames = 0;
            TitleStep::Wait
        }
    }
}

/// Give up on a phase that has run out of frames, or keep waiting.
///
/// Takes the guard by value because the abandon path has to drop the state before it logs, and a
/// caller holding the lock across that is how a per-frame hook deadlocks itself.
fn expire_or_wait(
    mut guard: std::sync::MutexGuard<'_, Option<Swap>>,
    what: &str,
) -> ds2_continue::TitleStep {
    let expired = guard
        .as_ref()
        .is_some_and(|swap| swap.frames >= REREAD_DEADLINE_FRAMES);
    if !expired {
        return ds2_continue::TitleStep::Wait;
    }
    *guard = None;
    drop(guard);
    abandon(what);
    ds2_continue::TitleStep::Finished
}

/// Put both sides back and say why, for every path that gives up.
///
/// **Disarms rather than assumes.** A flow that abandoned between arming the load side and reaching
/// a character would otherwise leave every subsequent read in the session answering from a
/// container the player never chose -- including the title's own list, which is the screen they
/// would be looking at while it happened.
fn abandon(why: &str) {
    // SAFETY: the game is mapped and past `DllMain`; both calls are no-ops on a side that is not
    // armed.
    let load = unsafe { session_dir::LOAD.disarm() };
    let save = unsafe { session_dir::SAVE.disarm() };
    ds2_continue::clear_title_gate();
    log_line(format_args!(
        "{LOG_PREFIX} swap ABANDONED -- {why}. load-side-restored={load} save-side-restored={save}"
    ));
}

/// The character list took its load branch: the player chose, and the game accepted.
///
/// Registered with `ds2-continue`, which reports this for every load -- including ones this flow
/// had nothing to do with. The phase check is what keeps it from arming a save-side redirect during
/// an ordinary boot.
fn load_confirmed(slot: i32) {
    let Ok(mut guard) = SWAP.lock() else {
        return;
    };
    let Some(swap) = guard.as_ref() else {
        return;
    };
    if swap.phase != Phase::Choosing {
        return;
    }
    let staged = swap.staged.clone();
    // The one moment the save side is correct. The character about to be loaded came out of the
    // staged container, so that is where its progress belongs, and the player's own container has
    // been untouched since the save that left it.
    // SAFETY: the game is mapped and past `DllMain`; this is its own thread at the title.
    let armed = unsafe { session_dir::SAVE.arm() };
    ds2_continue::clear_title_gate();
    *guard = None;
    drop(guard);
    if armed {
        log_line(format_args!(
            "{LOG_PREFIX} swap done slot={slot} -- loading from {staged}, and this session now \
             saves there too; your own container is untouched"
        ));
    } else {
        log_line(format_args!(
            "{LOG_PREFIX} swap slot={slot} loading from {staged} but THE SAVE SIDE IS NOT ARMED -- \
             this character's saves will be written into your own container, overwriting slot \
             {slot} of it. Leave to the title without saving if that is not what you want"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The staging directory is not the launch redirect's.
    ///
    /// Sharing one would mean a row press during a `[save_redirect] path = ...` session rewrote the
    /// container that session is playing out of, from under it.
    #[test]
    fn the_staging_directory_is_this_flows_own() {
        assert_eq!(STAGING_DIR_NAME, "ds2-swapped-save");
        assert_ne!(STAGING_DIR_NAME, "ds2-staged-save");
    }

    /// Nothing is pending in a fresh process, and nothing arms itself.
    #[test]
    fn a_fresh_flow_is_not_running() {
        assert!(SWAP.lock().expect("fresh mutex").is_none());
    }

    /// The grace is shorter than the deadlines it sits between, or a backed-out list would be
    /// mistaken for a stuck one.
    #[test]
    fn the_three_bounds_are_ordered() {
        const {
            assert!(HANDOVER_GRACE_FRAMES < REREAD_DEADLINE_FRAMES);
            assert!(REREAD_DEADLINE_FRAMES < LEAVE_DEADLINE_TICKS);
        }
    }

    /// Every refusal reads differently, because they are acted on differently: two of them mean
    /// "press it again" and the rest mean "this build cannot do it".
    #[test]
    fn every_refusal_reads_differently() {
        let reasons = [
            NotBegun::AlreadyPending.to_string(),
            NotBegun::NoGameDirectory.to_string(),
            NotBegun::NoSteamId.to_string(),
            NotBegun::Staging("x".into()).to_string(),
            NotBegun::NoTitleGate.to_string(),
            NotBegun::CannotLeave.to_string(),
        ];
        let mut seen = reasons.to_vec();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), reasons.len(), "{reasons:?}");
    }
}
