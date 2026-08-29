//! Reading digits off the keyboard, for when there is no link to read anywhere else.
//!
//! # Why this exists at all
//!
//! The first choice was Steam's own text field, and on this machine it is simply not there:
//! `ShowGamepadTextInput` returns `false` on every press because the overlay is disabled. The
//! second choice was the clipboard, which works, but only answers the question "what did the
//! player already copy" -- and a player who has the build id in their head and nothing in their
//! clipboard had no way in at all. This is the third choice: they type it.
//!
//! # It reads TEN KEYS, and that is the whole design
//!
//! Only `0`-`9` (top row and numpad alike) and backspace. Not Enter, not Escape, not letters.
//!
//! That is not a shortcut, it is the thing that makes this safe to ship WITHOUT a suppression
//! hook. **This mod cannot stop a key from reaching the game.** DARK SOULS II reads the keyboard
//! through DirectInput's `GetDeviceState`, not through the window message queue, so there is no
//! message to swallow -- suppressing a key would mean wrapping the game's own DirectInput device,
//! which is a hook nobody here has written or tested. So instead of suppressing keys, this takes
//! only keys the game does not want:
//!
//! * The digits are unbound in DARK SOULS II's default keyboard layout, so typing a build id
//!   presses nothing.
//! * **Enter and Escape are deliberately NOT read**, because the pause menu owns both -- Enter
//!   confirms the highlighted row and Escape backs out. Reading them would mean this field and
//!   the menu underneath it both acting on one keypress. So submission is the ROW PRESS, which is
//!   what Enter does anyway by way of the game's own menu, and cancelling is pressing the row with
//!   an empty field.
//!
//! **The honest limit**: a player who has rebound a digit to something will fire that something
//! while typing. Nothing here can prevent that, and pretending otherwise is worse than saying it.
//!
//! # Edges, not levels, and only while the game has focus
//!
//! `GetAsyncKeyState` reports whether a key is DOWN, so it reports the same key on every frame it
//! is held; the previous frame's state is kept here and only the transitions become characters.
//! And it reports that key whatever window has focus -- so a player who alt-tabs away to read the
//! build id off a browser would otherwise type the browser's keystrokes into this field. Every
//! poll is gated on the foreground window being the game's own.

use core::ffi::c_void;

unsafe extern "system" {
    fn GetAsyncKeyState(key: i32) -> i16;
    fn GetForegroundWindow() -> *mut c_void;
}

/// `VK_BACK`, which is also the `WM_CHAR` unit for it -- so it needs no translation.
const VK_BACK: i32 = 0x08;

/// The keys this reads, paired with the `WM_CHAR` unit each produces.
///
/// Digits appear twice because `VK_0`-`VK_9` (`0x30`-`0x39`) and `VK_NUMPAD0`-`VK_NUMPAD9`
/// (`0x60`-`0x69`) are different virtual keys for the same character, and a player typing a number
/// reaches for whichever is nearer.
const WATCHED: [(i32, u16); 21] = [
    (VK_BACK, VK_BACK as u16),
    (0x30, b'0' as u16),
    (0x31, b'1' as u16),
    (0x32, b'2' as u16),
    (0x33, b'3' as u16),
    (0x34, b'4' as u16),
    (0x35, b'5' as u16),
    (0x36, b'6' as u16),
    (0x37, b'7' as u16),
    (0x38, b'8' as u16),
    (0x39, b'9' as u16),
    (0x60, b'0' as u16),
    (0x61, b'1' as u16),
    (0x62, b'2' as u16),
    (0x63, b'3' as u16),
    (0x64, b'4' as u16),
    (0x65, b'5' as u16),
    (0x66, b'6' as u16),
    (0x67, b'7' as u16),
    (0x68, b'8' as u16),
    (0x69, b'9' as u16),
];

/// Whether each watched key was down last poll. Indexes match [`WATCHED`].
///
/// A plain `static mut` rather than an atomic because [`poll`] runs only on the game thread, from
/// the menu-row tick, and nothing else touches it.
static mut PREVIOUS: [bool; WATCHED.len()] = [false; WATCHED.len()];

/// Whether the game's window is the one the player is typing into.
///
/// A null `HWND` from the game -- before the window exists, or if the singleton moves -- reads as
/// NOT focused, so the field goes deaf rather than eating a stranger's keystrokes.
fn game_has_focus() -> bool {
    let window = crate::clipboard::game_window();
    // SAFETY: a plain Win32 call with no arguments.
    !window.is_null() && unsafe { GetForegroundWindow() } == window
}

/// The `WM_CHAR` units for keys pressed since the last poll, in [`WATCHED`] order.
///
/// **Call once per frame.** Calling it twice in one frame is harmless but the second call sees no
/// edges, and skipping frames loses keys pressed and released between them.
///
/// # Safety
///
/// Game thread only -- it keeps unsynchronised state between calls.
pub(crate) unsafe fn poll() -> Vec<u16> {
    let focused = game_has_focus();
    let mut out = Vec::new();
    for (index, (key, unit)) in WATCHED.iter().enumerate() {
        // SAFETY: a plain Win32 call taking an integer.
        //
        // The HIGH bit is "down now"; the low bit is "was pressed since the last call" and is
        // deliberately unused -- it would report presses made while another window had focus, and
        // it is per-caller state that the game's own polling would race us for.
        let down = focused && unsafe { GetAsyncKeyState(*key) } < 0;
        // SAFETY: game thread only, per this function's contract.
        let was_down = unsafe { core::ptr::addr_of_mut!(PREVIOUS[index]).replace(down) };
        if down && !was_down {
            out.push(*unit);
        }
    }
    out
}

/// Sample the keyboard WITHOUT emitting anything, so the next poll reports only new presses.
///
/// Called when a field OPENS. [`PREVIOUS`] otherwise holds whatever the last session left in it,
/// and -- more importantly -- a key the player is holding down at the moment the field opens was
/// pressed AT THE GAME, not at this field. Seeding the state with what is down right now makes
/// that key produce nothing until it is released and pressed again.
///
/// # Safety
///
/// Game thread only, same as [`poll`].
pub(crate) unsafe fn sync_to_current() {
    let focused = game_has_focus();
    for (index, (key, _)) in WATCHED.iter().enumerate() {
        // SAFETY: a plain Win32 call taking an integer.
        let down = focused && unsafe { GetAsyncKeyState(*key) } < 0;
        // SAFETY: game thread only, per this function's contract.
        unsafe { core::ptr::addr_of_mut!(PREVIOUS[index]).write(down) };
    }
}
