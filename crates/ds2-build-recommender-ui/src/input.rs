//! What the player pressed this frame, for the recommender panel: keyboard and pad, as edges.
//!
//! The same reader `ds2-save-file`'s picker uses (`picker_input.rs` there), trimmed to what this
//! panel types: letters, digits, space, `.` and `-`, plus navigation. It reads around the game's own
//! device polls -- the keyboard through `GetAsyncKeyState`, the pad through `XInputGetState` on the
//! `XINPUT1_3.dll` the game already has loaded -- because while the panel is up the input harness's
//! `hold` blanks every device the game reads, and that blanking must not blind the panel as well.
//!
//! Pad 0 through `XInputGetState` is where the player's pad is. Measured 2026-09-28 with
//! `scripts/frida/pad-arm-census.js` on a session with a controller in: one `PadDevice`, XInput
//! port 0, third backend -1 (the XInput arm), and `XInputGetState` answering `ERROR_SUCCESS` for
//! user 0 and `ERROR_DEVICE_NOT_CONNECTED` for users 1 to 3.
//!
//! A copy rather than a shared module because the picker's is private to its crate and carries
//! clipboard shortcuts this panel has no use for; the key table is fixed rather than layout-aware
//! for the reason given there.

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
    Up,
    Down,
    /// D-pad left or the left arrow: the previous control, or a number down one.
    Left,
    /// D-pad right or the right arrow: the next control, or a number up one.
    Right,
    PageUp,
    PageDown,
    /// Enter or A: finish typing.
    Confirm,
    /// B: leave a field or a list, then the panel.
    Back,
    /// Tab: the next field.
    Tab,
    Backspace,
    /// Escape or Start: leave a field or a list, then the panel.
    Close,
    /// X: Run, from anywhere a press moves the cursor.
    Run,
    Char(char),
}

/// Frames a held key waits before it starts repeating, at the game's 60.
const REPEAT_DELAY_FRAMES: u32 = 24;

/// Frames between repeats once it does.
const REPEAT_EVERY_FRAMES: u32 = 4;

const VK_BACK: i32 = 0x08;
const VK_TAB: i32 = 0x09;
const VK_RETURN: i32 = 0x0d;
const VK_SHIFT: i32 = 0x10;
const VK_CONTROL: i32 = 0x11;
const VK_ESCAPE: i32 = 0x1b;
const VK_SPACE: i32 = 0x20;
const VK_PRIOR: i32 = 0x21;
const VK_NEXT: i32 = 0x22;
const VK_LEFT: i32 = 0x25;
const VK_UP: i32 = 0x26;
const VK_RIGHT: i32 = 0x27;
const VK_DOWN: i32 = 0x28;
const VK_OEM_MINUS: i32 = 0xbd;
const VK_OEM_PERIOD: i32 = 0xbe;
const VK_DECIMAL: i32 = 0x6e;
/// `VK_NUMPAD0`; the nine after it are the other keypad digits.
const VK_NUMPAD0: i32 = 0x60;

/// The navigation keys and what each one means.
const NAV_KEYS: [(i32, Press); 11] = [
    (VK_UP, Press::Up),
    (VK_DOWN, Press::Down),
    (VK_LEFT, Press::Left),
    (VK_RIGHT, Press::Right),
    (VK_RETURN, Press::Confirm),
    (VK_ESCAPE, Press::Close),
    (VK_TAB, Press::Tab),
    (VK_PRIOR, Press::PageUp),
    (VK_NEXT, Press::PageDown),
    (VK_BACK, Press::Backspace),
    (VK_SPACE, Press::Char(' ')),
];

/// Punctuation a weapon name or a number needs.
const SYMBOL_KEYS: [(i32, char); 3] =
    [(VK_OEM_MINUS, '-'), (VK_OEM_PERIOD, '.'), (VK_DECIMAL, '.')];

/// XInput's button bits, from `XINPUT_GAMEPAD`.
const PAD_UP: u16 = 0x0001;
const PAD_DOWN: u16 = 0x0002;
const PAD_LEFT: u16 = 0x0004;
const PAD_RIGHT: u16 = 0x0008;
const PAD_LB: u16 = 0x0100;
const PAD_RB: u16 = 0x0200;
const PAD_A: u16 = 0x1000;
const PAD_B: u16 = 0x2000;
const PAD_X: u16 = 0x4000;
const PAD_START: u16 = 0x0010;

