//! What the player pressed this frame, for the link panel: keyboard and pad, as edges.
//!
//! The same reader `ds2-save-file`'s picker uses (`picker_input.rs` there), with the key table a
//! link needs instead of the one a path needs: `?`, `=`, `#` and `&` as well as `:` `/` `.` `-`
//! `_`, and Home, End and Delete for moving about in forty characters of URL. It reads around the
//! game's own device polls -- the keyboard through `GetAsyncKeyState`, the pad through
//! `XInputGetState` on the `XINPUT1_3.dll` the game already has loaded -- because while the panel
//! is up the input harness's `hold` blanks every device the game reads, and that blanking must not
//! blind the panel as well.
//!
//! A copy rather than a shared module for the reason `ds2-build-recommender-ui`'s copy gives: the
//! picker's is private to its crate, and each panel's key table is its own. Fixed virtual keys, not
//! the keyboard layout, as the picker explains.

use hudhook::windows::Win32::Foundation::HMODULE;
use hudhook::windows::Win32::System::LibraryLoader::{
    GetModuleHandleA, GetProcAddress, LoadLibraryA,
};
use hudhook::windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use hudhook::windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId,
};
use hudhook::windows::core::s;

/// One press, already debounced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Press {
    /// Enter or A: load the link.
    Confirm,
    /// Escape, B or Start: close the panel.
    Close,
    Left,
    Right,
    Home,
    End,
    Backspace,
    Delete,
    /// Ctrl+Backspace or X: empty the field.
    Clear,
    /// Ctrl+V, Shift+Insert or Y: the clipboard into the field.
    Paste,
    Char(char),
}

/// Frames a held key waits before it starts repeating, at the game's 60.
const REPEAT_DELAY_FRAMES: u32 = 24;

/// Frames between repeats once it does.
const REPEAT_EVERY_FRAMES: u32 = 4;

const VK_BACK: i32 = 0x08;
const VK_RETURN: i32 = 0x0d;
const VK_SHIFT: i32 = 0x10;
const VK_CONTROL: i32 = 0x11;
const VK_ESCAPE: i32 = 0x1b;
const VK_END: i32 = 0x23;
const VK_HOME: i32 = 0x24;
const VK_LEFT: i32 = 0x25;
const VK_RIGHT: i32 = 0x27;
const VK_INSERT: i32 = 0x2d;
const VK_DELETE: i32 = 0x2e;
const VK_OEM_1: i32 = 0xba; // ; :
const VK_OEM_PLUS: i32 = 0xbb; // = +
const VK_OEM_MINUS: i32 = 0xbd; // - _
const VK_OEM_PERIOD: i32 = 0xbe; // .
const VK_OEM_2: i32 = 0xbf; // / ?

/// The editing keys and what each one means. Backspace is read separately, because Ctrl turns it
/// into [`Press::Clear`].
const NAV_KEYS: [(i32, Press); 7] = [
    (VK_RETURN, Press::Confirm),
    (VK_ESCAPE, Press::Close),
    (VK_LEFT, Press::Left),
    (VK_RIGHT, Press::Right),
    (VK_HOME, Press::Home),
    (VK_END, Press::End),
    (VK_DELETE, Press::Delete),
];

/// Punctuation a link needs, unshifted and shifted. `None` is a shifted symbol no link uses.
const SYMBOL_KEYS: [(i32, char, Option<char>); 5] = [
    (VK_OEM_1, ';', Some(':')),
    (VK_OEM_PLUS, '=', Some('+')),
    (VK_OEM_MINUS, '-', Some('_')),
    (VK_OEM_PERIOD, '.', None),
    (VK_OEM_2, '/', Some('?')),
];

/// What Shift makes of each digit on a US layout, where a link has a use for it. `#` is here so
/// the fragment form can be typed and then refused with its reason, rather than silently eaten.
const SHIFTED_DIGITS: [Option<char>; 10] = [
    None,      // 0 )
    None,      // 1 !
    None,      // 2 @
    Some('#'), // 3
    None,      // 4 $
    Some('%'), // 5
    None,      // 6 ^
    Some('&'), // 7
    None,      // 8 *
    None,      // 9 (
];

