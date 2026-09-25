//! Answer a different container directory for **one side** of save/load, by swapping one pointer
//! in a session class's vtable.
//!
//! # Why a side at a time
//!
//! The detour in [`mod@crate::install`] replaces the directory for everything a `SaveLoadSystem` does,
//! which is correct at startup and wrong in a live session: DARK SOULS II saves on the way out of a
//! game, so a session that re-points the directory and then leaves writes the character it was
//! playing into the staged copy, and the `LOAD GAME` that follows reads back the one the player was
//! replacing.
//!
//! The way out is that the game already separates the two. A session asks for its directory through
//! a virtual, and the save class and the load class have their own overrides -- same three lines
//! each, different object to read the string from. So there is no runtime flag to decode: the class
//! that is asking is the answer, and the answers live at two addresses.
//!
//! | side | override | vtable |
//! |---|---|---|
//! | [`LOAD`] | [`ds2_rva::SL_LOAD_SESSION_DIRECTORY`] | [`ds2_rva::SL_LOAD_SESSION_VTABLE`] |
//! | [`SAVE`] | [`ds2_rva::SL_SAVE_SESSION_DIRECTORY`] | [`ds2_rva::SL_SAVE_SESSION_VTABLE`] |
//!
//! **The two are armed at different moments and that is the whole point.** The in-session character
//! swap arms [`LOAD`] when the player picks a donor container, so the title's character list and
//! the character load both read it, and arms [`SAVE`] only once the game has entered a character
//! that CAME from that container. A save side armed any earlier writes the player's own character
//! into somebody else's file; armed any later, the donor character's first bonfire overwrites a
//! slot of the player's own.
//!
//! # No code is patched
//!
//! Neither override has a call site. Their only references are their vtable slot and an RTTI entry,
//! so each is reached exclusively through its vtable and arming is a pointer write into `.rdata`.
//! The instruction stream is untouched, which takes Arxan out of the question for these sites the
//! way a `.flo` table substitution does for `ds2-menu-row`.
//!
//! # It is all-or-nothing per side, so it is armed and disarmed rather than left on
//!
//! Every read of a container section funnels through one path -- `FUN_140a86280`, whose three
//! callers are `loadSlotByIndex`, `loadSlot_0_andOtherSetup` and `loadSlot22_andOtherSetup` -- so
//! [`LOAD`] cannot be aimed at one load and not another. [`Side::arm`] and [`Side::disarm`] are a
//! pair, and [`Side::armed`] exists so a caller can assert which state it is in rather than assume.

use core::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::LOG_PREFIX;
use crate::install::log;

/// `fn(session, chars, len)` -- see [`ds2_rva::SL_SESSION_STRING_SET`].
type StringSet = unsafe extern "system" fn(*mut c_void, *const u16, usize);

/// An override's shape: `fn(this, out)`, where `out` is the session whose string is being filled.
type DirectoryOverride = unsafe extern "system" fn(*mut c_void, *mut c_void);

/// The game's own session-string setter, resolved on the first [`Side::arm`].
///
/// Shared by both sides because it is one function: both overrides end by calling it, and a second
/// copy of the address would be a second thing to keep right.
static STRING_SET: AtomicUsize = AtomicUsize::new(0);

/// One side of the split, and the storage it answers out of.
///
/// The fields are `&'static` references to statics rather than the statics themselves so that
/// [`LOAD`] and [`SAVE`] can be ordinary constants while each still owns storage the game's thread
/// can read. Everything a side needs is reachable from the side, which is what stops the two from
/// being two copies of one module.
pub struct Side {
    /// What this side is called in the log. `load` or `save`.
    what: &'static str,
    /// The vtable holding the override.
    vtable: u32,
    /// The override the slot must already contain, checked before anything is written.
    original_rva: u32,
    /// Which slot of that vtable.
    slot_index: usize,
    /// The replacement path, as UTF-16 with no terminator, or empty when nothing is staged.
    ///
    /// A `Mutex<Vec<u16>>` rather than a raw pointer because the override runs on the game's
    /// thread while [`Side::set_directory`] runs on whichever thread the row used.
    directory: &'static Mutex<Vec<u16>>,
    /// What the slot held before [`Side::arm`] replaced it, so [`Side::disarm`] restores rather
    /// than guesses. Zero means not armed.
    original: &'static AtomicUsize,
    /// Address of the slot itself, cached at [`Side::arm`] so [`Side::disarm`] cannot resolve a
    /// different one.
    slot: &'static AtomicUsize,
    /// This side's replacement override. One `extern` function per side, because the game hands
    /// the override no way to say which side called it.
    replacement: DirectoryOverride,
    /// Times the replacement ran and handed the game the staged directory.
    ///
    /// **An arm is not an answer.** Swapping the vtable slot is a write this crate can confirm by
    /// reading the slot back; it says nothing about whether the game ever reached that slot for the
    /// work being waited on. Without this counter a failed read has two indistinguishable
    /// explanations -- the session class asking was not the one whose vtable was swapped, or it was
    /// and the container behind that path could not be read -- and they call for opposite fixes.
    answered: &'static AtomicUsize,
    /// Times the replacement ran and handed the game its own directory instead.
    ///
    /// Counted separately because it is the quiet failure: the override was reached, so the slot is
    /// right, and the answer was still the player's own folder. Every path into it is a refusal
    /// inside [`answer`] -- no staged directory, an unresolved setter, a poisoned lock.
    passed_through: &'static AtomicUsize,
}

