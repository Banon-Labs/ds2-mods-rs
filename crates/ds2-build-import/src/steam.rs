//! Borrowing the game's Steam text field, and putting it back the way it was found.
//!
//! # Why a second interface and not the game's
//!
//! DARK SOULS II ships a `steam_api64.dll` that knows `SteamUtils005`, and at `0x140ff2415` it
//! loads four arguments into `ShowGamepadTextInput` and never writes the fifth. That fifth is
//! `pchExistingText` -- the prefill -- and it exists from `SteamUtils007` on.
//!
//! **That is a limit of the GAME's call, not of the API.** `ISteamClient012::GetISteamUtils` takes
//! a version STRING and passes it through to `steamclient64.dll`, which vends every version up to
//! `011`. So asking for [`ds2_rva::STEAM_UTILS_VERSION_WITH_PREFILL`] hands back a different object
//! whose slot `0xa0` takes the prefill, while the game keeps its own `005` pointer untouched.
//!
//! Keeping them separate is not tidiness. `007` puts `ShowGamepadTextInput` at the SAME slot as
//! `005`, so handing the game the newer pointer would leave its four-argument call passing whatever
//! happened to be in `r9` as a `const char*`.
//!
//! # The interlock, and what it is defending against
//!
//! The game's `GamepadTextInputDismissed_t` listener ([`ds2_rva::SOFTWARE_KEYBOARD_DISMISSED_HANDLER`])
//! does not check whether the game asked for the keyboard. It is registered process-wide in
//! `steam_api64.dll`'s table, so it fires for a session THIS crate opened. It cannot crash -- it is
//! fourteen branchless bytes that read no text -- but it writes `m_state`, and that has two
//! consequences worth naming:
//!
//! * **Left dirty, the Steam keyboard never comes back.** `show` refuses unless it reads `-1`, and
//!   the only writers of `-1` are inside `getResult`, which the game cannot reach once `show` is
//!   failing. One careless session and character naming silently falls back to the in-game widget
//!   for the rest of the process.
//! * **Opened over the game's own session, the game harvests OUR text.** `FeSoftKeyImputJob` polls
//!   the same field, sees a finished state it did not cause, and commits whatever Steam is holding
//!   as the player's character name.
//!
//! So: refuse unless idle, claim it while open, restore it when done -- including on every error
//! path. Three aligned dword accesses, no hooks, no detour, nothing patched.

use core::ffi::c_void;

use ds2_game_base::mem::{game_rva, safe_read_i32, safe_read_usize};

/// `ISteamUtils`, resolved at whatever version was asked for.
#[derive(Clone, Copy)]
pub(crate) struct SteamUtils(usize);

/// Why the field could not be opened. Every variant is a different thing to tell the player.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SteamError {
    /// `steam_api64.dll` is not loaded, or does not export what this needs.
    NoSteamApi,
    /// `SteamClient()` returned null -- the API is present but not initialised.
    NoClient,
    /// `steamclient64.dll` would not vend the version with the prefill.
    NoUtils,
    /// The Steam overlay is off. **The one failure no amount of reading the executable predicts.**
    OverlayDisabled,
    /// The game is using the keyboard, or left it in a finished state we must not clear.
    KeyboardBusy(i32),
    /// `ShowGamepadTextInput` returned false.
    Refused,
    /// A module base or RVA would not resolve.
    Unresolved,
}

impl core::fmt::Display for SteamError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SteamError::NoSteamApi => write!(f, "steam_api64.dll exports not found"),
            SteamError::NoClient => write!(f, "SteamClient() is null -- Steam not initialised"),
            SteamError::NoUtils => write!(
                f,
                "steamclient would not vend {}",
                ds2_rva::STEAM_UTILS_VERSION_WITH_PREFILL
            ),
            SteamError::OverlayDisabled => {
                write!(f, "the Steam overlay is disabled -- no field can be shown")
            }
            SteamError::KeyboardBusy(state) => {
                write!(f, "the game's keyboard is busy (m_state={state})")
            }
            SteamError::Refused => write!(f, "ShowGamepadTextInput returned false"),
            SteamError::Unresolved => write!(f, "a module base or RVA would not resolve"),
        }
    }
}

