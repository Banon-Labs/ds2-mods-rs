//! Redirect the **loads only**, by swapping one pointer in `SLLoadSession`'s vtable.
//!
//! # Why this exists beside [`crate::install`]
//!
//! The detour in that module replaces the directory for everything a `SaveLoadSystem` does, which is
//! correct at startup and wrong in a live session: DARK SOULS II saves on the way out of a game, so
//! a session that re-points the directory and then quits writes the character it was playing into
//! the staged copy, and the `LOAD GAME` that follows reads back the one the player was replacing.
//! That is the whole reason `ds2-save-file`'s import row records a handoff and restarts.
//!
//! The way out is that the game already separates the two. A session asks for its directory through
//! a virtual, and the save class and the load class have their own overrides -- same three lines
//! each, different object to read the string from. So there is no runtime flag to decode: the class
//! that is asking is the answer, and the answers live at two addresses.
//!
//! | override | address |
//! |---|---|
//! | [`ds2_rva::SL_SAVE_SESSION_DIRECTORY`] -- left alone, so saves keep writing the player's folder | `0x140a8ec40` |
//! | [`ds2_rva::SL_LOAD_SESSION_DIRECTORY`] -- replaced here | `0x140a8f8d0` |
//!
//! # No code is patched
//!
//! The load override has no call sites. Its only references are its vtable slot and an RTTI entry,
//! so it is reached exclusively through [`ds2_rva::SL_LOAD_SESSION_VTABLE`] and arming this is a
//! pointer write into `.rdata`. The instruction stream is untouched, which takes Arxan out of the
//! question for this site the way a `.flo` table substitution does for `ds2-menu-row`.
//!
//! # It is all-or-nothing, so it is armed around one load and then disarmed
//!
//! Every read of a container section funnels through one path -- `FUN_140a86280`, whose three
//! callers are `loadSlotByIndex`, `loadSlot_0_andOtherSetup` and `loadSlot22_andOtherSetup` -- so
//! the swap cannot be aimed at one load and not another. Left armed, every subsequent read in the
//! session answers from the staged copy, including the autosave's own reads. [`arm`] and [`disarm`]
//! are therefore a pair, and [`armed`] exists so a caller can assert which state it is in rather
//! than assume.
//!
//! # What is not established
//!
//! That the title screen's character list goes through this path. It is built from slot reads, so it
//! very likely does, and while armed it would list the staged container's characters -- which is
//! what a picker wants and a surprise everywhere else. Nothing here has been run in the game.

use core::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::LOG_PREFIX;
use crate::install::log;

/// The replacement path, as UTF-16 with no terminator, or empty when nothing is staged.
///
/// A `Mutex<Vec<u16>>` rather than a raw pointer because the override runs on the game's thread
/// while [`set_directory`] runs on whichever thread the row used, and the game will read this while
/// we are not holding it otherwise.
static DIRECTORY: Mutex<Vec<u16>> = Mutex::new(Vec::new());

/// The original slot contents, so [`disarm`] restores rather than guesses. Zero means not armed.
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);

/// Address of the slot itself, cached at [`arm`] so [`disarm`] cannot resolve a different one.
static SLOT: AtomicUsize = AtomicUsize::new(0);

/// The game's own session-string setter, resolved once at [`arm`].
static STRING_SET: AtomicUsize = AtomicUsize::new(0);

/// `fn(session, chars, len)` -- see [`ds2_rva::SL_SESSION_STRING_SET`].
type StringSet = unsafe extern "system" fn(*mut c_void, *const u16, usize);

/// The override's shape: `fn(this, out)`, where `out` is the session whose string is being filled.
type DirectoryOverride = unsafe extern "system" fn(*mut c_void, *mut c_void);

/// What the load side answers while armed. Windows form, because this runs inside the prefix.
///
/// Setting it does not arm anything. A directory with no arm is inert, which is deliberate: the two
/// halves fail in different ways and a caller that does one and not the other should get the
/// harmless outcome.
pub fn set_directory(windows_path: &str) {
    let wide = shaped(windows_path);
    let Ok(mut held) = DIRECTORY.lock() else {
        log(format_args!(
            "{LOG_PREFIX} load-session directory NOT set -- the lock is poisoned"
        ));
        return;
    };
    *held = wide;
}