impl Side {
    /// What this side answers while armed. Windows form, because this runs inside the prefix.
    ///
    /// Setting it does not arm anything. A directory with no arm is inert, which is deliberate: the
    /// two halves fail in different ways and a caller that does one and not the other should get
    /// the harmless outcome.
    pub fn set_directory(&self, windows_path: &str) {
        let wide = shaped(windows_path);
        let Ok(mut held) = self.directory.lock() else {
            log(format_args!(
                "{LOG_PREFIX} {}-session directory NOT set -- the lock is poisoned",
                self.what
            ));
            return;
        };
        *held = wide;
    }

    /// Whether the vtable slot currently holds the replacement.
    pub fn armed(&self) -> bool {
        self.original.load(Ordering::Acquire) != 0
    }

    /// `(answered, passed through)` since the process started.
    ///
    /// The first number is the only evidence that arming this side changed what the game read.
    /// A flow that armed, asked for a container, and failed reports it: zero means the request
    /// never reached this vtable slot at all, and no amount of work on the staged file would have
    /// made a difference.
    pub fn answers(&self) -> (usize, usize) {
        (
            self.answered.load(Ordering::Relaxed),
            self.passed_through.load(Ordering::Relaxed),
        )
    }

    /// The directory this side would answer, as text, for a log line that has to say what is
    /// staged rather than that something is.
    pub fn directory(&self) -> String {
        self.directory
            .lock()
            .map(|held| String::from_utf16_lossy(&held))
            .unwrap_or_default()
    }

    /// Put the replacement in the vtable slot. Idempotent; returns whether the slot now holds it.
    ///
    /// # Safety
    ///
    /// The game must be loaded and past the point where its `.rdata` is mapped, which is everything
    /// after `DllMain`.
    pub unsafe fn arm(&self) -> bool {
        if self.armed() {
            return true;
        }
        let Ok(vtable) = ds2_game_base::mem::game_rva(self.vtable) else {
            log(format_args!(
                "{LOG_PREFIX} {}-session NOT armed -- the game module base is unknown",
                self.what
            ));
            return false;
        };
        let slot = vtable + self.slot_index * size_of::<usize>();

        // WHAT THE SLOT MUST ALREADY CONTAIN. An RVA is a number, and on a build this vtable was
        // not read from, this address points at something else that would accept the write and
        // produce a game that loads the wrong thing quietly. The slot has to hold the override this
        // module expects.
        let Ok(expected) = ds2_game_base::mem::game_rva(self.original_rva) else {
            return false;
        };
        // SAFETY: a vtable slot in the image's `.rdata`, read-only but mapped.
        // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one -- it reads
        // through `ReadProcessMemory`, which validates the range in the kernel. A game structure that
        // moved or was freed answers None rather than faulting.
        let found = unsafe { ds2_game_base::mem::safe_read_usize(slot) };
        if found != Some(expected) {
            log(format_args!(
                "{LOG_PREFIX} {}-session NOT armed -- vtable slot at {slot:#x} holds {:#x}, \
                 expected {expected:#x}",
                self.what,
                found.unwrap_or(0)
            ));
            return false;
        }

        let Ok(setter) = ds2_game_base::mem::game_rva(ds2_rva::SL_SESSION_STRING_SET) else {
            return false;
        };
        STRING_SET.store(setter, Ordering::Release);
        self.slot.store(slot, Ordering::Release);

        // Through a pointer rather than straight to an integer: a function ITEM is zero-sized and
        // `as usize` on one is the cast clippy's `function_casts_as_integer` refuses, because the
        // same spelling on a function POINTER means something else. `as *const ()` first makes it
        // the address of the code, which is what a vtable slot holds.
        let replacement = self.replacement as *const () as usize;
        // SAFETY: `slot` has been read back and matches the expected original, so it is the vtable
        // entry and not an arbitrary address.
        if !unsafe { write_slot(slot, replacement) } {
            log(format_args!(
                "{LOG_PREFIX} {}-session NOT armed -- the slot at {slot:#x} could not be made \
                 writable",
                self.what
            ));
            return false;
        }
        self.original.store(expected, Ordering::Release);
        log(format_args!(
            "{LOG_PREFIX} {}-session armed slot={slot:#x} original={expected:#x} directory={}",
            self.what,
            self.directory()
        ));
        true
    }

