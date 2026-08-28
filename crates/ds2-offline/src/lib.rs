//! Play DARK SOULS II offline: keep the network service in the state it is constructed in, and
//! refuse the socket calls that would leave this machine anyway.
//!
//! # Why anyone wants this
//!
//! Every other crate in this repo patches `.text` in a running copy of DARK SOULS II. That is
//! exactly what FromSoftware's matchmaking servers are watching for, and a modded client that
//! logs in is a client that can be soft-banned. "Run it offline" is the prerequisite for all the
//! rest of this workspace, not a feature beside it. It is also the only setting under which an
//! invasion cannot interrupt a measurement run.
//!
//! # Two layers, because one of them provably is not enough
//!
//! The obvious design is one patch: find the flag that means "we are online", force it to zero,
//! done. That design was built, and then the disassembly said it does not do what it claims.
//!
//! * **The flag layer** ([`flag`]) is real and it is most of the answer. `NetService::isOnline`
//!   (`0x140513600`) is five bytes -- `movzx eax, BYTE PTR [rcx+0x3a]; ret` -- with **34 call
//!   sites**, every one followed by `test al,al`. `FeSubStateTitleOnlineCheck`'s own work starter
//!   is one of them and returns without starting anything when it reads zero. The top menu greys
//!   out its online rows from another. This layer settles what the game *believes*.
//!
//! * **The socket layer** ([`winsock`]) exists because `FeSubStateTitleGameServerLogin`'s work
//!   starter -- `0x1400f9820`, vtable slot 8 -- **does not read that flag**. It asks
//!   `NetSvrManager` two questions of its own and then builds a login job. Read its disassembly
//!   and the flag layer's story falls apart at exactly the step that matters: the one that talks
//!   to FromSoftware. So the flag layer alone would have shipped a mod that told the player they
//!   were offline while the login went out on the wire.
//!
//! The second layer is therefore not belt-and-braces. It is the layer that makes the crate's
//! name true, and the first layer is what keeps the game's own UI honest about it.
//!
//! # The flag layer neuters the setter rather than forging the getter
//!
//! `NetService`'s constructor (`0x140512f30`) writes `mov BYTE PTR [rbx+0x3a], 0` four
//! instructions in. **Offline is the state this object is born in.** Its setter,
//! `0x140513820` = `mov BYTE PTR [rcx+0x3a], dl; ret`, is the only way out of it, and
//! `FeSubStateTitleSetOfflineMode::v1` is nothing but a tail-jump into the `NetSvrManager` slot
//! that calls that setter with zero.
//!
//! So the primary patch replaces the **setter** with `ret`. That is a materially different claim
//! from forging the getter: it does not impose a value on the game, it prevents a departure from
//! the game's own initial one, and every reader of the flag -- including any that never goes
//! through the getter -- sees a value the game itself wrote.
//!
//! The getter patch is still applied, as a second and independent switch, because "the setter is
//! the only writer" is an inference from one search of one image and the getter patch does not
//! depend on it being right.
//!
//! # What this crate does NOT do
//!
//! * **It does not touch Steam.** `steamclient64.dll` and `GameOverlayRenderer64.dll` are loaded
//!   into this process and own their own sockets; [`winsock`] patches the import table of
//!   `DarkSoulsII.exe` and nothing else, so Steam's own connection, the overlay, achievements and
//!   the friends list are untouched. That is deliberate -- the target is FromSoftware's game
//!   servers, not the platform -- but it means this crate is not a firewall and must not be
//!   described as one.
//! * **It does not suppress the network boot substates.** `0x20` SteamNetworkCheck, `0x39`
//!   GameServerLogin and `0x44` Information still run; they now fail early instead of waiting on
//!   a server. Their failure is a path the shipped game already has -- it is what produces
//!   `FeSubStateTitleOnlineCheckFailWarn` and the "could not retrieve information" box, both of
//!   which `ds2-dialog-skip` already answers. Removing the substates outright is
//!   `ds2-mods-rs-rk4`'s business, not this crate's.
//! * **It does not block loopback.** `127.0.0.0/8` and `::1` are allowed through, because Proton,
//!   Wine and the Steam API all use local sockets and breaking those breaks the game rather than
//!   its matchmaking.
//!
//! # `0x14160de19` is not the switch, and this is where that was settled
//!
//! `docs/DS2-BOOT-WORK.md` records a byte the game reads to force the online flag to zero, and
//! asks whether setting it removes the network boot chain. It does not: it is read at exactly one
//! instruction in the whole image, inside the top-menu builder, and the boot chain calls
//! `0x140513600` directly. See [`ds2_rva::NET_FORCE_OFFLINE_MENU_ONLY`].

#![cfg_attr(not(windows), allow(unused))]

#[cfg(windows)]
mod flag;
#[cfg(windows)]
mod install;
#[cfg(windows)]
mod winsock;

#[cfg(windows)]
pub use install::{LogFn, Outcome, Request, install, set_logger};
#[cfg(windows)]
pub use winsock::{BlockedCounts, counts};

/// Prefix on every line this crate writes to the loader log. Distinct from `ds2-loader:` and from
/// every other feature crate's, so a boot that goes wrong can be attributed to one feature.
pub const LOG_PREFIX: &str = "ds2-offline:";