/// XInput's button bits, from `XINPUT_GAMEPAD`.
const PAD_LEFT: u16 = 0x0004;
const PAD_RIGHT: u16 = 0x0008;
const PAD_START: u16 = 0x0010;
const PAD_A: u16 = 0x1000;
const PAD_B: u16 = 0x2000;
const PAD_X: u16 = 0x4000;
const PAD_Y: u16 = 0x8000;

/// The pad buttons and what each one means. A pad cannot type, so it pastes and loads.
const PAD_BUTTONS: [(u16, Press); 7] = [
    (PAD_A, Press::Confirm),
    (PAD_B, Press::Close),
    (PAD_START, Press::Close),
    (PAD_Y, Press::Paste),
    (PAD_X, Press::Clear),
    (PAD_LEFT, Press::Left),
    (PAD_RIGHT, Press::Right),
];

/// `XINPUT_STATE`: a packet number, then `XINPUT_GAMEPAD`, whose first field is the button word.
#[repr(C)]
#[derive(Default)]
struct XInputState {
    packet: u32,
    buttons: u16,
    left_trigger: u8,
    right_trigger: u8,
    thumbs: [i16; 4],
}

type XInputGetStateFn = unsafe extern "system" fn(u32, *mut XInputState) -> u32;

/// Frames each key or button has been held, indexed by a slot per key. `0` is up.
pub(crate) struct Reader {
    held: Vec<u32>,
    xinput: Option<XInputGetStateFn>,
    /// Whether the last press came from the pad, so the key help names pad buttons.
    pad_last: bool,
}

impl Reader {
    /// A reader with nothing held, looking up `XInputGetState` once.
    pub(crate) fn new() -> Self {
        Self {
            // Nav, Backspace, Insert, V, the letters, the digits, the symbols, the pad.
            held: vec![0; NAV_KEYS.len() + 2 + 26 + 10 + SYMBOL_KEYS.len() + PAD_BUTTONS.len()],
            xinput: xinput_get_state(),
            pad_last: false,
        }
    }

    /// Whether the player last pressed something on the pad rather than the keyboard.
    pub(crate) fn pad_last(&self) -> bool {
        self.pad_last
    }

    /// Everything pressed since the last call, in a fixed order. Nothing while another window has
    /// the keyboard, so a player typing elsewhere -- in the browser they copied the link from --
    /// does not type into the panel.
    pub(crate) fn poll(&mut self) -> Vec<Press> {
        let mut pressed = Vec::new();
        if !game_has_focus() {
            self.held.iter_mut().for_each(|held| *held = 0);
            return pressed;
        }
        let shift = key_down(VK_SHIFT);
        let ctrl = key_down(VK_CONTROL);
        let buttons = self.pad_buttons();
        let mut slot = 0usize;
        let mut step = |down: bool, press: Press, repeats: bool, held: &mut Vec<u32>| {
            let frames = &mut held[slot];
            slot += 1;
            if !down {
                *frames = 0;
                return;
            }
            *frames = frames.saturating_add(1);
            let fire = *frames == 1
                || (repeats
                    && *frames > REPEAT_DELAY_FRAMES
                    && (*frames - REPEAT_DELAY_FRAMES).is_multiple_of(REPEAT_EVERY_FRAMES));
            if fire {
                pressed.push(press);
            }
        };
        for (vk, press) in NAV_KEYS {
            let repeats = !matches!(press, Press::Confirm | Press::Close);
            step(key_down(vk), press, repeats, &mut self.held);
        }
        let backspace = if ctrl { Press::Clear } else { Press::Backspace };
        step(
            key_down(VK_BACK),
            backspace,
            backspace == Press::Backspace,
            &mut self.held,
        );
        step(
            key_down(VK_INSERT) && shift,
            Press::Paste,
            false,
            &mut self.held,
        );
        for (offset, letter) in ('a'..='z').enumerate() {
            let down = key_down(0x41 + offset as i32);
            if ctrl {
                // With Ctrl held a letter is a shortcut and the only one here is V. The letter's
                // own slot, so letting go of Ctrl with V still down does not type a `v` as well.
                step(down && letter == 'v', Press::Paste, false, &mut self.held);
                continue;
            }
            let typed = if shift {
                letter.to_ascii_uppercase()
            } else {
                letter
            };
            step(down, Press::Char(typed), true, &mut self.held);
        }
        for (offset, digit) in ('0'..='9').enumerate() {
            let typed = if shift {
                SHIFTED_DIGITS[offset]
            } else {
                Some(digit)
            };
            let down = key_down(0x30 + offset as i32) && typed.is_some() && !ctrl;
            step(
                down,
                Press::Char(typed.unwrap_or(digit)),
                true,
                &mut self.held,
            );
        }
        for (vk, plain, shifted) in SYMBOL_KEYS {
            let typed = if shift { shifted } else { Some(plain) };
            let down = key_down(vk) && typed.is_some() && !ctrl;
            step(
                down,
                Press::Char(typed.unwrap_or(plain)),
                true,
                &mut self.held,
            );
        }
        for (bit, press) in PAD_BUTTONS {
            let repeats = matches!(press, Press::Left | Press::Right);
            step(buttons & bit != 0, press, repeats, &mut self.held);
        }
        // A press this frame with a pad button down came from the pad; any other, the keyboard.
        if !pressed.is_empty() {
            self.pad_last = PAD_BUTTONS.iter().any(|(bit, _)| buttons & bit != 0);
        }
        pressed
    }

