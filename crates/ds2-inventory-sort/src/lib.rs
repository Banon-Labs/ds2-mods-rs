//! Open DARK SOULS II's own inventory sort dialog from a button you choose.
//!
//! # This adds no feature. The game already has one, on a button you cannot move.
//!
//! DARK SOULS II ships inventory sorting: a `①：Sort` prompt, a dialog headed "How should the list
//! be sorted?" (`common.fmg` 80059901), and per-category keys -- Default positions, By effect,
//! Attack, Weight, Damage reduction for weapons; Defense and Weight for armour; Held for
//! consumables. Choosing the key already in force reverses the direction. `Attack` is the sum of
//! all seven attack components, so it is a real total attack rating rather than a base number.
//!
//! What it does not ship is a way to move that button. `win32onlymessage.fmg` 10332..10341 is the
//! COMPLETE list of rebindable menu actions -- cursor up/down/left/right, Confirm, Cancel, Toggle
//! menu left/right, Function 1, Function 2 -- and sorting is not one of them: it rides on one of
//! the two generic Function keys along with Remove, Delete and Reset-to-default. There is no
//! controller remapping at all; a scan of every shipped `.fmg` finds no button-config screen.
//!
//! So this crate is a rebinding and nothing more. It does not draw a row, add a sort key, reorder
//! anything itself, or touch the shipped `①` binding, which keeps working.
//!
//! # The equip screen gets the same button, which the game never gave it
//!
//! Choosing a weapon to equip opens a `FeGroupItemEquip` list, and that screen ships no sort prompt
//! at all -- there is no button to rebind there, only one to add.
//!
//! **The sorting itself is already wired in.** That list is rebuilt by `FeIngameItemSelectMenu::v57`
//! (`0x140097400`), which calls the same shared builder `0x140036080` the Inventory tab uses, and
//! that builder reads the same per-category sort key. So a sort chosen in the Inventory tab ALREADY
//! reorders the equip list; what is missing is only a way to choose one without leaving the screen.
//!
//! One dialog opener serves both. `FeGroupInGameMenuInventory2`, `FeGroupItemEquip` and
//! `FeItemBoxMenu` (the storage box) share a base class: their `+0x50` vtables agree slot for slot
//! through `+0x28`, so `[this+0x58]` and `this+0x50` mean the same thing in each, and the opener
//! reaches everything else through virtual slots each class implements for itself -- `+0x140` for
//! the category, `+0x148` to rebuild. That the machinery is generic rather than Inventory-specific
//! is not inferred: the storage box's own copy of the opener builds its rows from the SAME closure,
//! vtable `0x1410b1ec0` and member function `0x1400ba300`, an Inventory-named functor the shipped
//! game already reuses for a different class's list.
//!
//! # How, and why it is five hooks and no injected input
//!
//! [`ds2_rva::FE_INVENTORY_SORT_DIALOG_OPEN`] takes exactly one argument -- a live group -- and
//! builds and shows the dialog itself. So the only question is where that object is, and the answer
//! is a constructor and a destructor, once per menu:
//!
//! * [`ds2_rva::FE_INVENTORY_GROUP_CTOR`] runs when the player opens the Inventory tab. The detour
//!   records `RCX` and calls the original.
//! * [`ds2_rva::FE_INVENTORY_GROUP_DTOR`] runs when it goes away. The detour clears the record.
//! * [`ds2_rva::FE_EQUIP_GROUP_CTOR`] and [`ds2_rva::FE_EQUIP_GROUP_DTOR`] do the same for the equip
//!   picker. The constructor takes FOUR arguments, not three, and the detour forwards all four.
//! * `ds2-menu-row`'s per-frame tick polls the button on the game thread.
//!
//! Both menus can be live at once, so each record carries a sequence number and a press goes to the
//! one built most recently -- which is the one the player is looking at. Each pair installs all or
//! nothing: a constructor hook whose destructor refused would record a pointer nothing ever clears,
//! so that menu is left untracked instead. Losing one pair does not disarm the other.
//!
//! Nothing here synthesises a keypress into the game's input path. That was the other available
//! design and it is worse in the way `ds2-safe-input`'s own docs describe: an injected button is a
//! press nobody made, it can stick, and it would fire every OTHER thing the shipped Function key
//! does in whatever menu happens to be open.
//!
//! # What it refuses, and who does the refusing
//!
//! The game does. `FE_INVENTORY_SORT_DIALOG_OPEN` tests `[this+0x58]` and returns having done
//! nothing when the group already has a child dialog up, so a press while the sort dialog (or a
//! discard prompt) is open is declined by the shipped code rather than by this crate's guess about
//! whether now is a good moment. A press with neither menu open reaches a null record and is
//! dropped here, which is the only refusal this crate owns.
//!
//! # The binding moves without restarting the game
//!
//! House rule, and the reason `ds2-hotkey-config` exists: a hotkey is a NAME in a config file, and
//! editing that file takes effect in about a second. A watcher thread re-reads
//! `<Game>/ds2-mods.toml` and publishes into an [`ds2_hotkey_config::live::AtomicChord`]; the
//! per-frame tick does one atomic load and never touches a lock or the filesystem. A value that
//! does not parse keeps the key that was already working and says so in the log.

#![cfg_attr(not(windows), allow(unused))]

#[cfg(windows)]
mod install;

#[cfg(windows)]
pub use install::{LogFn, Outcome, Request, install, set_logger};

/// Prefix on every line this crate writes to the loader log.
pub const LOG_PREFIX: &str = "ds2-inventory-sort:";

/// The config section this crate reads its binding out of. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "inventory_sort";

/// The keyboard binding key. A name from [`ds2_hotkey_config::keys`], e.g. `"F7"`, `"]"`, `"KP_5"`.
pub const CONFIG_KEY_KEYBOARD: &str = "key";

/// The controller binding key. An XInput button name, e.g. `"Y"`, `"X"`, `"LThumb"`. See
/// [`install::PAD_BUTTONS`].
pub const CONFIG_KEY_PAD: &str = "pad";
