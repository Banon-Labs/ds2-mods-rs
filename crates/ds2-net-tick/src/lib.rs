//! One detour on the net session update, shared by every feature that needs a game-thread tick
//! there.
//!
//! [`ds2_rva::NET_SESSION_UPDATE`] runs 60 times a second on the game thread, in the world and at
//! the title (measured with `scripts/frida/tick-count.js`). `ds2-voice-chat` reads its key there,
//! before the original, so a toggled option byte is picked up in the same frame. `ds2-weapon-sync`
//! reads the roster there, after the original, and pushes weapons through the game's weapon
//! update. MinHook binds one detour per address, and the union in `ds2-hook` cannot carry this
//! target: the delta is a float in `xmm1`, which its dispatcher is free to clobber. When each crate
//! created its own hook, whichever installed second failed, and the two could not run together.
//!
//! So this crate owns the only detour, and features register plain callbacks with it:
//!
//! ```text
//! detour(this, delta):
//!     every Before callback, in registration order
//!     the original update
//!     every After callback, in registration order
//! ```
//!
//! The hook is created by the first registration. A callback is `fn(usize)` taking the session
//! manager the game passed, and a panic inside one is caught at the detour, so one feature's bug
//! can neither unwind into the engine nor stop the other features' callbacks.

/// What every line this crate writes begins with.
pub const LOG_PREFIX: &str = "ds2-net-tick:";

pub mod registry;

pub use registry::{Registry, TickFn, When};

#[cfg(windows)]
mod install;

#[cfg(windows)]
pub use install::register;
