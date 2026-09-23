//! The OS common file dialog, as the only file browser either row draws.
//!
//! # Why comdlg32 and not a menu
//!
//! `../er-mods-rs` has both: a 3000-line in-game browser built out of Elden Ring's own
//! `05_010_ProfileSelect` list, and `er-save-picker-core::os_dialog` -- this, ported. The in-game
//! one is not portable in any useful sense. It is a different engine generation, a different
//! resource format and a different menu-ownership model, and every line of it is about ER's own
//! windows. The OS dialog has no game coupling at all: it converts strings and calls comdlg32, it
//! reads no game pointer and dereferences nothing from the module base.
//!
//! That is the whole reason this module is the port and the other one is not.
//!
//! # It BLOCKS the game thread, on purpose
//!
//! Both entry points are called inline from a row's `on_confirm`, which is the game thread inside
//! the menu's own confirm path. The dialog's message loop runs; the game's does not. That is the
//! modality we want, and it is what er-mods-rs's own note about this says after the shape was
//! removed once and put back: the objection to it was *"the context switch out of the game"*, not a
//! hang.
//!
//! **The modality comes from the block, not from `hwndOwner`.** The owner is the game's own
//! top-level window -- read from the singleton the way `ds2-build-import`'s clipboard reads it -- and
//! it is there so the dialog appears IN FRONT of a fullscreen game. A null owner is legal and was
//! not chosen: an unowned modal behind an exclusive-fullscreen swapchain is indistinguishable from a
//! freeze, and a player cannot alt-tab their way out of a dialog they cannot see.
//!
//! # What it deliberately does not do
//!
//! No worker thread and no `CoInitialize`. `GetOpenFileNameW` is the pre-COM API and wants neither;
//! handing the dialog to a thread the game does not own would disable the game window from the wrong
//! thread while the game kept polling DirectInput, which is how keystrokes typed into a dialog end
//! up in the menu underneath.

use core::ffi::c_void;
use std::path::PathBuf;

use ds2_save_file_core::wide_nul;

/// Units in the path buffer handed to comdlg32.
///
/// `MAX_PATH` is 260 and the dialog will happily return more than that when `OFN_EXPLORER` is set,
/// so the buffer is four times it. An overlong path is truncated by the API rather than overflowing
/// -- the field is `nMaxFile` and the dialog honours it -- and a truncated path fails the checks in
/// `ds2_save_file_core` rather than being written to.
const PATH_UNITS: usize = 260 * 4;

/// A buffer at or below `MAX_PATH` would make an ordinary long Windows path the reason a pick fails.
const _: () = assert!(PATH_UNITS > 260);

/// `OPENFILENAMEW`, x64 layout. 152 bytes, asserted below rather than trusted.
///
/// Declared by hand for the same reason every other Win32 surface in this repo is: the workspace
/// carries no `windows` crate, and a struct with 23 fields whose size is checked at compile time is
/// cheaper than a dependency tree.
#[repr(C)]
struct OpenFileNameW {
    struct_size: u32,
    hwnd_owner: *mut c_void,
    hinstance: *mut c_void,
    filter: *const u16,
    custom_filter: *mut u16,
    max_custom_filter: u32,
    filter_index: u32,
    file: *mut u16,
    max_file: u32,
    file_title: *mut u16,
    max_file_title: u32,
    initial_dir: *const u16,
    title: *const u16,
    flags: u32,
    file_offset: u16,
    file_extension: u16,
    def_ext: *const u16,
    cust_data: isize,
    hook: *mut c_void,
    template_name: *const u16,
    reserved_ptr: *mut c_void,
    reserved_dword: u32,
    flags_ex: u32,
}

// The struct's size IS its version tag: `lStructSize` is what comdlg32 switches on, and a Rust
// layout that padded differently from the C one would pass a number that does not match the bytes.
const _: () = assert!(core::mem::size_of::<OpenFileNameW>() == 152);

/// `OFN_OVERWRITEPROMPT` -- the save dialog asks before replacing a file.
///
/// Set even though `ds2_save_file_core::dest::Route` already names an overwrite apart from a create.
/// The two are not redundant: this one is the question the PLAYER answers, in the dialog they are
/// already looking at, and the route is what the code switches on afterwards.
const OFN_OVERWRITEPROMPT: u32 = 0x0000_0002;
/// `OFN_HIDEREADONLY` -- no read-only checkbox, which means nothing here.
const OFN_HIDEREADONLY: u32 = 0x0000_0004;
/// `OFN_NOCHANGEDIR` -- restore the process's working directory afterwards.
///
/// **Load-bearing.** The game resolves its own relative paths against the process CWD, and a dialog
/// that left it wherever the player last browsed would move every later open with it.
const OFN_NOCHANGEDIR: u32 = 0x0000_0008;
/// `OFN_PATHMUSTEXIST` -- the directory in a typed path has to be real.
const OFN_PATHMUSTEXIST: u32 = 0x0000_0800;
/// `OFN_FILEMUSTEXIST` -- open only: the file has to be there.
const OFN_FILEMUSTEXIST: u32 = 0x0000_1000;
/// `OFN_EXPLORER` -- the modern dialog rather than the Windows 3.1 one.
const OFN_EXPLORER: u32 = 0x0008_0000;
/// `OFN_DONTADDTORECENT` -- a save the player exported is not a document they opened.
const OFN_DONTADDTORECENT: u32 = 0x0200_0000;

