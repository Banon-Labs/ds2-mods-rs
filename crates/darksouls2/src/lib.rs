//! Typed layouts for DARK SOULS II's own objects.
//!
//! Each type is a hand-written `#[repr(C)]` view over memory the game owns: the fields this repo
//! has read out of the binary, and padding named by offset in between. Addresses stay in
//! `ds2-rva`, which this crate depends on and does not re-export. Every field offset is pinned by
//! an `offset_of!` test, so a layout that drifts from what was measured fails the build.
//!
//! `docs/DS2-BINDINGS-CRATE.md` has the scope and the staged plan; `docs/DS2-BINDINGS-PLAN.md`
//! has the module layout.

pub mod game;