/// The staged path in the form the override hands back, separate from the static so the shaping can
/// be tested without a global that parallel tests would fight over.
fn shaped(windows_path: &str) -> Vec<u16> {
    // An empty path clears the staging rather than becoming a bare separator. The separator alone is
    // the prefix's drive root -- a folder that exists and holds no container -- and
    // `directory_override` reads empty as "nothing staged, use the game's own", so this is what
    // makes `set_directory("")` an undo instead of a redirect to the worst possible place.
    if windows_path.is_empty() {
        return Vec::new();
    }
    let mut wide: Vec<u16> = windows_path.encode_utf16().collect();
    // The game appends a filename to this, and every caller of the original gets a trailing
    // separator from `SAVE_DIR_BUILD`. Matching that is not cosmetic: without it the container is
    // looked for beside the folder rather than inside it.
    if !matches!(wide.last(), Some(&c) if c == u16::from(b'\\')) {
        wide.push(u16::from(b'\\'));
    }
    wide
}

/// Whether the vtable slot currently holds the replacement.
pub fn armed() -> bool {
    ORIGINAL.load(Ordering::Acquire) != 0
}

/// Our override. **Runs on the game's thread, inside its load path.**
///
/// Falls back to the original whenever anything is not as expected -- no staged directory, an
/// unresolvable setter, a poisoned lock. A load that reads the player's own folder is the game
/// behaving normally; a load that reads nothing is a session with no save, which looks to the
/// player exactly like a corrupted profile.
unsafe extern "system" fn directory_override(this: *mut c_void, out: *mut c_void) {
    let original = ORIGINAL.load(Ordering::Acquire);
    let fallback = || {
        if original != 0 {
            // SAFETY: `original` is what the slot held before `arm` replaced it, so it is the
            // game's own override with this exact signature.
            let original: DirectoryOverride = unsafe { core::mem::transmute(original) };
            unsafe { original(this, out) };
        }
    };

    let setter = STRING_SET.load(Ordering::Acquire);
    if setter == 0 {
        fallback();
        return;
    }
    let Ok(held) = DIRECTORY.lock() else {
        fallback();
        return;
    };
    if held.is_empty() {
        drop(held);
        fallback();
        return;
    }
    // SAFETY: `setter` is `SL_SESSION_STRING_SET` resolved in `arm`, and `out` is the session the
    // game handed us -- the same one the original would have filled.
    let setter: StringSet = unsafe { core::mem::transmute(setter) };
    unsafe { setter(out, held.as_ptr(), held.len()) };
}

/// Put the replacement in the vtable slot. Idempotent; returns whether the slot now holds it.
///
/// # Safety
///
/// The game must be loaded and past the point where its `.rdata` is mapped, which is everything
/// after `DllMain`.
pub unsafe fn arm() -> bool {
    if armed() {
        return true;
    }
    let Ok(slot) = ds2_game_base::mem::game_rva(ds2_rva::SL_LOAD_SESSION_VTABLE) else {
        log(format_args!(
            "{LOG_PREFIX} load-session NOT armed -- the game module base is unknown"
        ));
        return false;
    };
    let slot = slot + ds2_rva::SL_LOAD_SESSION_DIRECTORY_VTABLE_SLOT * size_of::<usize>();

    // WHAT THE SLOT MUST ALREADY CONTAIN. An RVA is a number, and on a build this vtable was not
    // read from, this address points at something else that would accept the write and produce a
    // game that loads the wrong thing quietly. The slot has to hold the override this crate expects.
    let Ok(expected) = ds2_game_base::mem::game_rva(ds2_rva::SL_LOAD_SESSION_DIRECTORY) else {
        return false;
    };
    // SAFETY: a vtable slot in the image's `.rdata`, read-only but mapped.
    let found = unsafe { ds2_game_base::mem::safe_read_usize(slot) };
    if found != Some(expected) {
        log(format_args!(
            "{LOG_PREFIX} load-session NOT armed -- vtable slot at {slot:#x} holds {:#x}, expected \
             {expected:#x}",
            found.unwrap_or(0)
        ));
        return false;
    }

    let Ok(setter) = ds2_game_base::mem::game_rva(ds2_rva::SL_SESSION_STRING_SET) else {
        return false;
    };
    STRING_SET.store(setter, Ordering::Release);
    SLOT.store(slot, Ordering::Release);

    // Through a pointer rather than straight to an integer: a function ITEM is zero-sized and
    // `as usize` on one is the cast clippy's `function_casts_as_integer` refuses, because the same
    // spelling on a function POINTER means something else. `as *const ()` first makes it the
    // address of the code, which is what a vtable slot holds.
    let replacement = directory_override as *const () as usize;
    // SAFETY: `slot` has been read back and matches the expected original, so it is the vtable entry
    // and not an arbitrary address.
    if !unsafe { write_slot(slot, replacement) } {
        log(format_args!(
            "{LOG_PREFIX} load-session NOT armed -- the slot at {slot:#x} could not be made writable"
        ));
        return false;
    }
    ORIGINAL.store(expected, Ordering::Release);
    log(format_args!(
        "{LOG_PREFIX} load-session armed slot={slot:#x} original={expected:#x}"
    ));
    true
}