    /// Mark everything currently down as already held, so none of it fires on the next poll: the
    /// Enter or A that pressed the row is still down on the frame the panel opens.
    pub(crate) fn swallow_held(&mut self) {
        let _ = self.poll();
    }

    /// Whether any key or button this reader watches is down. The panel keeps the hold for as long
    /// as this is true after it closes, so the press that closed it never reaches the pause menu.
    pub(crate) fn anything_down(&mut self) -> bool {
        self.pad_buttons() != 0
            || NAV_KEYS.iter().any(|(vk, _)| key_down(*vk))
            || key_down(VK_BACK)
            || (0x30..=0x5a).any(key_down)
    }

    fn pad_buttons(&self) -> u16 {
        let Some(get_state) = self.xinput else {
            return 0;
        };
        let mut state = XInputState::default();
        // SAFETY: `XInputGetState` from the loaded `XINPUT1_3.dll`, with pad 0 and a live
        // `XINPUT_STATE`-shaped out-parameter this frame owns.
        let status = unsafe { get_state(0, &mut state) };
        if status == 0 { state.buttons } else { 0 }
    }
}

/// `XInputGetState` out of the `XINPUT1_3.dll` the game imports, loading it only if it is somehow
/// not already mapped. `None` means no pad for the panel; the keyboard and mouse still work.
fn xinput_get_state() -> Option<XInputGetStateFn> {
    // SAFETY: plain module and export lookups with static NUL-terminated names.
    unsafe {
        let module: HMODULE = GetModuleHandleA(s!("XINPUT1_3.dll"))
            .or_else(|_| LoadLibraryA(s!("XINPUT1_3.dll")))
            .ok()?;
        let address = GetProcAddress(module, s!("XInputGetState"))?;
        Some(core::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            XInputGetStateFn,
        >(address))
    }
}

/// Is `vk` down right now?
fn key_down(vk: i32) -> bool {
    // SAFETY: `GetAsyncKeyState` takes a virtual-key code and returns a bitfield.
    (unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000) != 0
}

/// Is the game the window the keyboard is talking to?
fn game_has_focus() -> bool {
    // SAFETY: two `user32` calls; the only pointer is to an owned local.
    unsafe {
        let window = GetForegroundWindow();
        if window.is_invalid() {
            return false;
        }
        let mut process = 0u32;
        GetWindowThreadProcessId(window, Some(&mut process));
        process == std::process::id()
    }
}