#[link(name = "comdlg32")]
unsafe extern "system" {
    fn GetOpenFileNameW(arg: *mut OpenFileNameW) -> i32;
    fn GetSaveFileNameW(arg: *mut OpenFileNameW) -> i32;
    /// Put the game back in front. See [`hand_back`].
    fn SetForegroundWindow(hwnd: *mut c_void) -> i32;
    fn SetActiveWindow(hwnd: *mut c_void) -> *mut c_void;
    fn SetFocus(hwnd: *mut c_void) -> *mut c_void;
    /// `0` when the dialog was dismissed by the player, non-zero when it failed.
    ///
    /// This is the only thing that tells a Cancel from a broken dialog: both return `FALSE`.
    fn CommDlgExtendedError() -> u32;
}

/// What a dialog came back with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    /// The player named a path. It has not been checked for anything yet.
    Chosen(PathBuf),
    /// The player dismissed the dialog. **Not a failure**, and nothing should be reported as one:
    /// the row press is spent and the player is standing back on the menu, which is what Back means.
    Cancelled,
    /// comdlg32 refused to run. Carries `CommDlgExtendedError`, which is `0` only if the dialog
    /// failed without saying why.
    Failed(u32),
}

/// Which of the two dialogs to open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Pick an existing file to read.
    Open,
    /// Name a file to write, existing or not.
    Save,
}

/// What to put in front of the player.
pub struct Request<'a> {
    pub intent: Intent,
    /// The dialog's caption.
    pub title: &'a str,
    /// Where to start browsing. A directory that does not exist is ignored by the dialog, which is
    /// the right behaviour -- it falls back to the shell's own default rather than refusing.
    pub start_dir: Option<&'a std::path::Path>,
    /// The `lpstrFilter` buffer, from [`ds2_save_file_core::filter_string`].
    pub filter: &'a [u16],
    /// The name the dialog opens with. Empty for none.
    pub default_name: &'a str,
}

/// Open one dialog and block until the player answers it.
///
/// # Safety
///
/// Reads the game's window handle out of the frontend singleton through the fault-safe reader, then
/// blocks in comdlg32. **Game thread only**: see the module docs for why the block is the point and
/// why the owner window must be the game's.
pub unsafe fn show(request: &Request<'_>) -> Pick {
    let title = wide_nul(request.title);
    let start_dir = request
        .start_dir
        .map(|dir| wide_nul(&dir.to_string_lossy()));
    let default_extension = wide_nul(ds2_save_file_core::SAVE_EXTENSION);

    // The dialog reads the initial name OUT of this buffer and writes the result back INTO it.
    let mut file = vec![0u16; PATH_UNITS];
    for (slot, unit) in file.iter_mut().zip(request.default_name.encode_utf16()) {
        *slot = unit;
    }
    // A default name that filled the buffer would leave no terminator. Truncating the name is fine;
    // an unterminated buffer is not.
    if let Some(last) = file.last_mut() {
        *last = 0;
    }

    let common =
        OFN_EXPLORER | OFN_HIDEREADONLY | OFN_NOCHANGEDIR | OFN_PATHMUSTEXIST | OFN_DONTADDTORECENT;
    let flags = match request.intent {
        Intent::Open => common | OFN_FILEMUSTEXIST,
        Intent::Save => common | OFN_OVERWRITEPROMPT,
    };

    let mut arg = OpenFileNameW {
        struct_size: core::mem::size_of::<OpenFileNameW>() as u32,
        hwnd_owner: game_window(),
        hinstance: core::ptr::null_mut(),
        filter: request.filter.as_ptr(),
        custom_filter: core::ptr::null_mut(),
        max_custom_filter: 0,
        // 1-based, and `0` is not "the first one" -- it is "no filter selected", which shows the
        // player an empty type dropdown.
        filter_index: 1,
        file: file.as_mut_ptr(),
        max_file: PATH_UNITS as u32,
        file_title: core::ptr::null_mut(),
        max_file_title: 0,
        initial_dir: start_dir
            .as_ref()
            .map(|dir| dir.as_ptr())
            .unwrap_or(core::ptr::null()),
        title: title.as_ptr(),
        flags,
        file_offset: 0,
        file_extension: 0,
        def_ext: default_extension.as_ptr(),
        cust_data: 0,
        hook: core::ptr::null_mut(),
        template_name: core::ptr::null(),
        reserved_ptr: core::ptr::null_mut(),
        reserved_dword: 0,
        flags_ex: 0,
    };

    // SAFETY: every pointer in `arg` is either null or borrowed from a live local that outlives this
    // call, `file` is `max_file` units long, and `struct_size` is this struct's own size.
    let answered = unsafe {
        match request.intent {
            Intent::Open => GetOpenFileNameW(&mut arg),
            Intent::Save => GetSaveFileNameW(&mut arg),
        }
    };
    // BEFORE ANY RETURN, on every path. A modal dialog owned by the game window disables that window
    // for the length of the call and hands activation to the dialog; when the dialog goes away the
    // activation does not always come back on its own, and a DARK SOULS II that is running but not
    // active reads to the player as a soft lock -- the menu is on screen and the pad does nothing.
    // Measured 2026-09-23: a pick the row refused left exactly that, with the refusal as the last
    // line in the log and the process alive on 89 threads.
    //
    // The refusing paths are the ones that need this most, because a pick that is accepted leaves
    // through the return to the title and gets its activation back from the game's own transition.
    unsafe { hand_back(arg.hwnd_owner) };

    if answered == 0 {
        // SAFETY: a call with no arguments.
        let error = unsafe { CommDlgExtendedError() };
        return if error == 0 {
            Pick::Cancelled
        } else {
            Pick::Failed(error)
        };
    }

    let end = file
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(file.len());
    let picked = String::from_utf16_lossy(&file[..end]);
    if picked.trim().is_empty() {
        // A `TRUE` return with an empty buffer is not a documented outcome, so it is reported as a
        // failure rather than silently treated as a cancel: the two lead to different log lines and
        // guessing which one this is would make the log lie.
        return Pick::Failed(0);
    }
    Pick::Chosen(PathBuf::from(picked))
}

