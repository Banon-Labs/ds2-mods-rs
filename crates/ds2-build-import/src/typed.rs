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
//! # It reads TWELVE KEYS, and that is the whole design
//!
//! `0`-`9`, backspace and Enter. Not Escape, not letters.
//!
//! **Every digit is watched twice.** `VK_0`-`VK_9` (`0x30`-`0x39`) and `VK_NUMPAD0`-`VK_NUMPAD9`
//! (`0x60`-`0x69`) are different virtual keys producing the same character, and a player reaches
//! for whichever is nearer. Enter needs no such pair: the main and numpad Enter keys share
//! `VK_RETURN`, differing only in an extended-key bit that lives in a message's `lParam` --
//! and [`GetAsyncKeyState`] has no `lParam`, so one entry covers both.
//!
//! The short list is not a shortcut, it is the thing that makes this safe to ship WITHOUT a
//! suppression hook. **This mod cannot stop a key from reaching the game.** DARK SOULS II reads
//! the keyboard through DirectInput's `GetDeviceState`, not through the window message queue, so
//! there is no message to swallow -- suppressing a key would mean wrapping the game's own
//! DirectInput device, which is a hook nobody here has written or tested. So instead of
//! suppressing keys, this takes only keys whose second effect is harmless:
//!
//! * The digits are unbound in DARK SOULS II's default keyboard layout, so typing a build id
//!   presses nothing.
//! * **Enter is read even though the pause menu may also confirm the row with it**, and the
//!   duplicate cannot fire twice. Whichever half acts first claims `SESSION_OPEN` and takes the
//!   typing session with it; the other then finds no session and a claim it cannot make, and does
//!   nothing. That holds in BOTH orders, so it does not depend on knowing which arrives first --
//!   which is worth saying plainly, because whether this game binds Enter to menu-confirm at all
//!   has not been established here.
//! * **Escape is still NOT read.** The menu backs out on it, and a cancel that also closes the
//!   menu is a different behaviour from the one the row documents. Pressing the row with an empty
//!   field remains the cancel.
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
    fn GetWindowThreadProcessId(window: *mut c_void, process: *mut u32) -> u32;
    fn GetCurrentProcessId() -> u32;
}

/// `VK_BACK`, which is also the `WM_CHAR` unit for it -- so it needs no translation.
const VK_BACK: i32 = 0x08;

/// `VK_RETURN`, which is also its own `WM_CHAR` unit, and which BOTH Enter keys produce.
///
/// The numpad's Enter is not a separate virtual key. It is distinguished from the main one only by
/// the extended-key bit in a keyboard message's `lParam`, and [`GetAsyncKeyState`] takes a virtual
/// key and no `lParam` -- so watching this one value covers both, and there is no numpad twin to
/// add here the way there is for the digits.
const VK_RETURN: i32 = 0x0D;

/// The keys this reads, paired with the `WM_CHAR` unit each produces.
///
/// Digits appear twice because `VK_0`-`VK_9` (`0x30`-`0x39`) and `VK_NUMPAD0`-`VK_NUMPAD9`
/// (`0x60`-`0x69`) are different virtual keys for the same character, and a player typing a number
/// reaches for whichever is nearer.
const WATCHED: [(i32, u16); 22] = [
    (VK_BACK, VK_BACK as u16),
    (VK_RETURN, VK_RETURN as u16),
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

/// Whether the foreground window belongs to THIS PROCESS.
///
/// # It asks about the process, not about one window, and the first version got that wrong
///
/// The first version compared `GetForegroundWindow()` against the `HWND` in the game's own
/// singleton and read keys only when they were equal. That is an equality nobody checked: a
/// process has many windows, the one the singleton records is not necessarily the one the
/// compositor considers foreground, and under Proton there is a Wine frame in between.
///
/// **That equality turned out to HOLD** -- typing worked in the game on the first run. This is not
/// a bug fix, it is the removal of an assumption that happened to be true: nothing had checked it,
/// and a gate that is false on every frame makes the field silently deaf.
///
/// The question the field actually needs answered is "is the player typing at US" and the process
/// is the right granularity for it. Any window of ours having focus means the keystroke was aimed
/// here; which window it was is not the field's business.
fn game_has_focus() -> bool {
    // SAFETY: a plain Win32 call with no arguments.
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_null() {
        return false;
    }
    let mut owner = 0u32;
    // SAFETY: `foreground` is a window handle Win32 just returned, and `owner` is a live `u32`.
    unsafe { GetWindowThreadProcessId(foreground, &raw mut owner) };
    // SAFETY: a plain Win32 call with no arguments.
    owner != 0 && owner == unsafe { GetCurrentProcessId() }
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
