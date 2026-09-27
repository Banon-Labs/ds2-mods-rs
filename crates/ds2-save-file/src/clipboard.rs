//! The Windows clipboard, as Unicode text, for the picker's Ctrl+C, Ctrl+X and Ctrl+V.
//!
//! Under Wine this is the desktop's clipboard: a path copied in a Linux file manager pastes here.
//! Every failure -- the clipboard held by another window, no text on it -- is just nothing pasted
//! or nothing copied; a keystroke is not worth a log line.

use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};

/// `CF_UNICODETEXT`: NUL-terminated UTF-16.
const CF_UNICODETEXT: u32 = 13;

/// The clipboard's text, if it holds any.
pub(crate) fn read() -> Option<String> {
    // SAFETY: no owner window; the clipboard is closed again on every path below.
    unsafe { OpenClipboard(None) }.ok()?;
    let text = (|| {
        // SAFETY: the clipboard is open; the handle stays owned by the clipboard.
        let handle = unsafe { GetClipboardData(CF_UNICODETEXT) }.ok()?;
        let global = HGLOBAL(handle.0);
        // SAFETY: a CF_UNICODETEXT handle is a global memory block holding NUL-terminated UTF-16.
        let data = unsafe { GlobalLock(global) }.cast::<u16>();
        if data.is_null() {
            return None;
        }
        let mut len = 0usize;
        // SAFETY: reading up to the terminating NUL the format guarantees.
        while unsafe { *data.add(len) } != 0 {
            len += 1;
        }
        // SAFETY: `len` units were just read from this locked block.
        let text = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(data, len) });
        // SAFETY: unlocking the lock taken above.
        let _ = unsafe { GlobalUnlock(global) };
        Some(text)
    })();
    // SAFETY: closing the clipboard opened above.
    let _ = unsafe { CloseClipboard() };
    text
}

/// Put `text` on the clipboard, replacing what was there.
pub(crate) fn write(text: &str) {
    let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: no owner window; closed again below.
    if unsafe { OpenClipboard(None) }.is_err() {
        return;
    }
    // SAFETY: the clipboard is open. On success the block's ownership passes to the clipboard.
    unsafe {
        if EmptyClipboard().is_ok()
            && let Ok(global) = GlobalAlloc(GMEM_MOVEABLE, units.len() * size_of::<u16>())
        {
            let data = GlobalLock(global).cast::<u16>();
            if !data.is_null() {
                std::ptr::copy_nonoverlapping(units.as_ptr(), data, units.len());
                let _ = GlobalUnlock(global);
                let _ = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(global.0)));
            }
        }
        let _ = CloseClipboard();
    }
}
