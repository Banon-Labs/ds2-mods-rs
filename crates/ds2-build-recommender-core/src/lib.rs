//! The `PvP` build recommender panel, as pure data: state, weapon table and backend contract.
//!
//! The in-game panel (`ds2-build-recommender-ui`) draws what this crate says and routes presses
//! into it. Every decision that does not need a running game is made here and tested on the host:
//!
//! * [`weapons`] -- the compiled-in weapon table, its search filter, and each weapon's infusions.
//! * [`model`] -- the panel's state: nine stats, the soul level they make, the mode and its options.
//! * [`nav`] -- the panel without a mouse: the D-pad cursor's grid, and Left/Right on a number.
//! * [`backend`] -- the [`backend::RecommenderBackend`] contract the panel asks, the
//!   [`backend::StubBackend`] that answers it when there is no data file, and the rules that hold
//!   whichever backend answers: results are never shown for a build under its floors, a generated
//!   build lists 15 one-handed and 5 two-hand-only weapons, and its rings are granted three copies
//!   to a suggestion.
//! * [`corpus`] -- [`corpus::CorpusBackend`], the ranking in `scripts/ds2-builds-recommend.py`
//!   ported, over the data file its `--export-backend` writes.
//! * [`flex`] -- a build's weapon flexibility, how it ranks among its stat neighbours, and the
//!   line the panel shows for it.
//!
//! [`backend::to_import`] is the seam to `ds2-build-import`: a generated build becomes the same
//! [`ds2_build_import_core::Build`] a soulsplanner link does, plus the extra copies the planner's
//! four ring slots cannot express.

pub mod backend;
pub mod corpus;
pub mod flex;
pub mod model;
pub mod nav;
pub mod weapons;

pub use backend::{
    Answer, Calibration, GeneratedBuild, OptimizedBuild, Outcome, RecommenderBackend, ResultRow,
    StubBackend, ask, generate, to_import,
};
pub use corpus::CorpusBackend;
pub use model::{Grip, Mode, Objective, PanelState, STAT_COUNT, STAT_LABELS, StatusFilter};
pub use weapons::{WeaponKind, WeaponRow};
