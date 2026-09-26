//! A small modal Win32 dialog with one edit control, for when Steam will not draw a field.
//!
//! # Where it sits
//!
//! The row's confirm tries Steam's text field first. When Steam declines -- a desktop Steam outside
//! Big Picture always does -- this opens instead: a caption, one line of text, OK and Cancel. The
//! player types or pastes a soulsplanner link; Enter is OK and Escape is Cancel, both mapped by the
//! dialog manager rather than by code here.
//!
//! The template, the prefill rule and the reading of the answer are host-tested in
//! `ds2_build_url_core::dialog`. What is left here is the Win32 calls around them.
//!
//! # It blocks the game thread, the same way `ds2-save-file`'s file dialog does
//!
//! Opened inline from the row's confirm, on the game thread, owned by the game's own window. The
//! dialog's message loop runs and the game's frame does not, which is what keeps the game's own
//! input from leaking: nothing polls DirectInput while the dialog is up, so keys typed into it
//! never reach the menu underneath. The owner is there so the dialog appears in front of a
//! fullscreen game; an unowned modal behind the swapchain would read as a freeze.
//!
//! # The key that closed it is still down when it returns
//!
//! The dialog manager acts on key-down, so Enter or Escape is still held when the modal returns.
//! The game reads the keyboard through DirectInput, which reports held keys, and an Enter the pause
//! menu sees as a fresh press would confirm the row again and reopen this dialog; an Escape would
//! close the menu. So before returning, the game thread waits (bounded) until those keys are up.
//! The same wait runs before opening, and queued keyboard messages are dropped, so the press that
//! confirmed the row cannot arrive in the dialog as an instant OK. Both are reasoning, not
//! measurement: whether the menu would have seen either edge has not been observed in the game.
//!
//! # Not covered
//!
//! The crash logger's hang watchdog times the game thread, and a dialog left open long enough will
//! be reported as a stall. `ds2-save-file`'s dialog has the same exposure; the fix for both is a
//! modal marker in `ds2-game-base` that is not on main yet.

use core::ffi::c_void;

use ds2_build_url_core::dialog::{ID_CANCEL, ID_EDIT, ID_OK, TEXT_LIMIT, template};

/// `WM_INITDIALOG`.
const WM_INITDIALOG: u32 = 0x0110;
/// `WM_COMMAND`.
const WM_COMMAND: u32 = 0x0111;
/// `EM_SETSEL`.
const EM_SETSEL: u32 = 0x00B1;
/// `EM_LIMITTEXT`.
const EM_LIMITTEXT: u32 = 0x00C5;
/// `WM_KEYFIRST`..`WM_KEYLAST`: every keyboard message, including `WM_CHAR`.
const WM_KEYFIRST: u32 = 0x0100;
const WM_KEYLAST: u32 = 0x0109;
/// `PM_REMOVE`.
const PM_REMOVE: u32 = 0x0001;
/// `DWLP_USER` on x64: `DWLP_MSGRESULT` (8 bytes) then `DWLP_DLGPROC` (8 bytes) come first.
const DWLP_USER: i32 = 16;

/// `VK_RETURN`, `VK_ESCAPE` and `VK_SPACE`: the keys that confirm or back out of a menu or a dialog.
const SETTLE_KEYS: [i32; 3] = [0x0D, 0x1B, 0x20];

/// How long to wait for those keys to come up. A tap is well under this; a key held past it is the
/// player's choice and is let through rather than stalling the game.
const SETTLE_LIMIT: std::time::Duration = std::time::Duration::from_millis(1000);
const SETTLE_STEP_MS: u32 = 10;

/// `MSG`, x64 layout, only ever written by `PeekMessageW` and thrown away.
#[repr(C)]
struct Msg {
    hwnd: *mut c_void,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    point: [i32; 2],
    private: u32,
}

const _: () = assert!(core::mem::size_of::<Msg>() == 48);

type DlgProc = unsafe extern "system" fn(*mut c_void, u32, usize, isize) -> isize;