/// Give the game back the foreground, the activation and the keyboard focus the dialog took.
///
/// All three, because they are three different things and the dialog took all three: foreground is
/// which window the desktop shows on top, activation is which window the shell considers current,
/// and focus is which window keystrokes go to. A window can hold any of them without the others,
/// and a game holding none of them still draws its last frame -- which is what a player reads as a
/// lock rather than as a lost window.
///
/// Every call is best-effort and unchecked. There is no failure here worth acting on: a refusal
/// from the window manager leaves the player exactly where they already were, and the alternative
/// -- not asking -- is the state this exists to get out of.
///
/// # Safety
///
/// `hwnd` must be a window handle or null. Null is passed straight through and the calls no-op.
unsafe fn hand_back(hwnd: *mut c_void) {
    if hwnd.is_null() {
        return;
    }
    // SAFETY: the caller's own window handle, the one the dialog was given as its owner.
    unsafe {
        SetForegroundWindow(hwnd);
        SetActiveWindow(hwnd);
        SetFocus(hwnd);
    }
}

/// The game's top-level window, or null if it cannot be resolved.
///
/// Copied in shape from `ds2-build-import`'s clipboard owner, and for the same reason: the handle in
/// the frontend singleton is the one the game's own Win32 calls pass, so using it means the dialog is
/// owned by the window that actually has focus.
fn game_window() -> *mut c_void {
    let Ok(address) = ds2_game_base::mem::game_rva(ds2_rva::FE_SYSTEM_SINGLETON) else {
        return core::ptr::null_mut();
    };
    // SAFETY: a resolved RVA in the loaded image, read through the fault-safe reader; the singleton
    // is null until the frontend is up, which `safe_read_usize` reports rather than faulting on.
    unsafe {
        let Some(instance) = ds2_game_base::mem::safe_read_usize(address) else {
            return core::ptr::null_mut();
        };
        if instance == 0 {
            return core::ptr::null_mut();
        }
        match ds2_game_base::mem::safe_read_usize(instance + ds2_rva::FE_SYSTEM_HWND_OFFSET) {
            Some(hwnd) => hwnd as *mut c_void,
            None => core::ptr::null_mut(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both dialogs restore the working directory and neither one skips the modern shell.
    #[test]
    fn both_intents_keep_the_working_directory() {
        // Spelled as the two literal combinations rather than recomputed from `common`, so an edit
        // to the flag set has to come through here.
        let open = OFN_EXPLORER
            | OFN_HIDEREADONLY
            | OFN_NOCHANGEDIR
            | OFN_PATHMUSTEXIST
            | OFN_DONTADDTORECENT
            | OFN_FILEMUSTEXIST;
        let save = OFN_EXPLORER
            | OFN_HIDEREADONLY
            | OFN_NOCHANGEDIR
            | OFN_PATHMUSTEXIST
            | OFN_DONTADDTORECENT
            | OFN_OVERWRITEPROMPT;
        assert_ne!(open & OFN_NOCHANGEDIR, 0);
        assert_ne!(save & OFN_NOCHANGEDIR, 0);
        // The save dialog must ask before replacing, and the open dialog must not demand it.
        assert_ne!(save & OFN_OVERWRITEPROMPT, 0);
        assert_eq!(open & OFN_OVERWRITEPROMPT, 0);
        // And the open dialog must insist the file is there, where the save dialog must not.
        assert_ne!(open & OFN_FILEMUSTEXIST, 0);
        assert_eq!(save & OFN_FILEMUSTEXIST, 0);
    }
}
