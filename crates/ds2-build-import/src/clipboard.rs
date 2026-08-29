//! Reading a link off the Windows clipboard.
//!
//! # Why this and not the game's own clipboard code
//!
//! DARK SOULS II already has a `CF_UNICODETEXT` reader -- `AppGUISystem::v12`, RVA `0x00b4ac70`,
//! whose `rcx` is never touched so it is callable with any `this`. It is not used here, for two
//! reasons that are both about what it CANNOT tell us:
//!
//! * **It cannot fail.** `mov eax,1` is on every path. `OpenClipboard` refusing, `GetClipboardData`
//!   returning null and an empty clipboard all skip the write and return `1`, leaving the caller's
//!   string exactly as it was. "Another process holds the clipboard" and "the clipboard still holds
//!   what it held last time" become the same answer, and a row that silently reuses the previous
//!   link is a bug that looks like the player's mistake.
//! * **Its out-parameter is a live `std::wstring`.** A forty-character link is far past
//!   [`ds2_rva::WSTRING_SSO_MAX`], so the assign allocates from the GAME's heap and this DLL would
//!   own that allocation -- freeing it with Rust's allocator is heap corruption whose crash names
//!   somebody else's function, and not freeing it leaks on every press.
//!
//! Thirty lines of `unsafe` gets an observable result at every step and a `Vec<u16>` this crate
//! owns outright. The game's version stays documented in `ds2-rva` so the next reader does not have
//! to re-derive why it was passed over.
//!
//! # The one hazard
//!
//! **A clipboard left open blocks every other process on the machine.** So `CloseClipboard` lives
//! in a guard's `Drop` rather than at the end of the happy path -- there are four early returns
//! here, and the one that matters is the one somebody adds later.

use core::ffi::c_void;

/// `CF_UNICODETEXT`. The same format the game's own reader asks for at `0x140b4ac8e`.
const CF_UNICODETEXT: u32 = 13;

/// The most UTF-16 units to take off the clipboard.
///
/// A clipboard can hold a document. This is a link, and everything past the bound is text this
/// process would otherwise copy and then throw away.
const MAX_UNITS: usize = 8 * 1024;

unsafe extern "system" {
    fn IsClipboardFormatAvailable(format: u32) -> i32;
    fn OpenClipboard(owner: *mut c_void) -> i32;
    fn CloseClipboard() -> i32;
    fn GetClipboardData(format: u32) -> *mut c_void;
    fn GlobalLock(handle: *mut c_void) -> *mut c_void;
    fn GlobalUnlock(handle: *mut c_void) -> i32;
}

/// An open clipboard, closed when it drops.
struct OpenBoard;

impl Drop for OpenBoard {
    fn drop(&mut self) {
        // SAFETY: only constructed after `OpenClipboard` returned non-zero, so this closes exactly
        // one successful open.
        unsafe { CloseClipboard() };
    }
}

impl OpenBoard {
    /// Open the clipboard against the game's own window, as the game's own reader does.
    ///
    /// `owner` is the game's `HWND`, read from the singleton at
    /// [`ds2_rva::FE_SYSTEM_HWND_OFFSET`] -- the same one `AppGUISystem::v12` passes. A null owner
    /// works too, but associating the open with the window that actually has focus is what the
    /// game does and there is no reason to differ.
    fn open(owner: *mut c_void) -> Option<Self> {
        // SAFETY: a plain Win32 call with no pointer arguments of ours.
        if unsafe { OpenClipboard(owner) } == 0 {
            return None;
        }
        Some(Self)
    }
}

/// The game's top-level window, or null if it cannot be resolved.
pub(crate) fn game_window() -> *mut c_void {
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

/// The clipboard's text, if it holds any.
///
/// `None` covers every way this can come to nothing -- no text on the clipboard, another process
/// holding it, an empty string, invalid UTF-16 -- because none of them is different from the
/// player's point of view and all of them mean "there is no link here".
pub(crate) fn text() -> Option<String> {
    // SAFETY: a query with no arguments of ours. Asked BEFORE opening, so a clipboard holding an
    // image is never opened at all -- an open is a lock on every other process's clipboard access.
    if unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) } == 0 {
        return None;
    }
    let _board = OpenBoard::open(game_window())?;

    // SAFETY: the handle belongs to the clipboard, not to us -- it must not be freed, and it stays
    // valid until `CloseClipboard`, which the guard defers to the end of this scope.
    let units = unsafe {
        let handle = GetClipboardData(CF_UNICODETEXT);
        if handle.is_null() {
            return None;
        }
        let locked = GlobalLock(handle) as *const u16;
        if locked.is_null() {
            return None;
        }
        let mut units = Vec::new();
        for index in 0..MAX_UNITS {
            let unit = *locked.add(index);
            if unit == 0 {
                break;
            }
            units.push(unit);
        }
        GlobalUnlock(handle);
        units
    };
    if units.is_empty() {
        return None;
    }
    String::from_utf16(&units).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The format asked for is the wide one the game itself asks for.
    #[test]
    fn the_format_is_the_one_the_game_reads() {
        assert_eq!(CF_UNICODETEXT, 13);
    }

    /// The bound is big enough for any link and small enough not to copy a document.
    #[test]
    fn the_bound_is_a_link_not_a_document() {
        assert!(MAX_UNITS > ds2_build_import_core::BUILD_URL_PREFIX.len() * 8);
        const { assert!(MAX_UNITS <= 64 * 1024) };
    }
}