#[link(name = "user32")]
unsafe extern "system" {
    fn DialogBoxIndirectParamW(
        instance: *mut c_void,
        template: *const c_void,
        owner: *mut c_void,
        proc_: DlgProc,
        init: isize,
    ) -> isize;
    fn EndDialog(dialog: *mut c_void, result: isize) -> i32;
    fn GetDlgItem(dialog: *mut c_void, id: i32) -> *mut c_void;
    fn SetWindowTextW(window: *mut c_void, text: *const u16) -> i32;
    fn GetWindowTextLengthW(window: *mut c_void) -> i32;
    fn GetWindowTextW(window: *mut c_void, text: *mut u16, max: i32) -> i32;
    fn SendMessageW(window: *mut c_void, message: u32, wparam: usize, lparam: isize) -> isize;
    fn SetWindowLongPtrW(window: *mut c_void, index: i32, value: isize) -> isize;
    fn GetWindowLongPtrW(window: *mut c_void, index: i32) -> isize;
    fn PeekMessageW(msg: *mut Msg, window: *mut c_void, first: u32, last: u32, remove: u32) -> i32;
    fn GetAsyncKeyState(key: i32) -> i16;
    fn SetForegroundWindow(window: *mut c_void) -> i32;
    fn SetActiveWindow(window: *mut c_void) -> *mut c_void;
    fn SetFocus(window: *mut c_void) -> *mut c_void;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn GetLastError() -> u32;
    fn Sleep(milliseconds: u32);
}

/// What the dialog came back with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// OK, with the edit control's text exactly as it stood. Not yet cleaned or checked.
    Entered(String),
    /// Cancel, Escape or the close box.
    Cancelled,
    /// The dialog did not open. Carries `GetLastError`.
    Failed(u32),
}

/// What the dialog procedure reads and writes, reached through `DWLP_USER`.
struct State {
    /// The edit control's opening text, terminated.
    prefill: Vec<u16>,
    /// Filled on OK.
    entered: Option<String>,
}

/// Open the dialog with `prefill` in the edit control and block until the player answers it.
///
/// # Safety
///
/// Game thread only, from the row's confirm. The block is the point: see the module docs.
pub(crate) unsafe fn ask(prefill: &str) -> Answer {
    // The template has to start on a four-byte boundary, and so does every control inside it, so
    // the bytes are copied into `u32`s rather than handed over from a `Vec<u8>`.
    let bytes = template();
    let aligned: Vec<u32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| u32::from_le_bytes(*chunk))
        .collect();

    let mut state = State {
        prefill: prefill.encode_utf16().chain([0]).collect(),
        entered: None,
    };
    let owner = crate::clipboard::game_window();

    // SAFETY: plain Win32 calls on this thread's own queue and the async key table.
    unsafe {
        settle_keys();
        drop_queued_keys();
    }

    // SAFETY: `aligned` is a complete template built and tested by `ds2_build_url_core::dialog`,
    // and outlives the call. `state` outlives the call too, and the procedure is the only reader of
    // the pointer passed as the init parameter. `owner` is the game's window or null.
    let result = unsafe {
        DialogBoxIndirectParamW(
            GetModuleHandleW(core::ptr::null()),
            aligned.as_ptr().cast(),
            owner,
            dialog_proc,
            (&raw mut state) as isize,
        )
    };
    let error = if result <= 0 {
        // SAFETY: a plain Win32 call, made before anything else can set the thread's last error.
        unsafe { GetLastError() }
    } else {
        0
    };

    // Before any return, on every path. A modal owned by the game window disables it for the
    // length of the call, and activation does not always come back on its own when the dialog goes
    // away; a game that is running but not active reads to the player as a soft lock. This is the
    // same hand-back `ds2-save-file`'s dialog does, for the same measured reason.
    // SAFETY: `owner` is the handle the dialog was given, or null, which `hand_back` skips.
    unsafe {
        hand_back(owner);
        settle_keys();
        drop_queued_keys();
    }

    // `-1` is a failed create; `0` is what the call returns for an invalid owner. Neither is a
    // value the procedure ever passes to `EndDialog`.
    match result {
        r if r == isize::from(ID_OK as i16) => {
            Answer::Entered(state.entered.take().unwrap_or_default())
        }
        r if r == isize::from(ID_CANCEL as i16) => Answer::Cancelled,
        _ => Answer::Failed(error),
    }
}