    /// Put the game's own override back. Idempotent; returns whether the slot holds the original.
    ///
    /// # Safety
    ///
    /// Same as [`Side::arm`].
    pub unsafe fn disarm(&self) -> bool {
        let original = self.original.load(Ordering::Acquire);
        if original == 0 {
            return true;
        }
        let slot = self.slot.load(Ordering::Acquire);
        // SAFETY: `slot` is the address `arm` wrote, cached rather than re-resolved so a changed
        // base cannot send the restore somewhere else.
        if !unsafe { write_slot(slot, original) } {
            log(format_args!(
                "{LOG_PREFIX} {}-session STILL ARMED -- the slot at {slot:#x} could not be restored",
                self.what
            ));
            return false;
        }
        self.original.store(0, Ordering::Release);
        log(format_args!("{LOG_PREFIX} {}-session disarmed", self.what));
        true
    }
}

/// What an armed side hands the game, shared by both replacements.
///
/// Falls back to the original whenever anything is not as expected -- no staged directory, an
/// unresolvable setter, a poisoned lock. A load that reads the player's own folder is the game
/// behaving normally; a load that reads nothing is a session with no save, which looks to the
/// player exactly like a corrupted profile.
///
/// # Safety
///
/// Runs on the game's thread with the receiver and out-parameter the game passed.
unsafe fn answer(side: &Side, this: *mut c_void, out: *mut c_void) {
    let original = side.original.load(Ordering::Acquire);
    let fallback = || {
        let n = side.passed_through.fetch_add(1, Ordering::Relaxed) + 1;
        if n <= 2 {
            log(format_args!(
                "{LOG_PREFIX} {}-session override answered with the game's own directory \
                 (count={n}) -- the slot is right and the staging is not",
                side.what
            ));
        }
        if original != 0 {
            // SAFETY: `original` is what the slot held before `arm` replaced it, so it is the
            // game's own override with this exact signature.
            let original: DirectoryOverride = unsafe { core::mem::transmute(original) };
            // SAFETY: `original` is the trampoline MinHook produced for this target, so calling it runs the
            // bytes the detour displaced. The arguments are this detour's own, passed through untouched.
            unsafe { original(this, out) };
        }
    };

    let setter = STRING_SET.load(Ordering::Acquire);
    if setter == 0 {
        fallback();
        return;
    }
    let Ok(held) = side.directory.lock() else {
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
    // SAFETY: every pointer here is one the game handed this detour, or is derived from it by an
    // offset this crate validated before installing. The callee's own contract asks for exactly
    // that live object, and reads inside it go through the fault-tolerant readers.
    unsafe { setter(out, held.as_ptr(), held.len()) };
    let n = side.answered.fetch_add(1, Ordering::Relaxed) + 1;
    // The first two only. A session asks for its directory once, so two lines cover an arm and its
    // retry; past that this is a per-frame sink writing the same sentence.
    if n <= 2 {
        log(format_args!(
            "{LOG_PREFIX} {}-session override ANSWERED count={n} units={} -- the game asked this \
             slot for a directory and got the staged one",
            side.what,
            held.len()
        ));
    }
}

/// The load side: what the game reads a container through.
///
/// Armed while the title's character list and the character load should answer from a staged
/// container rather than the player's own.
pub static LOAD: Side = Side {
    what: "load",
    vtable: ds2_rva::SL_LOAD_SESSION_VTABLE,
    original_rva: ds2_rva::SL_LOAD_SESSION_DIRECTORY,
    slot_index: ds2_rva::SL_LOAD_SESSION_DIRECTORY_VTABLE_SLOT,
    directory: &LOAD_DIRECTORY,
    original: &LOAD_ORIGINAL,
    slot: &LOAD_SLOT,
    replacement: load_override,
    answered: &LOAD_ANSWERED,
    passed_through: &LOAD_PASSED_THROUGH,
};

static LOAD_DIRECTORY: Mutex<Vec<u16>> = Mutex::new(Vec::new());
static LOAD_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static LOAD_SLOT: AtomicUsize = AtomicUsize::new(0);
static LOAD_ANSWERED: AtomicUsize = AtomicUsize::new(0);
static LOAD_PASSED_THROUGH: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn load_override(this: *mut c_void, out: *mut c_void) {
    // SAFETY: the game's own call, forwarded with its own arguments.
    unsafe { answer(&LOAD, this, out) };
}

/// The save side: what the game writes a container through.
///
/// **Left alone unless the character being played came out of the staged container.** Everything
/// this crate does at startup deliberately leaves it pointed at the player's own folder.
pub static SAVE: Side = Side {
    what: "save",
    vtable: ds2_rva::SL_SAVE_SESSION_VTABLE,
    original_rva: ds2_rva::SL_SAVE_SESSION_DIRECTORY,
    slot_index: ds2_rva::SL_SAVE_SESSION_DIRECTORY_VTABLE_SLOT,
    directory: &SAVE_DIRECTORY,
    original: &SAVE_ORIGINAL,
    slot: &SAVE_SLOT,
    replacement: save_override,
    answered: &SAVE_ANSWERED,
    passed_through: &SAVE_PASSED_THROUGH,
};

static SAVE_DIRECTORY: Mutex<Vec<u16>> = Mutex::new(Vec::new());
static SAVE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static SAVE_SLOT: AtomicUsize = AtomicUsize::new(0);
static SAVE_ANSWERED: AtomicUsize = AtomicUsize::new(0);
static SAVE_PASSED_THROUGH: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn save_override(this: *mut c_void, out: *mut c_void) {
    // SAFETY: the game's own call, forwarded with its own arguments.
    unsafe { answer(&SAVE, this, out) };
}

/// The staged path in the form an override hands back, separate from the statics so the shaping can
/// be tested without a global that parallel tests would fight over.
fn shaped(windows_path: &str) -> Vec<u16> {
    // An empty path clears the staging rather than becoming a bare separator. The separator alone is
    // the prefix's drive root -- a folder that exists and holds no container -- and `answer` reads
    // empty as "nothing staged, use the game's own", so this is what makes `set_directory("")` an
    // undo instead of a redirect to the worst possible place.
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
    /// state, which is why it does not touch the staged directories: every other test here is pure,
    /// and this one stays that way so the harness can run them all at once.
    #[test]
    fn a_fresh_module_has_neither_side_armed() {
        assert!(!LOAD.armed());
        assert!(!SAVE.armed());
    }

    /// The two sides address two different vtables, which is the entire reason this file is not one
    /// redirect with a flag.
    ///
    /// A copy-paste that left both pointing at the load vtable would arm the save side onto the
    /// load override: every save would then be handed the load side's staged path, and the player's
    /// own container would stop being written without anything saying so.
    #[test]
    fn the_two_sides_are_not_the_same_slot() {
        assert_ne!(LOAD.vtable, SAVE.vtable);
        assert_ne!(LOAD.original_rva, SAVE.original_rva);
        assert_ne!(
            LOAD.replacement as *const () as usize,
            SAVE.replacement as *const () as usize
        );
        // Each side must name ITS OWN storage. Two sides sharing one `Mutex` would make setting the
        // load directory silently set the save one.
        assert!(!std::ptr::eq(LOAD.directory, SAVE.directory));
        assert!(!std::ptr::eq(LOAD.original, SAVE.original));
        assert!(!std::ptr::eq(LOAD.slot, SAVE.slot));
    }

    /// Each side's replacement must answer out of that side's own storage.
    ///
    /// The override takes no argument saying which side it is, so the only thing connecting
    /// `load_override` to `LOAD` is the constant it names inside its body -- a one-word mistake
    /// that no type would catch. This pins it by address.
    #[test]
    fn each_replacement_belongs_to_its_own_side() {
        assert_eq!(
            LOAD.replacement as *const () as usize,
            load_override as *const () as usize
        );
        assert_eq!(
            SAVE.replacement as *const () as usize,
            save_override as *const () as usize
        );
    }
}
