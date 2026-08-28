//! Point DARK SOULS II's save directory somewhere else, so another player's save can be loaded
//! without touching your own.
//!
//! # What it moves, and what it deliberately does not
//!
//! One detour, on [`ds2_rva::SAVE_DIR_BUILD`] -- the `SaveLoadSystem` helper that produces the
//! folder a `.sl2` lives in:
//!
//! ```text
//! FUN_140248db0(std::wstring *out, const wchar_t *steamid)
//!     out  = "%APPDATA%\\DarkSoulsII\\"
//!     out += steamid
//!     out += "\\"
//! ```
//!
//! Its two callers are both `SaveLoadSystem` methods, so this reaches the saves and nothing else.
//! The wider chokepoint one level down ([`ds2_rva::SAVE_APPDATA_ROOT_BUILD`]) is shared with
//! `GraphicsConfig_SOFS.xml`, and moving the graphics config is not what anyone asked for.
//!
//! # The Steam ID is part of what this replaces, and that is the useful part
//!
//! The second argument is the running account's Steam ID as text, fetched through the same vtable
//! slot the cached ID comes from -- which is why the on-disk layout is
//! `…\DarkSoulsII\<steamid hex>\DS2SOFS0000.sl2`. Because the detour replaces the whole result
//! rather than only the root, a redirect can point straight at a donor save's own folder
//! (`…\DarkSoulsII\01100001526d6d84\`) instead of requiring it be renamed to the running
//! account's. Hooking the root instead would have forced the rename, since the game would go on
//! appending its own ID underneath.
//!
//! # Why it calls the game's `assign` instead of writing the string
//!
//! `out` is a live MSVC `std::basic_string<wchar_t>` owned by the caller, and it may already hold
//! an allocation from the game's allocator. Writing its three fields directly would either leak
//! that allocation or invite a free from the wrong heap. [`ds2_rva::WSTRING_ASSIGN`] is the same
//! function the original uses to seat its own result, so handing the problem back to the code that
//! owns it costs one indirect call and removes an entire class of bug.
//!
//! # Under Proton
//!
//! The game builds a Windows path, so the configured value is a Windows path. Wine maps `Z:` to
//! `/`, so `/home/you/DS2` is `Z:\home\you\DS2`. A value that does not end in a separator gets one
//! appended: the caller appends the file name to whatever this leaves behind, and the trailing
//! backslash is this function's job rather than the caller's.
//!
//! # It logs the path it produced, every time
//!
//! Both arms -- redirected and pass-through -- read the resulting string back out and log it. A
//! redirect that silently produced the wrong folder is indistinguishable from a game with no save,
//! because DS2 shows no LOAD GAME row when it finds nothing. The log line is what tells those two
//! apart without anyone having to guess.

// Windows-only by construction: this is a MinHook detour on a PE image.
#![cfg(windows)]

pub mod install;
pub mod stage;

/// Prefix on every line this crate writes, so its lines can be grepped out of the shared log.
pub const LOG_PREFIX: &str = "ds2-save-redirect:";

pub use install::{Outcome, install, set_logger, set_source};
