//! The Invadable hosts panel: how many DARK SOULS II Seamless Co-op hosts this player could invade
//! now, in every invasion bracket Seamless publishes, and how often new ones open.
//!
//! Ported from er-mods-rs `er-invasion-warp`'s band census (commit 26312975), whose requirements it
//! keeps: counted from Steam's lobby list with `RequestLobbyList` and string filters, nothing joined
//! and nothing written; only what this player's own search could return; one query in flight and
//! only while Seamless is not searching; counting from the first open, in the background after.
//!
//! # The parts
//!
//! - [`census`]: the sweep, the 50-lobby split, the arrivals and the grid's cells. Host-tested.
//! - [`keys`]: the two keys out of `[invadable_hosts]`, hot-reloaded by text. Host-tested.
//! - `steam` (Windows): the matchmaking interface. The census and Seamless share one, and Steam
//!   keeps one lobby-list request per interface, so the interface's vtable pointer is pointed at a
//!   copy whose staging and request slots note what Seamless is doing. No code byte is patched.
//! - `driver` (Windows): the one query in flight, the clock, the arrivals file, the panel's view.
//! - `panel` (Windows): the imgui window, its keys and the game's input while it is open.

/// What every line this crate writes begins with.
pub const LOG_PREFIX: &str = "ds2-invadable-hosts:";

/// The file the arrivals persist in, beside `ds2-mods.toml`.
pub const ARRIVALS_FILE_NAME: &str = "ds2-invadable-hosts.txt";

/// Milliseconds between two heartbeat lines.
pub const HEARTBEAT_EVERY_MS: u64 = 10_000;

pub mod census;
pub mod keys;

#[cfg(windows)]
mod driver;
#[cfg(windows)]
mod install;
#[cfg(windows)]
mod panel;
#[cfg(windows)]
mod steam;

#[cfg(windows)]
pub use install::{LogFn, install, set_logger};
