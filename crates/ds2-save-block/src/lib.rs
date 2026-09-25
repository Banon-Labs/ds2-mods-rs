//! Stop DARK SOULS II from saving unless a menu row asked it to.
//!
//! The player who turns this on saves through `ds2-save-file`'s `Save Game to File` row and nowhere
//! else. So the five-minute autosave, the save on the way out to the title menu, and every request a
//! bonfire, an item pickup, a level-up or a new game cycle makes are all refused, and the container
//! on disk changes only when a row is pressed.
//!
//! | asks for a save | how it asks | what happens here |
//! |---|---|---|
//! | bonfire, item, souls, level-up, quit to menu, new cycle (23 call sites) | `SaveLoadSystem::RequestSave` sets `+0x1a2` | erased on the next frame, before the update reads it |
//! | the periodic autosave | the update's own timer at `+0x64` crosses 300.0 | the accumulator is held at zero, so it never crosses |
//! | kind 14 | `RequestSave` sets `+0x1a9` alone, deferring by a frame | erased with the rest |
//! | `Save Game to File`, `Load Character from File` | [`permit_save`], then `RequestSave` | passes through untouched |
//!
//! # One detour, on the consumer
//!
//! [`ds2_rva::SAVE_LOAD_SYSTEM_UPDATE`] is the only function in the image that begins a save: it
//! reads the request flags and calls the performer at `0x1402e78c0`, which has exactly one call site.
//! Hooking the twenty-three requesters would be twenty-three detours that still missed the autosave
//! timer, because that timer lives inside the consumer and sets the flag itself.
//!
//! So the detour clears the request, runs the original, and the original finds nothing to do. To the
//! game that state is indistinguishable from a frame in which nobody asked: the fields cleared are
//! the same four the engine clears itself once a save has started, and no caller waits on a
//! completion counter -- the requester that re-asserts every frame tests the flag it set and stops
//! asking when it is clear.
//!
//! # Why not skip the save instead
//!
//! Letting the request stand and skipping the performer looks equivalent and is not. The update sets
//! "a save is in flight" whether or not the call is made, and the next frame's completion poll then
//! answers `4` for an idle system -- a failure status, which the engine has a message for:
//! *"Failed to save game.\nReturning to Title Menu."* Erasing the request avoids that path entirely
//! rather than hoping nothing reads the status.
//!
//! # What a permit is, and why it is a countdown
//!
//! `RequestSave` does not save; it sets a flag the update acts on some frames later, when the game is
//! ready (`+0x1a4`, `+0x1a5`, the cooldown at `+0x60`). A row that wants its save therefore has to
//! keep the door open for longer than one frame, so [`permit_save`] arms a countdown of
//! [`policy::PERMIT_FRAMES`] frames and the door shuts the moment the save it allowed has begun. See
//! [`policy::decide`], which is where both halves of that are tested.
//!
//! # What this does not touch
//!
//! * **The title screen's own writes.** Creating, deleting or copying a character writes through
//!   three functions of its own (`0x1402e7450`, `0x1402e7d20`, `0x1402e7f10`) that never go near the
//!   in-game update. Those are a player pressing a button on a list, not the game saving by itself.
//! * **Loads.** Nothing here touches the load side, the container the game reads, or
//!   `ds2-save-redirect`'s redirection of either.
//! * **Progress the game would otherwise have persisted.** That is the point of the feature and it
//!   is also its cost: with this on, anything gained since the last row press is gone if the game
//!   closes. `ds2-build-import` applies a build to the live character and relies on a later save to
//!   persist it -- under this feature that save is the row press.

/// What every line this crate writes begins with, so its lines can be grepped out of the shared log.
pub const LOG_PREFIX: &str = "ds2-save-block:";

pub mod policy;

pub use policy::{dropped_requests, permit_remaining, permit_save};

#[cfg(windows)]
mod install;

#[cfg(windows)]
pub use install::{LogFn, Outcome, install, set_logger};
