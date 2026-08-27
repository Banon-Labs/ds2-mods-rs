//! Player-nameable hotkeys that take effect without restarting the game.
//!
//! # The incident this crate exists for
//!
//! It happened in the sibling Elden Ring workspace, not here, and that is the point of porting
//! this crate before any DS2 mod exists. A DLL there polled a hard-coded `VK_F7` every frame.
//! Another DLL loaded alongside it had picked the same default, so a keypress meant for one of
//! them warped the player mid-session, and nothing in either config file could separate them --
//! neither side had a config key to move. The same shape was everywhere: a `VK_*` constant, no
//! config key, no way to change it, and -- where a config key did exist -- no way to change it
//! without quitting the game.
//!
//! Every mod in this repo that binds a key answers both questions the same way, from the first
//! one, because the answer arrived before the mods did.
//!
//! # What a DLL gets from here
//!
//! * [`keys`] -- one table of key NAMES (`"F7"`, `"]"`, `"KP_Plus"`, `"Insert"`) carrying both
//!   numbering schemes a key reaches this process by: Win32 virtual keys and DirectInput
//!   scancodes.
//! * [`reload`] -- [`reload::HotFile`], which notices a config file changed by comparing its TEXT
//!   (not its mtime, which has one-second resolution on the filesystems a Wine or Proton prefix
//!   tends to sit on) and throttles itself to roughly one read per second.
//! * [`kv`] -- [`kv::KeyValues`], `key = value` under `[section]` headers, in a strict subset of
//!   TOML and without a TOML dependency. It reports every line it could not use rather than
//!   skipping it, for the same reason [`binding`] reports a value it could not parse.
//! * [`binding`] -- [`binding::Binding`], which turns "the file changed" into one of exactly three
//!   outcomes: the key moved (reset your edge detector), the key did not move (do NOT reset it), or
//!   the value was junk and the last working key is still in force.
//! * [`live`] -- [`live::AtomicChord`], a binding the detour that actually reads the keyboard can
//!   load without touching a lock the reload path also wants.
//!
//! # What it deliberately does NOT do
//!
//! It does not read the keyboard, own a config file's schema, or know what a key means, and it
//! knows nothing whatsoever about DARK SOULS II -- no address, no offset, no structure. Each DLL
//! keeps its own file, its own key names, and its own hook; this crate is the vocabulary and the
//! reload decision, which are the two parts that were being reinvented differently each time.
//!
//! [`kv`] is the syntax of a config file and not its schema, and the distinction is the whole
//! reason it is allowed in here: it can tell you that `enabled` was written down and what text
//! followed the `=`, and it has no idea that `enabled` is a boolean, which section it belongs
//! under, or what its absence should mean. Those stay with the DLL that owns the file.

pub mod binding;
pub mod keys;
pub mod kv;
pub mod live;
pub mod reload;

pub use binding::{Binding, BindingUpdate};
pub use keys::{
    Chord, KeyParseError, Scancode, VirtualKey, chord_down, chord_name, parse_chord,
    parse_scancode, parse_scancode_chord, parse_virtual_key, scancode_name, vk_name,
};
pub use kv::{KeyValues, RejectReason, Rejected};
pub use live::AtomicChord;
pub use reload::{FileChange, HotFile};