/// The pad buttons and what each one means.
const PAD_BUTTONS: [(u16, Press); 10] = [
    (PAD_UP, Press::Up),
    (PAD_DOWN, Press::Down),
    (PAD_LEFT, Press::Left),
    (PAD_RIGHT, Press::Right),
    (PAD_A, Press::Confirm),
    (PAD_B, Press::Back),
    (PAD_X, Press::Run),
    (PAD_LB, Press::PageUp),
    (PAD_RB, Press::PageDown),
    (PAD_START, Press::Close),
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

/// Frames each key or button has been held, a slot per key. `0` is up.
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
            held: vec![0; NAV_KEYS.len() + 26 + 10 + 10 + SYMBOL_KEYS.len() + PAD_BUTTONS.len()],
            xinput: xinput_get_state(),
            pad_last: false,
        }
    }

    /// Whether the player last pressed something on the pad rather than the keyboard.
    pub(crate) fn pad_last(&self) -> bool {
        self.pad_last
    }

    /// Everything pressed since the last call, in a fixed order. Nothing while another window has
    /// the keyboard, so a player typing elsewhere does not drive the panel.
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
            let repeats = !matches!(
                press,
                Press::Confirm | Press::Back | Press::Tab | Press::Close
            );
            step(key_down(vk), press, repeats, &mut self.held);
        }
        for (offset, letter) in ('a'..='z').enumerate() {
            // With Ctrl held a letter is a shortcut somewhere else, and types nothing here.
            let down = key_down(0x41 + offset as i32) && !ctrl;
            let typed = if shift {
                letter.to_ascii_uppercase()
            } else {
                letter
            };
            step(down, Press::Char(typed), true, &mut self.held);
        }
        for (offset, digit) in ('0'..='9').enumerate() {
            let down = key_down(0x30 + offset as i32) && !shift && !ctrl;
            step(down, Press::Char(digit), true, &mut self.held);
        }
        for (offset, digit) in ('0'..='9').enumerate() {
            let down = key_down(VK_NUMPAD0 + offset as i32) && !ctrl;
            step(down, Press::Char(digit), true, &mut self.held);
        }
        for (vk, typed) in SYMBOL_KEYS {
            step(
                key_down(vk) && !shift,
                Press::Char(typed),
                true,
                &mut self.held,
            );
        }
        for (bit, press) in PAD_BUTTONS {
            let repeats = !matches!(
                press,
                Press::Confirm | Press::Back | Press::Close | Press::Run
            );
            step(buttons & bit != 0, press, repeats, &mut self.held);
        }
        // A press this frame with a pad button down came from the pad; any other, the keyboard.
        if !pressed.is_empty() {
            self.pad_last = PAD_BUTTONS.iter().any(|(bit, _)| buttons & bit != 0);
        }
        pressed
    }

    /// Mark everything currently down as already held, so none of it fires on the next poll: the
    /// Enter or A that pressed the row must not also press something in the panel.
    pub(crate) fn swallow_held(&mut self) {
        let _ = self.poll();
    }

    /// Whether any key or button this reader watches is down. The panel keeps the hold for as long
    /// as this is true after it closes, so the press that closed it never reaches the pause menu.
    pub(crate) fn anything_down(&mut self) -> bool {
        self.pad_buttons() != 0
            || NAV_KEYS.iter().any(|(vk, _)| key_down(*vk))
            || (0x30..=0x5a).any(key_down)
            || (VK_NUMPAD0..=VK_DECIMAL).any(key_down)
    }

    /// The pad's button word: pad 0's through `XInputGetState`, plus whatever the input harness is
    /// holding. The harness writes its mask into the game's own `PadDevice`, which this reader
    /// never looks at, and while a panel holds the input it stops writing it there at all -- so
    /// without this an agent driving the harness could not press anything in the panel.
    fn pad_buttons(&self) -> u16 {
        let authored = ds2_input_harness::authored::Authored::current()
            .buttons
            .unwrap_or(0);
        let Some(get_state) = self.xinput else {
            return authored;
        };
        let mut state = XInputState::default();
        // SAFETY: `XInputGetState` from the loaded `XINPUT1_3.dll`, with pad 0 and a live
        // `XINPUT_STATE`-shaped out-parameter this frame owns.
        let status = unsafe { get_state(0, &mut state) };
        authored | if status == 0 { state.buttons } else { 0 }
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
