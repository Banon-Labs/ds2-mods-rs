//! Swap the character you are playing for one out of a file you picked, without ending the process.
//!
//! # This does not run. [`begin`] refuses, and the row restarts the game instead
//!
//! Everything up to the title was measured working -- the pick stages, the game leaves by its own
//! Quit Game action, the title gate is called, the list can be opened. What is missing is a
//! container directory a mod can write once the title is reached, and there is not one: four
//! candidate fields were each disproved by a live run, not by argument.
//!
//! | field | what disproved it |
//! |---|---|
//! | `SLLoadSession`'s directory virtual | the work method reaches the string through the accessor, never the virtual -- `load-answered=1` on a read that still failed |
//! | [`ds2_rva::SAVE_DIR_BUILD`] re-pointed mid-session | it runs at session setup, not per request -- `session-dir-answered=0` |
//! | the storage worker's own string | a worker belongs to a live request; at the title the holder's id names a finished one -- `set-made-no-write` |
//! | `SLLoadContent`'s string | it is the container's NAME. The field read back `DS2SOFS`; writing a path there renames the save rather than moving it |
//!
//! So [`crate::import`] falls through to the route that records the pick and restarts, which is the
//! one measured reading a donor container end to end: the loader consumes the handoff at attach and
//! arms the directory builder before the save system exists, so the game's own session setup builds
//! its directory from it. The body below is kept assembled rather than deleted, because the only
//! thing it lacks is a way to drive a session into [`ds2_rva::SL_SESSION_STATE_SETUP`] at the
//! title -- a live read says a session genuinely sits in that state when its directory is written,
//! and finding what puts it there is the one open question.
//!
//! # What the player sees, when this is on
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
//! **1. The container directory cannot be written from the title, and four runs say so.** It lives
//! on the storage worker at [`ds2_rva::SL_WORKER_DIRECTORY_OFFSET`], written only by
//! [`ds2_rva::SL_WORKER_SET_DIRECTORY`] from the [`ds2_rva::SL_SESSION_STATE_SETUP`] arm of the
//! session pump. A worker exists only for a live request and is found by the id at
//! `[[system+0x38]+8]`, which at the title names a request that has already finished -- so the set
//! lands nowhere. The same failed lookup is why [`ds2_rva::SL_GET_SESSION_STATE`] answers
//! [`ds2_rva::SL_SESSION_STATE_DONE`] there.
//!
//! | seam | what disproved it |
//! |---|---|
//! | `SLLoadSession`'s directory virtual | the work method reaches the string through the accessor, never the virtual -- `load-answered=1` on a read that still failed |
//! | [`ds2_rva::SAVE_DIR_BUILD`] re-pointed mid-session | it runs at session setup, not per request -- `session-dir-answered=0` |
//! | `SLLoadContent`'s own string | it is the container's NAME, not a folder: the field read back `DS2SOFS`, see [`ds2_rva::SL_CONTENT_NAME_OFFSET`] |
//! | the worker's own string | `set-made-no-write ... no worker for its id` |
//!
//! **So this flow refuses at the title rather than arming**, and says which of those it is. It is
//! not a fix waiting to be finished: no field currently known can be written from there, and the
//! native path that does write one is session setup, which nothing asks for directly -- every one
//! of [`ds2_rva::SL_SESSION_BUILD_SAVE`]'s nine call sites passes kind zero.
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
//! those records come from.
//!
//! **And asking is only half of it.** That call hands the request to the `SLSession` worker and
//! returns; the interlock is cleared, and entry 7 parsed into the records block, by a separate
//! per-frame call -- [`ds2_rva::SAVE_LOAD_SYSTEM_PUMP`] -- whose only two shipped callers are
//! title substates that are not on screen while the top menu is. So at the top menu the request
//! is accepted, is performed, and then sits there. Four steps in one order: arm, re-read, **pump
//! until it says done**, open the list.
//!
//! **4. The list's load branch is where a character becomes committed.** Phase
//! [`ds2_rva::FE_DATA_LIST_PHASE_LOAD`] is reached only after the game's own occupancy, exclusion
//! and ownership checks have passed. `ds2-continue` reports it, and that report is what arms the
//! save side -- because at that instant, and not before, the character about to be played is one
//! that came out of the staged container.
//!
//! # The ordering is the safety, and it only works in one direction
//!
//! One directory means one window, and the window is the whole of the safety:
//!
//! | when | worker directory | why it is safe |
//! |---|---|---|
//! | the press | the game's own | the character being left has not been saved yet |
//! | at the title | **staged** | no character is loaded, so there is nothing a save could write |
//! | a character is confirmed | staged | what is about to be played came from there, so its progress belongs there |
//! | any path that gives up | put back | or the player's own list would describe somebody else's container |
//!
//! Staging any earlier writes the player's own character into somebody else's container. The
//! moment is fact 4, and the put-back is why nothing is staged until the game's own directory has
//! been observed -- that record is the only way home.
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
//! # What the live runs have said so far
//!
//! **2026-09-23, the first.** Everything up to the re-read worked -- the pick staged, the game
//! left, the title reached, the side armed, `accepted=true` on the request. Then fifteen seconds
//! of nothing and `swap ABANDONED -- the container re-read never finished`, because the request
//! was waiting on a collector that only runs inside two title substates. That is the pump above,
//! and calling it here is the fix that run bought.
//!
//! **2026-09-23, after the pump.** The pump answered on the very next frame with status 3: the
//! read had failed. `load-answered=1` said the game had reached the armed vtable slot, which is
//! what disproved the per-side split and started the search that ended at the storage worker.
//!
//! **2026-09-23, the worker's own string.** `set-made-no-write ... the request manager had no
//! worker for its id`: the call reached the game and the lookup found nothing, because a worker
//! belongs to a live request and the title is between requests.
//!
//! **2026-09-23, the load content's string.** The reader reported its numbers instead of
//! `<unreadable>` and the numbers answered it: `length=356486873167 capacity=7` is UTF-16
//! `"OFS\0"` beside a length of seven -- `DS2SOFS`, the save's own name. The field is the
//! container's identity, and writing a path into it would rename what the player is loading
//! rather than move it, so [`ds2_rva::SL_CONTENT_NAME_OFFSET`] is now named for what it holds.
//!
//! The flow therefore stops at the title and reports which field it could not write. Everything
//! before that point -- the pick, the stage, the return to title, the title gate -- has been run
//! and works; what is missing is a way to ask the game for a session setup.

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

