//! The one `IDXGISwapChain::Present` detour, and the tables of everything that runs from it.
//!
//! | table | registered with | runs | used by |
//! |---|---|---|---|
//! | clock | [`frame_hook::add_frame_hook`] | first, every frame | input harness, net-effects key, save picker |
//! | drawers | [`frame_hook::add_drawer`] | second, with the swap chain | `ds2-invasion-path`'s route lines |
//! | panels | `panels::add_panel` | last, inside one imgui frame | net-effects selector, save picker |
//!
//! Then the real `Present`. The order is the point: a consumer's clock never stops because a
//! drawer failed, and the imgui panels are always on top of the lines.
//!
//! # Why one crate owns it
//!
//! A second MinHook detour on the same `Present` from the same DLL fails with
//! `MH_ERROR_ALREADY_CREATED`, and hudhook holds exactly one render loop per process. Both used to
//! belong to whichever feature took them first -- `ds2-invasion-path` for the detour,
//! `ds2-net-effects` for the render loop -- so the save picker could neither draw with the arrows
//! switched off nor draw beside the selector. [`install`] is idempotent and every feature that
//! draws calls it.

pub mod frame_hook;
pub mod game_hud;
mod log;
#[cfg(windows)]
pub mod panels;
#[cfg(windows)]
mod present;

pub use game_hud::game_hud_visible;
pub use log::{LOG_PREFIX, LogFn, set_logger};
#[cfg(windows)]
pub use present::install;

/// Host builds have no `Present` to hook. `false`: nothing will tick or draw.
#[cfg(not(windows))]
#[must_use]
pub fn install() -> bool {
    false
}
