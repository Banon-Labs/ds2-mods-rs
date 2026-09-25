//! Point DARK SOULS II's save directory somewhere else, so another player's save can be loaded
//! without touching your own.
//!
//! # What it moves, and what it deliberately does not
//!
//! One detour, on [`ds2_rva::SAVE_DIR_BUILD`] -- the `SaveLoadSystem` helper that produces the
//! folder a `.sl2` lives in:
//!
//! ```text
//! FUN_140248db0(std::wstring *out, const wchar_t *steamid)
//!     out  = "%APPDATA%\\DarkSoulsII\\"
//!     out += steamid
//!     out += "\\"
//! ```
//!
//! Its two callers are both `SaveLoadSystem` methods, so this reaches the saves and nothing else.
//! The wider chokepoint one level down ([`ds2_rva::SAVE_APPDATA_ROOT_BUILD`]) is shared with
//! `GraphicsConfig_SOFS.xml`, and moving the graphics config is not what anyone asked for.
//!
//! # The Steam ID is part of what this replaces, and that is the useful part
//!
//! The second argument is the running account's Steam ID as text, fetched through the same vtable
//! slot the cached ID comes from -- which is why the on-disk layout is
//! `…\DarkSoulsII\<steamid hex>\DS2SOFS0000.sl2`. Because the detour replaces the whole result
//! rather than only the root, a redirect can point straight at a donor save's own folder
//! (`…\DarkSoulsII\01100001526d6d84\`) instead of requiring it be renamed to the running
//! account's. Hooking the root instead would have forced the rename, since the game would go on
//! appending its own ID underneath.
//!
//! # Why it calls the game's `assign` instead of writing the string
//!
//! `out` is a live MSVC `std::basic_string<wchar_t>` owned by the caller, and it may already hold
//! an allocation from the game's allocator. Writing its three fields directly would either leak
//! that allocation or invite a free from the wrong heap. [`ds2_rva::WSTRING_ASSIGN`] is the same
//! function the original uses to seat its own result, so handing the problem back to the code that
//! owns it costs one indirect call and removes an entire class of bug.
//!
//! # Under Proton
//!
//! The game builds a Windows path, so the configured value is a Windows path. Wine maps `Z:` to
//! `/`, so `/home/you/DS2` is `Z:\home\you\DS2`. A value that does not end in a separator gets one
//! appended: the caller appends the file name to whatever this leaves behind, and the trailing
//! backslash is this function's job rather than the caller's.
//!
//! # It logs the path it produced, every time
//!
//! Both arms -- redirected and pass-through -- read the resulting string back out and log it. A
//! redirect that silently produced the wrong folder is indistinguishable from a game with no save,
//! because DS2 shows no LOAD GAME row when it finds nothing. The log line is what tells those two
//! apart without anyone having to guess.
//!
//! # Mid-session, it is [`request_dir`] and not this
//!
//! The detour above moves the directory for a whole process, which is what a launch wants and what
//! a live session cannot use: `SAVE_DIR_BUILD` runs during session setup, so re-pointing it after
//! the game has booted changes a string nothing reads again.
//!
//! [`request_dir`] is the mid-session half. It calls the same function session setup calls with
//! `SAVE_DIR_BUILD`'s result -- the one that seats the directory on the storage worker that opens
//! files -- so it reaches the field a container read consults, at a moment of the caller's
//! choosing. `ds2-save-file`'s in-session character swap is the flow that uses it.
//!
//! [`session_dir`] is the third module here and is the seam that did NOT work: it replaces one
//! vtable slot per side, and a live run measured `load-answered=1` on a read that still failed,
//! because the load session's work method reaches its string through the accessor rather than
//! through the virtual. It is kept because the pair of overrides is how the game separates saves
//! from loads, and because a seam that was disproved by measurement is worth being able to point
//! at.

// Windows-only by construction: this is a MinHook detour on a PE image.
#![cfg(windows)]

pub mod active;
pub mod install;
pub mod open_redirect;
pub mod request_dir;
pub mod session_dir;
pub mod stage;

/// Prefix on every line this crate writes, so its lines can be grepped out of the shared log.
pub const LOG_PREFIX: &str = "ds2-save-redirect:";

/// Directory beside the executable that a handoff's save is written into.
///
/// Only [`ds2_save_file::take_handoff`](../ds2_save_file/fn.take_handoff.html)'s route reaches
/// this. There used to be a `[save_redirect] path = ...` key that pointed a whole launch at a file,
/// and it was removed for lying about what it did: it never opened the file it was given. It copied
/// it here, pointed the game at this directory, and overwrote the copy on the next launch -- so a
/// session started that way played a throwaway duplicate and lost everything done in it, under a
/// help string that said "load the save at WINPATH".
///
/// `[save_redirect] directory` is the standing key that replaced it, and [`set_directory`] is why
/// it is safe where the old one was not: a folder needs no copy, so there is no duplicate to play
/// and no overwrite to lose. Nothing in that path comes through here.
pub const STAGING_DIR_NAME: &str = "ds2-save-staging";

/// The `.sl2` the game is reading and writing right now, whichever seam is pointing it there.
///
/// Three things can move the container, and they are consulted innermost-answer-first because that
/// is the order the game reaches them in:
///
/// 1. [`session_dir::SAVE`] -- a vtable override on the save session's own directory. While it is
///    armed the game builds its save path out of that folder, so nothing underneath ever sees the
///    player's own.
/// 2. [`open_redirect`] -- one `CreateFileW` answering a different file. It matches the exact path
///    the game asks for, so it applies only when that path is the one the directory builder made,
///    which is why the builder's answer is what gets offered to it.
/// 3. [`live_directory`] -- what the builder produced. The answer when nothing is armed.
///
/// # Why a caller cannot stop at `live_directory`
///
/// `ds2-save-file`'s character swap leaves the save-session override unarmed on purpose and moves
/// the saves with the open redirect alone. A caller that stops at step 3 is therefore handed the
/// player's own container -- a file the game has not written a byte to since the swap -- while the
/// whole session goes to the staged copy. `Save Game to File` did exactly that, and handed the
/// player back the save they had started the session with, under the name of the character they
/// had spent it playing.
///
/// `None` means the game has not built a save path yet, which cannot be true once a character is
/// loaded and is reported rather than guessed around.
pub fn live_container() -> Option<std::path::PathBuf> {
    let name = active_save_file_name();
    if session_dir::SAVE.armed() {
        let directory = session_dir::SAVE.directory();
        if !directory.is_empty() {
            return Some(std::path::PathBuf::from(directory).join(name));
        }
    }
    let own = live_directory()?.join(name);
    Some(open_redirect::diverted_path(&own).unwrap_or(own))
}

pub use active::{active_save_file_name, is_save_container_name};
pub use install::{
    Outcome, clear_session_directory, install, live_directory, live_steam_id, session_answers,
    set_directory, set_logger, set_session_directory, set_source,
};
pub use stage::SAVE_FILE_NAME;
pub use stage::validate_source;