/// The dialog procedure: fill the edit control, then answer OK and Cancel.
///
/// # Safety
///
/// Called by the dialog manager only, with the `State` pointer `ask` passed as the init parameter.
unsafe extern "system" fn dialog_proc(
    dialog: *mut c_void,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    match message {
        WM_INITDIALOG => {
            // SAFETY: `lparam` is the `State` pointer `ask` passed; it outlives the dialog.
            unsafe {
                SetWindowLongPtrW(dialog, DWLP_USER, lparam);
                let state = &*(lparam as *const State);
                let edit = GetDlgItem(dialog, i32::from(ID_EDIT));
                SendMessageW(edit, EM_LIMITTEXT, TEXT_LIMIT, 0);
                SetWindowTextW(edit, state.prefill.as_ptr());
                // Caret after the prefill, so the first key typed appends the build id.
                let end = state.prefill.len().saturating_sub(1) as isize;
                SendMessageW(edit, EM_SETSEL, end as usize, end);
                SetFocus(edit);
            }
            // `FALSE`: focus was set here, so the dialog manager must not move it.
            0
        }
        WM_COMMAND => {
            let id = (wparam & 0xFFFF) as u16;
            if id == ID_OK {
                // SAFETY: `DWLP_USER` was set to the live `State` in `WM_INITDIALOG`, which the
                // dialog manager always sends before any command.
                unsafe {
                    let state = GetWindowLongPtrW(dialog, DWLP_USER) as *mut State;
                    let edit = GetDlgItem(dialog, i32::from(ID_EDIT));
                    if let Some(state) = state.as_mut() {
                        state.entered = Some(window_text(edit));
                    }
                    EndDialog(dialog, isize::from(ID_OK as i16));
                }
                1
            } else if id == ID_CANCEL {
                // SAFETY: the dialog handle the manager passed.
                unsafe { EndDialog(dialog, isize::from(ID_CANCEL as i16)) };
                1
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// A window's text.
///
/// # Safety
///
/// `window` must be a window handle or null.
unsafe fn window_text(window: *mut c_void) -> String {
    // SAFETY: forwarded to the caller.
    let length = unsafe { GetWindowTextLengthW(window) }.max(0) as usize;
    let mut buffer = vec![0u16; length + 1];
    // SAFETY: `buffer` is `length + 1` units long, and that is the bound passed.
    let copied = unsafe { GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32) };
    buffer.truncate(copied.max(0) as usize);
    String::from_utf16_lossy(&buffer)
}

/// Wait, bounded by [`SETTLE_LIMIT`], until none of [`SETTLE_KEYS`] is held.
///
/// # Safety
///
/// Game thread; it sleeps the thread it runs on.
unsafe fn settle_keys() {
    let deadline = std::time::Instant::now() + SETTLE_LIMIT;
    loop {
        // SAFETY: a plain Win32 call taking an integer. The high bit is "down now".
        let held = SETTLE_KEYS
            .iter()
            .any(|key| unsafe { GetAsyncKeyState(*key) } < 0);
        if !held || std::time::Instant::now() >= deadline {
            return;
        }
        // SAFETY: a plain Win32 call.
        unsafe { Sleep(SETTLE_STEP_MS) };
    }
}

/// Drop every keyboard message queued for this thread.
///
/// # Safety
///
/// Game thread. The game reads the keyboard through DirectInput, so what is dropped here is a key
/// message nothing else was waiting on, not input the game needed.
unsafe fn drop_queued_keys() {
    let mut msg = Msg {
        hwnd: core::ptr::null_mut(),
        message: 0,
        wparam: 0,
        lparam: 0,
        time: 0,
        point: [0; 2],
        private: 0,
    };
    // SAFETY: `msg` is a live `MSG`-sized local; a null window means every window of this thread.
    while unsafe {
        PeekMessageW(
            &raw mut msg,
            core::ptr::null_mut(),
            WM_KEYFIRST,
            WM_KEYLAST,
            PM_REMOVE,
        )
    } != 0
    {}
}

/// Give the game back the foreground, the activation and the keyboard focus the dialog took.
///
/// # Safety
///
/// `window` must be a window handle or null. Null is skipped.
unsafe fn hand_back(window: *mut c_void) {
    if window.is_null() {
        return;
    }
    // SAFETY: the game's own window handle, the one the dialog was given as its owner.
    unsafe {
        SetForegroundWindow(window);
        SetActiveWindow(window);
        SetFocus(window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ids the procedure passes to `EndDialog` are the ones `ask` matches on, and neither is a
    /// failure value.
    #[test]
    fn the_end_codes_are_distinct_from_failure() {
        for code in [ID_OK, ID_CANCEL] {
            let code = isize::from(code as i16);
            assert!(code > 0);
        }
        assert_ne!(ID_OK, ID_CANCEL);
    }

    /// The settle wait is short enough not to read as a hang and long enough to outlast a tap.
    #[test]
    fn the_settle_wait_is_bounded() {
        assert!(SETTLE_LIMIT.as_millis() >= 200);
        assert!(SETTLE_LIMIT.as_millis() <= 2000);
        assert!(u128::from(SETTLE_STEP_MS) < SETTLE_LIMIT.as_millis());
    }
}