// The three exports the game's own shim already provides. Resolved by name at runtime rather than
// linked, because this DLL must load whether or not Steam is there.
type SteamClientFn = unsafe extern "system" fn() -> usize;
type GetHSteamPipeFn = unsafe extern "system" fn() -> i32;

// `ISteamClient012::GetISteamUtils(this, HSteamPipe, const char *pchVersion)`.
type GetISteamUtilsFn = unsafe extern "system" fn(usize, i32, *const u8) -> usize;
// `ISteamUtils::IsOverlayEnabled(this)`.
type IsOverlayEnabledFn = unsafe extern "system" fn(usize) -> bool;
// `ISteamUtils::ShowGamepadTextInput(this, mode, lineMode, desc, charMax, existing)` -- the
// five-argument form. `charMax` lands at `[rsp+0x20]` and `existing` at `[rsp+0x28]`, which is the
// slot the game never writes.
type ShowGamepadTextInputFn =
    unsafe extern "system" fn(usize, u32, u32, *const u8, u32, *const u8) -> bool;
// `ISteamUtils::GetEnteredGamepadTextLength(this)` -- includes the terminator.
type GetEnteredTextLengthFn = unsafe extern "system" fn(usize) -> u32;
// `ISteamUtils::GetEnteredGamepadTextInput(this, *mut u8, u32)` -- UTF-8 out.
type GetEnteredTextFn = unsafe extern "system" fn(usize, *mut u8, u32) -> bool;