/// Put the game's own override back. Idempotent; returns whether the slot holds the original.
///
/// # Safety
///
/// Same as [`arm`].
pub unsafe fn disarm() -> bool {
    let original = ORIGINAL.load(Ordering::Acquire);
    if original == 0 {
        return true;
    }
    let slot = SLOT.load(Ordering::Acquire);
    // SAFETY: `slot` is the address `arm` wrote, cached rather than re-resolved so a changed base
    // cannot send the restore somewhere else.
    if !unsafe { write_slot(slot, original) } {
        log(format_args!(
            "{LOG_PREFIX} load-session STILL ARMED -- the slot at {slot:#x} could not be restored"
        ));
        return false;
    }
    ORIGINAL.store(0, Ordering::Release);
    log(format_args!("{LOG_PREFIX} load-session disarmed"));
    true
}

/// Make one pointer-sized slot writable, write it, and put the protection back.
///
/// # Safety
///
/// `slot` must be a mapped, pointer-aligned address inside the game's image.
unsafe fn write_slot(slot: usize, value: usize) -> bool {
    if slot == 0 {
        return false;
    }
    let mut previous: u32 = 0;
    // SAFETY: the caller guarantees `slot` is mapped; `previous` is a live local.
    let opened = unsafe {
        VirtualProtect(
            slot as *mut c_void,
            size_of::<usize>(),
            PAGE_READWRITE,
            &raw mut previous,
        )
    };
    if opened == 0 {
        return false;
    }
    // SAFETY: the page is writable for the duration of this block and `slot` is pointer-aligned.
    unsafe { (slot as *mut usize).write(value) };
    let mut discard: u32 = 0;
    // SAFETY: restoring the protection this call just changed.
    unsafe {
        VirtualProtect(
            slot as *mut c_void,
            size_of::<usize>(),
            previous,
            &raw mut discard,
        )
    };
    true
}

// Declared rather than pulled from `windows-sys`, matching `ds2-boot-timeline`: one import with a
// stable ABI does not justify a dependency this crate otherwise has no use for.
unsafe extern "system" {
    fn VirtualProtect(address: *mut c_void, size: usize, new: u32, old: *mut u32) -> i32;
}

/// The only protection this module ever asks for. A vtable slot is data; it never needs execute.
const PAGE_READWRITE: u32 = 0x04;

#[cfg(test)]
mod tests {
    use super::*;

    /// What [`shaped`] produced, as a `String`, so a test asserts on the text the override hands the
    /// game rather than on a `Vec<u16>` nobody can read in a failure message.
    fn shaped_text(path: &str) -> String {
        String::from_utf16(&shaped(path)).expect("shaping only ever re-emits its own input")
    }

    /// The trailing separator is the whole point of [`shaped`] doing anything at all.
    ///
    /// The game appends `DS2SOFS0000.sl2` to whatever this leaves behind, so a path without one
    /// sends the load to a sibling of the folder -- which reads to a player as a profile with no
    /// save rather than as an error.
    #[test]
    fn a_directory_always_ends_in_a_separator_and_is_never_doubled() {
        assert_eq!(
            shaped_text("Z:\\home\\you\\staged"),
            "Z:\\home\\you\\staged\\"
        );
        assert_eq!(
            shaped_text("Z:\\home\\you\\staged\\"),
            "Z:\\home\\you\\staged\\"
        );
    }

    /// A forward slash is not the separator the game appends against, so it does not count as one.
    ///
    /// Wine accepts either in a path, but the character checked for here is the one
    /// [`ds2_rva::SAVE_DIR_BUILD`] itself ends with, and matching that is what makes the two
    /// spellings interchangeable.
    #[test]
    fn a_forward_slash_is_not_the_separator_this_looks_for() {
        assert_eq!(shaped_text("Z:/home/you/staged"), "Z:/home/you/staged\\");
    }

    /// An empty path clears the staging instead of becoming a bare separator.
    ///
    /// A lone `\` is the prefix's drive root: a folder that exists, holds no container, and would
    /// therefore turn an undo into the quietest possible wrong answer.
    #[test]
    fn nothing_staged_stays_nothing() {
        assert!(shaped("").is_empty());
    }

    /// Nothing in this module arms itself.
    ///
    /// The two halves are deliberately separate -- a directory with no arm is inert -- and this pins
    /// that the harmless half cannot perform the other by itself. It asserts the module's resting
    /// state, which is why it does not touch the staged directory: every other test here is pure,
    /// and this one stays that way so the harness can run them all at once.
    #[test]
    fn a_fresh_module_is_not_armed() {
        assert!(!armed());
    }
}
