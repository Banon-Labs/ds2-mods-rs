//! `ds2-game-base`: the shared low-level foundation every mod DLL in this workspace sits on.
//!
//! Ported from `../er-mods-rs`'s `er-game-base`, minus everything that knew what game it was
//! looking at.
//!
//! # This crate contains no game knowledge, and cannot acquire any
//!
//! No address, no offset, no structure layout and no field ordering appears anywhere below.
//! What is here is Win32 and PE-format mechanics: a kernel-validated read that fails closed, a
//! section-header walk, a WinHTTP request, a file logger, a hash.
//!
//! That is not a convention, it is the dependency graph: this crate does **not** depend on
//! [`ds2-rva`](https://github.com/Banon-Labs/ds2-mods-rs/blob/main/crates/ds2-rva/src/lib.rs),
//! the one crate permitted to hold a DS2 address, so nothing that claims to know something about
//! DARK SOULS II can reach in here. The reason the boundary is drawn this hard: DS2 shipped in
//! 2014 and Elden Ring in 2022, and between them FromSoftware replaced the menu stack, the
//! rendering backend and the object framework. An Elden Ring assumption that survived the port
//! by hiding in a "generic" helper would be a wrong claim about this game with no single file to
//! audit it in.
//!
//! Four modules of `er-game-base` did not survive that test and were deliberately left behind:
//! `rva.rs` (an Elden Ring 1.16.2 singleton address table -- superseded by `ds2-rva`, which is
//! empty because no reverse engineering has been done yet), `filecap.rs` (`FD4FileCap`,
//! `DLString<wchar_t>` and the DLIO virtual-root table -- the FD4 framework postdates this
//! engine), and `pgd.rs` / `profile_summary.rs` / `build_id.rs` (Elden Ring save, profile and
//! release-roster layouts).
//!
//! # Tiers
//!
//! **Tier A (default, zero external deps):** [`mem`], [`http`], [`log`], [`fnv1a`]. Raw
//! `#[link(name = "...")]` externs throughout, so a mini-DLL can depend on this crate without
//! dragging a dependency tree behind it.
//!
//! **Tier B (`game-types` feature, `cfg(windows)`-gated):** [`game_types`], the re-export facade
//! over typed game bindings. Declared and EMPTY -- see its own docs.

pub mod fnv1a;
/// Tier A: one blocking HTTPS request over WinHTTP -- a GET and a JSON POST.
///
/// `cfg(windows)` because it hand-declares the `winhttp` ABI. Lives in the substrate rather than
/// in whichever crate first needs a network call: a hand-declared Win32 ABI is exactly the kind
/// of thing that must have one declaration, and the second caller always arrives.
#[cfg(windows)]
pub mod http;
pub mod log;
pub mod mem;

/// Tier B: the typed-binding re-export facade. **Empty, and honestly so.**
///
/// In `../er-mods-rs` this module is `pub use eldenring; pub use fromsoftware_shared;` -- one
/// import surface the heavy consumers share, so a binding upgrade is one edit. There is no
/// equivalent to re-export here: `../fromsoftware-rs` has `shared`, `shared/stl`, `darksouls3`,
/// `eldenring`, `nightreign` and `sekiro` members and **no `darksouls2`**. Nothing typed exists
/// for this game, and a hand-written stand-in would be exactly the invented game knowledge this
/// crate is built to exclude.
///
/// The seam is declared anyway, so that when a `darksouls2` crate exists the shape it plugs into
/// is already agreed rather than argued about. DS3 is the nearer relative to model it on; even
/// it is a generation away.
#[cfg(all(windows, feature = "game-types"))]
pub mod game_types {}