unsafe extern "system" {
    fn GetModuleHandleA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

/// Look up one export in the game's own `steam_api64.dll`.
///
/// **The game's copy, deliberately.** Loading a second `steam_api64.dll` would give this crate its
/// own callback table and its own pipe, and the interlock below is meaningless against a different
/// Steam context than the one the game is using.
fn steam_api_export(name: &[u8]) -> Option<*mut c_void> {
    // SAFETY: both strings are NUL-terminated literals, and both calls only read them. A missing
    // module or export comes back null, which is the `None` below rather than a fault.
    unsafe {
        let module = GetModuleHandleA(c"steam_api64.dll".to_bytes_with_nul().as_ptr());
        if module.is_null() {
            return None;
        }
        let proc = GetProcAddress(module, name.as_ptr());
        (!proc.is_null()).then_some(proc)
    }
}

impl SteamUtils {
    /// Ask `steamclient` for the interface version whose text input takes a prefill.
    pub(crate) fn with_prefill_support() -> Result<Self, SteamError> {
        let client_fn =
            steam_api_export(c"SteamClient".to_bytes_with_nul()).ok_or(SteamError::NoSteamApi)?;
        let pipe_fn = steam_api_export(c"SteamAPI_GetHSteamPipe".to_bytes_with_nul())
            .ok_or(SteamError::NoSteamApi)?;

        // SAFETY: both addresses came from `GetProcAddress` on the loaded `steam_api64.dll`, so
        // they are that module's own code, and both exports are niladic `extern "C"` functions
        // whose signatures are fixed by the Steamworks ABI.
        let (client, pipe) = unsafe {
            let client: SteamClientFn = core::mem::transmute(client_fn);
            let pipe: GetHSteamPipeFn = core::mem::transmute(pipe_fn);
            (client(), pipe())
        };
        if client == 0 {
            return Err(SteamError::NoClient);
        }

        let version = c"SteamUtils007";
        debug_assert_eq!(
            version.to_str().ok(),
            Some(ds2_rva::STEAM_UTILS_VERSION_WITH_PREFILL),
            "the literal handed to Steam must be the version ds2-rva documents"
        );

        // SAFETY: `client` is a non-null `ISteamClient012*` from the game's own shim, so its first
        // qword is a vtable and `+0x48` is `GetISteamUtils` -- the slot Proton's own generated
        // wrapper calls. The version string is a NUL-terminated literal in this DLL and Steam only
        // reads it. A version steamclient will not vend comes back null, not as a fault.
        let utils = unsafe {
            let vtable = safe_read_usize(client).ok_or(SteamError::NoClient)?;
            let slot = safe_read_usize(vtable + ds2_rva::STEAM_CLIENT_GET_ISTEAM_UTILS_SLOT)
                .ok_or(SteamError::NoClient)?;
            let get: GetISteamUtilsFn = core::mem::transmute(slot);
            get(client, pipe, version.to_bytes_with_nul().as_ptr())
        };
        if utils == 0 {
            return Err(SteamError::NoUtils);
        }
        Ok(Self(utils))
    }

    /// Whether the overlay can draw a field at all.
    ///
    /// The game asks the same question in the same place, through the same slot, before it will
    /// open its own keyboard.
    pub(crate) fn overlay_enabled(self) -> bool {
        // SAFETY: `self.0` came from `GetISteamUtils` and is a live `ISteamUtils*`; the slot is the
        // one `SoftwareKeyboardManagerImpl::show` itself calls. The method takes only `this`.
        unsafe {
            let Some(vtable) = safe_read_usize(self.0) else {
                return false;
            };
            let Some(slot) = safe_read_usize(vtable + ds2_rva::STEAM_UTILS_IS_OVERLAY_ENABLED_SLOT)
            else {
                return false;
            };
            let enabled: IsOverlayEnabledFn = core::mem::transmute(slot);
            enabled(self.0)
        }
    }

    /// Show the field, prefilled.
    ///
    /// `description` and `prefill` must be NUL-terminated UTF-8 -- Steam takes `char*`, not wide.
    pub(crate) fn show(
        self,
        description: &[u8],
        prefill: &[u8],
        char_max: u32,
    ) -> Result<(), SteamError> {
        // SAFETY: the slot is the one the game calls on its own pointer, observed at `0x140ff2424`,
        // and it is the same slot on `007` -- verified against Proton's generated wrapper, which
        // marshals five arguments to `*0xa0`. Both strings are NUL-terminated and live for the
        // duration of the call; Steam copies what it keeps.
        let shown = unsafe {
            let vtable = safe_read_usize(self.0).ok_or(SteamError::Unresolved)?;
            let slot = safe_read_usize(vtable + ds2_rva::STEAM_UTILS_SHOW_GAMEPAD_TEXT_INPUT_SLOT)
                .ok_or(SteamError::Unresolved)?;
            let show: ShowGamepadTextInputFn = core::mem::transmute(slot);
            show(
                self.0,
                GAMEPAD_TEXT_INPUT_MODE_NORMAL,
                GAMEPAD_TEXT_INPUT_LINE_MODE_SINGLE,
                description.as_ptr(),
                char_max,
                prefill.as_ptr(),
            )
        };
        if shown {
            Ok(())
        } else {
            Err(SteamError::Refused)
        }
    }

    /// The text the player left, or `None` if there is none to read.
    pub(crate) fn entered_text(self) -> Option<String> {
        // SAFETY: both slots are the pair `SoftwareKeyboardManagerImpl::getResult` calls, at
        // `0x140ff20b1` and `0x140ff2101`. The buffer handed over is sized by the length Steam just
        // reported and is written by Steam alone.
        unsafe {
            let vtable = safe_read_usize(self.0)?;
            let length_slot =
                safe_read_usize(vtable + ds2_rva::STEAM_UTILS_GET_ENTERED_TEXT_LENGTH_SLOT)?;
            let text_slot = safe_read_usize(vtable + ds2_rva::STEAM_UTILS_GET_ENTERED_TEXT_SLOT)?;
            let length: GetEnteredTextLengthFn = core::mem::transmute(length_slot);
            let fetch: GetEnteredTextFn = core::mem::transmute(text_slot);

            // The reported length INCLUDES the terminator, so 0 and 1 both mean "nothing typed".
            let reported = length(self.0);
            if reported <= 1 || reported as usize > MAX_ENTERED_BYTES {
                return None;
            }
            let mut buffer = vec![0_u8; reported as usize];
            if !fetch(self.0, buffer.as_mut_ptr(), reported) {
                return None;
            }
            let end = buffer
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(buffer.len());
            buffer.truncate(end);
            String::from_utf8(buffer).ok()
        }
    }
}

/// `k_EGamepadTextInputModeNormal`. The game passes this too (`xor edx,edx` at `0x140ff241d`).
const GAMEPAD_TEXT_INPUT_MODE_NORMAL: u32 = 0;
/// `k_EGamepadTextInputLineModeSingleLine`. Also what the game passes.
const GAMEPAD_TEXT_INPUT_LINE_MODE_SINGLE: u32 = 0;

/// A ceiling on what Steam is allowed to hand back, so a nonsense length cannot become an
/// allocation. A URL is forty bytes; this is generous by three orders of magnitude and still small.
const MAX_ENTERED_BYTES: usize = 64 * 1024;

/// The game's `SoftwareKeyboardManagerImpl`, if it has been built yet.
///
/// **Null is the good case.** The dismissal listener is registered by the impl's constructor, so
/// while this is null nothing in the game is listening and a session opened here disturbs nothing.
fn impl_singleton() -> Option<usize> {
    let address = game_rva(ds2_rva::SOFTWARE_KEYBOARD_IMPL_SINGLETON).ok()?;
    // SAFETY: the address is a resolved RVA inside the loaded game image, and `safe_read_usize`
    // faults safely if the page is not mapped.
    let pointer = unsafe { safe_read_usize(address)? };
    (pointer != 0).then_some(pointer)
}

/// Whether the game has ever built its own `SoftwareKeyboardManagerImpl`.
///
/// Reported beside a refusal because it separates two very different stories. If the game has
/// NEVER built one, the game has never successfully opened a Steam keyboard either -- which on a
/// desktop Steam is the expected state and points at Big Picture rather than at anything this crate
/// did. If it HAS, then the same call works for the game and not for us, and the difference is ours
/// to find.
pub(crate) fn game_keyboard_built() -> bool {
    impl_singleton().is_some()
}

/// A claim on the game's keyboard state, released when it drops.
///
/// The whole point is the `Drop`. Restoring `m_state` on the happy path is easy to write and easy
/// to skip on an error path -- and the error paths are exactly where a session ends early. A guard
/// makes the restore unconditional, including on an unwind.
pub(crate) struct KeyboardClaim {
    state_field: Option<usize>,
}

impl KeyboardClaim {
    /// Refuse unless the game's keyboard is idle, then claim it.
    ///
    /// The refusal is the same test [`ds2_rva::SOFTWARE_KEYBOARD_IMPL_SHOW`] applies to itself:
    /// anything but `-1` means the game either owns a live session or is about to harvest a
    /// finished one, and either way a field opened now would be read as the player's answer to a
    /// question they were not asked.
    pub(crate) fn acquire() -> Result<Self, SteamError> {
        let Some(instance) = impl_singleton() else {
            // No impl, no listener, nothing to interlock against.
            return Ok(Self { state_field: None });
        };
        let field = instance + ds2_rva::SOFTWARE_KEYBOARD_IMPL_STATE_OFFSET;
        // SAFETY: `instance` is the pointer the game itself stores, and the field is the `int32`
        // its constructor initialises. Aligned four-byte accesses, so no torn read is possible.
        let state = unsafe { safe_read_i32(field) }.ok_or(SteamError::Unresolved)?;
        if state != ds2_rva::SOFTWARE_KEYBOARD_STATE_IDLE {
            return Err(SteamError::KeyboardBusy(state));
        }
        // SAFETY: as above. Claiming makes the game's own `show` bail rather than stack a second
        // keyboard on ours -- it degrades to the in-game widget, which is the graceful outcome.
        unsafe {
            core::ptr::write_volatile(field as *mut i32, ds2_rva::SOFTWARE_KEYBOARD_STATE_SHOWING)
        };
        Ok(Self {
            state_field: Some(field),
        })
    }

    /// The state the game's listener has written, if the session has ended.
    pub(crate) fn finished_state(&self) -> Option<i32> {
        let field = self.state_field?;
        // SAFETY: see `acquire`.
        let state = unsafe { safe_read_i32(field) }?;
        (state == ds2_rva::SOFTWARE_KEYBOARD_STATE_SUBMITTED
            || state == ds2_rva::SOFTWARE_KEYBOARD_STATE_CANCELLED)
            .then_some(state)
    }
}

impl Drop for KeyboardClaim {
    /// Put `m_state` back to idle, unconditionally.
    ///
    /// This is the write that keeps the game's own character-naming keyboard working. It is exactly
    /// what `getResult` does at `0x140ff221c`; without it, one session here is enough to make the
    /// Steam keyboard silently never appear again for the life of the process.
    fn drop(&mut self) {
        if let Some(field) = self.state_field {
            // SAFETY: see `acquire` -- an aligned `int32` inside an object the game keeps alive for
            // the process lifetime (its destructor runs only from the atexit teardown).
            unsafe {
                core::ptr::write_volatile(field as *mut i32, ds2_rva::SOFTWARE_KEYBOARD_STATE_IDLE)
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The literal handed to Steam is the version `ds2-rva` documents as the one that can prefill.
    ///
    /// They are two spellings of one fact: the string goes to `steamclient`, the constant is what
    /// the provenance is written against. A silent drift between them would ask for a version whose
    /// `ShowGamepadTextInput` takes four arguments and get no prefill and no error.
    #[test]
    fn the_version_asked_for_is_the_one_documented() {
        assert_eq!(
            c"SteamUtils007".to_str().unwrap(),
            ds2_rva::STEAM_UTILS_VERSION_WITH_PREFILL
        );
    }

    /// The four states are distinct, and "finished" is the pair the guard polls for.
    #[test]
    fn the_keyboard_states_do_not_overlap() {
        let all = [
            ds2_rva::SOFTWARE_KEYBOARD_STATE_IDLE,
            ds2_rva::SOFTWARE_KEYBOARD_STATE_SHOWING,
            ds2_rva::SOFTWARE_KEYBOARD_STATE_CANCELLED,
            ds2_rva::SOFTWARE_KEYBOARD_STATE_SUBMITTED,
        ];
        for (index, one) in all.iter().enumerate() {
            for other in &all[index + 1..] {
                assert_ne!(one, other);
            }
        }
        // The value the guard restores must be the one `show` demands, or the game's own keyboard
        // stays wedged for the life of the process.
        assert_eq!(ds2_rva::SOFTWARE_KEYBOARD_STATE_IDLE, -1);
        // And "claimed" must not read as "finished", or the poll would return on its own write.
        assert_ne!(
            ds2_rva::SOFTWARE_KEYBOARD_STATE_SHOWING,
            ds2_rva::SOFTWARE_KEYBOARD_STATE_CANCELLED
        );
        assert_ne!(
            ds2_rva::SOFTWARE_KEYBOARD_STATE_SHOWING,
            ds2_rva::SOFTWARE_KEYBOARD_STATE_SUBMITTED
        );
    }

    /// Every vtable slot this crate calls is eight-byte aligned, which is what a slot index means.
    ///
    /// A slot that is not a multiple of eight is an offset someone wrote in indices by mistake, and
    /// the symptom would be a call through the middle of a pointer.
    #[test]
    fn every_slot_is_a_pointer_index() {
        for slot in [
            ds2_rva::STEAM_CLIENT_GET_ISTEAM_UTILS_SLOT,
            ds2_rva::STEAM_UTILS_IS_OVERLAY_ENABLED_SLOT,
            ds2_rva::STEAM_UTILS_SHOW_GAMEPAD_TEXT_INPUT_SLOT,
            ds2_rva::STEAM_UTILS_GET_ENTERED_TEXT_LENGTH_SLOT,
            ds2_rva::STEAM_UTILS_GET_ENTERED_TEXT_SLOT,
        ] {
            assert_eq!(slot % core::mem::size_of::<usize>(), 0, "{slot:#x}");
        }
        // The three `ISteamUtils` gamepad slots are consecutive, in the order `getResult` calls
        // them -- show, then length, then text. That ordering is what the disassembly showed.
        assert_eq!(
            ds2_rva::STEAM_UTILS_GET_ENTERED_TEXT_LENGTH_SLOT,
            ds2_rva::STEAM_UTILS_SHOW_GAMEPAD_TEXT_INPUT_SLOT + 8
        );
        assert_eq!(
            ds2_rva::STEAM_UTILS_GET_ENTERED_TEXT_SLOT,
            ds2_rva::STEAM_UTILS_GET_ENTERED_TEXT_LENGTH_SLOT + 8
        );
    }

    /// A refusal says which state refused it, so the log names the culprit rather than the symptom.
    #[test]
    fn a_busy_keyboard_reports_the_state_that_refused() {
        let busy = SteamError::KeyboardBusy(ds2_rva::SOFTWARE_KEYBOARD_STATE_SUBMITTED);
        assert!(busy.to_string().contains("m_state=3"));
        assert_ne!(busy, SteamError::KeyboardBusy(0));
    }
}