/// Whether the in-session route may run at all.
///
/// `false`, and there is no way to set it: the flow has no field to write once the title is
/// reached. It is a constant rather than a deleted module so the assembled flow survives for
/// whoever finds the thing that would make it work, and so that turning it back on is one line
/// against a body that has been kept compiling rather than a rewrite from the git history.
static ARMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
    /// There is no container directory a mod can write from the title. See [`begin`].
    NoDirectoryField,
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
            Self::NoDirectoryField => write!(f, "no-writable-container-directory"),
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
///
/// # It refuses, and the refusal is the finding
///
/// Everything below this guard has been run in the game and works: the pick stages, the game
/// leaves by its own Quit Game action, the title gate is called, the character list can be opened.
/// What does not exist is a container directory a mod can write once the title is reached, and the
/// four candidate fields were each disproved by a run rather than by argument -- the table in this
/// module's header has them.
///
/// Leaving it enabled would take the player out of their game, sit them at the title, and give up
/// there. Refusing here sends them down [`crate::import`]'s other route instead, which ends in a
/// restart and is the one measured reading a donor container end to end: the loader consumes the
/// handoff at attach, arms [`ds2_rva::SAVE_DIR_BUILD`] before the save system exists, and the
/// game's own session setup builds its directory from the armed builder. Slower, and it works.
///
/// The body is kept rather than deleted because the parts that were proved are worth keeping
/// assembled for whoever finds what drives a session into
/// [`ds2_rva::SL_SESSION_STATE_SETUP`] at the title -- the one thing that would make the rest of
/// this run.
pub fn begin(picked: &Path) -> Result<(), NotBegun> {
    if !ARMED.load(std::sync::atomic::Ordering::Relaxed) {
        log_line(format_args!(
            "{LOG_PREFIX} swap not attempted for {} -- no container directory can be written from \
             the title, so the restart route is the one that loads a file",
            picked.display()
        ));
        return Err(NotBegun::NoDirectoryField);
    }
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
    // THE QUESTIONS FROM HERE ON ARE THE PLAYER'S. Measured 2026-09-23: confirming a character put
    // up a `common-window` with `cancel-dest=0x55` and no confirm destination, `ds2-dialog-skip`
    // read the single published edge as a notice's one outcome and answered it, and the cancel edge
    // is the way back to the list. Twelve rounds of that, and from the player's side a save slot
    // that accepts a press and does nothing.
    ds2_dialog_skip::hold();

    // Last, because it is the irreversible half. Everything above can be abandoned by dropping a
    // registration; once the game has been asked to leave, the player is watching a confirm dialog
    // and something had better be waiting for them at the title.
    if !ds2_menu_row::return_to_title() {
        ds2_continue::clear_title_gate();
        ds2_dialog_skip::release();
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
            // NOTHING POINTS A DIRECTORY HERE, and [`begin`] refuses before this is ever reached.
            // Four candidate fields were tried and each was disproved by a run; the table in this
            // module's header has them, and `ds2_save_redirect::request_dir` has the disassembly.
            //
            // What is left standing is the arm below: the side redirect, the container re-read, and
            // the wait. They were measured reaching the game -- `load-answered=1`, `accepted=true`,
            // the pump answering -- and they are correct for a container that has been pointed
            // somewhere. Pointing it is the missing piece, not this.
            //
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
            let Some(system) = game::save_load_system() else {
                return expire_or_wait(guard, "the save system could not be reached");
            };
            // THE REQUEST DOES NOT COMPLETE ITSELF. The `SLSession` worker reads the container, but
            // the interlock is cleared -- and entry 7 actually parsed into the block the character
            // list is built from -- only by this call, on the game thread. Its two shipped callers
            // are title substates that are not resident while the top menu is up, so for the one
            // moment this flow needs it, the flow that started the request is the only thing that
            // can finish it. Leaving it out is what the first live run spent fifteen seconds not
            // recovering from.
            let Some(status) = game::pump(system) else {
                *guard = None;
                drop(guard);
                abandon(
                    "the save-load pump is not at the address this build records, so a re-read \
                     started here could never be collected",
                );
                return TitleStep::Finished;
            };
            if status == ds2_rva::SAVE_LOAD_SYSTEM_PUMP_WORKING {
                return expire_or_wait(guard, "the container re-read never finished");
            }
            if status != ds2_rva::SAVE_LOAD_SYSTEM_PUMP_DONE
                && status != ds2_rva::SAVE_LOAD_SYSTEM_PUMP_IDLE
            {
                let picked = swap.picked.display().to_string();
                *guard = None;
                drop(guard);
                abandon(&format!(
                    "the container re-read failed status={status} -- the game could not read \
                     character records out of {picked}"
                ));
                return TitleStep::Finished;
            }
            if !game::interlock_idle(system) {
                return expire_or_wait(
                    guard,
                    "the container re-read reported finished but the save system stayed busy",
                );
            }
            if restoring {
                log_line(format_args!(
                    "{LOG_PREFIX} swap put back -- the character list describes your own container \
                     again"
                ));
                *guard = None;
                crate::import::restore();
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
    ds2_dialog_skip::release();
    // AND THE LABEL, which the press borrowed to say the game was being left. This is the path that
    // reaches a player who declined the confirm: the pause menu is still up, still in front of
    // them, and the row would otherwise go on announcing a departure that never happened.
    crate::import::restore();
    let (answered, passed) = session_dir::LOAD.answers();
    // Nothing to put back, because nothing was ever written: the flow refuses before it arms. The
    // container name is reported so a log says what the save system was pointed at when it gave up.
    let name = game::save_load_system()
        // SAFETY: game thread, on a path that has already stopped the flow.
        .and_then(|system| unsafe { ds2_save_redirect::request_dir::content_name(system) })
        .map_or_else(
            || String::from("<no load content>"),
            |name| name.to_string(),
        );
    log_line(format_args!(
        "{LOG_PREFIX} swap ABANDONED -- {why}. load-side-restored={load} save-side-restored={save} \
         load-answered={answered} load-passed-through={passed} container-name={name}"
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
    // The save side's vtable slot, armed for completeness rather than for effect: it is the seam a
    // measured run disproved, and the worker directory below is what a save actually writes to. It
    // stays because `export` reads `SAVE.directory()` to say where a Save Game to File row's file
    // came from, and because the two being armed together is the state that crate expects.
    // SAFETY: the game is mapped and past `DllMain`; this is its own thread at the title.
    let armed = unsafe { session_dir::SAVE.arm() };
    // The container the list is about, reported rather than assumed. This flow never armed a
    // redirect -- no writable directory field is known -- so a load confirmed here is a load out
    // of the container the game was already using.
    let directory = game::save_load_system()
        // SAFETY: game thread at the title, in the character list's own callback.
        .and_then(|system| unsafe { ds2_save_redirect::request_dir::content_name(system) })
        .map_or_else(
            || String::from("<no load content>"),
            |seated| seated.to_string(),
        );
    ds2_continue::clear_title_gate();
    // The flow is over, so the boxes go back to being this build's business. Released here rather
    // than at `StartIngame`: the load is committed, and a hold that outlived its flow would leave
    // every notice for the rest of the session waiting on a keypress.
    ds2_dialog_skip::release();
    *guard = None;
    drop(guard);
    // The flow is over, so the row's label is its own again -- before the character finishes
    // loading, so the first pause menu of the new session is bound from a caption that has already
    // been put back.
    crate::import::restore();
    if armed {
        log_line(format_args!(
            "{LOG_PREFIX} swap done slot={slot} content-directory={directory} -- loading from \
             {staged}, and this session now saves there too; your own container is untouched"
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

    /// The three pump statuses the wait branches on are three different numbers.
    ///
    /// They are read out of one `switch`'s exit paths, and a transcription that collapsed two of
    /// them would turn "still working" into "done" -- which opens the character list over a records
    /// block the re-read has not finished filling, and that is a list describing the wrong
    /// container with no sign that anything went wrong.
    #[test]
    fn the_pump_statuses_are_distinct() {
        const {
            assert!(ds2_rva::SAVE_LOAD_SYSTEM_PUMP_WORKING != ds2_rva::SAVE_LOAD_SYSTEM_PUMP_DONE);
            assert!(ds2_rva::SAVE_LOAD_SYSTEM_PUMP_WORKING != ds2_rva::SAVE_LOAD_SYSTEM_PUMP_IDLE);
            assert!(ds2_rva::SAVE_LOAD_SYSTEM_PUMP_DONE != ds2_rva::SAVE_LOAD_SYSTEM_PUMP_IDLE);
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
