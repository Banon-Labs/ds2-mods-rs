//! Record which save slot a load actually used, as the first half of a native continue flow.
//!
//! The end goal is `[continue] slot = N`: press CONTINUE and land in that character instead of in
//! the character list. This crate is the half that has to come first, and it **only records**. It
//! writes nothing into the game and changes no transition.
//!
//! # Why recording comes before driving
//!
//! The static trace in `docs/DS2-CONTINUE.md` establishes the whole path --
//! `TopMenu 0x47 -> LoadDataList 0x55 -> LoadProfile 0x57 -> 0x6a -> StartIngame 0x6b` -- and
//! establishes that the slot is nothing more than an `i32` at
//! [`ds2_rva::FE_TITLE_CONTEXT_SLOT_NUM_OFFSET`]. From that alone the continue flow looks like two
//! writes: set the slot, retarget the phase-2 transition from `0x55` to `0x57`.
//!
//! Two things say "looks like" rather than "is", and neither can be settled by reading more
//! disassembly:
//!
//! 1. **Skipping `0x55` skips its `enter`.** `FeSubStateTitleLoadDataList::v1` (`0x1400fae80`)
//!    runs before any of this, and `LoadProfile` may depend on what it set up -- the update reads
//!    the group at `[FE_TITLE_CONTEXT]+0x98` on every frame, and something has to have put it
//!    there. A continue flow that jumps the list and finds that pointer stale boots into nothing.
//! 2. **The ownership gate must survive.** On the load action the update calls `0x140af6610` with
//!    `record[0x1e8] & 0x3f` and takes a *different* destination when it passes. Skipping the list
//!    must not also skip that, or a clean refusal becomes a load that should not have happened.
//!
//! A recorded real load answers both, because the log shows what the game itself did on the frame
//! the player confirmed, and `ds2-boot-timeline` shows the substate chain around it.
//!
//! # One hook, and why not four
//!
//! Only [`ds2_rva::FE_SUBSTATE_LOAD_DATA_LIST_UPDATE`] is detoured. The obvious alternative is to
//! also hook the list's `enter`, its `v5`, and the top menu's update -- but `ds2-boot-timeline`
//! already reports every substate arrival and departure from machinery no substate can avoid, so
//! those three hooks would only re-report what an existing instrument already prints, at the cost
//! of three more patched sites during startup.
//!
//! What that instrument cannot show is the *decision*: which slot, which action, and which phase
//! came out. That is this one function, and this is the one hook.
//!
//! # The pre-select, which is the cheap half of the shortcut
//!
//! `[continue] slot = N` writes that slot into the field the list lays itself out from, in the
//! list's own `enter`, before `0x1400f1cb0` is called. The list still opens and still does all of
//! its own setup -- so this tests the risky half of the question (does `LoadProfile` accept a slot
//! the player never picked by hand) while leaving the recoverable half alone. If it goes wrong you
//! are looking at a character list, not a black screen.
//!
//! It refuses to point the cursor anywhere the game would not: same bound as the update's, and the
//! record must be occupied and not excluded. A configured slot that fails is logged and skipped.
//!
//! # Off by default
//!
//! Like `ds2-boot-timeline`, and for the same reason: it is an instrument. `[continue]
//! record = true`, or `ds2-run.py --continue-record`, turns it on for a measurement run.
//!
//! # Silencing the shortcut
//!
//! A shortcut nobody pressed anything for still plays the title music and the menu's confirm
//! sounds, which is conspicuous. `[continue] silence = true` holds FMOD's master channel group at
//! zero from install until `FeSubStateTitleStartIngame`, then restores the volume the game itself
//! had applied. See [`silence`] for how that lever was found and why the four approaches before it
//! failed.

#![cfg_attr(not(windows), allow(unused))]

#[cfg(windows)]
mod hide_menus;
#[cfg(windows)]
mod install;
#[cfg(windows)]
mod silence;

#[cfg(windows)]
pub use hide_menus::set_enabled as set_hide_menus;
#[cfg(windows)]
pub use install::{LogFn, Outcome, install, set_logger, set_preselect_slot};
#[cfg(windows)]
pub use silence::set_enabled as set_silence;

/// Prefix on every line this crate writes to the loader log.
pub const LOG_PREFIX: &str = "ds2-continue:";
