//! A soulsplanner link, and the build the page behind it carries.
//!
//! Everything here is TEXT IN, TEXT OUT. No addresses, no Windows, no Dark Souls -- which is what
//! lets `check.sh --host-tests` run it, and what lets the parser be checked against a saved page
//! without launching anything. The half that fetches, draws and applies lives elsewhere.
//!
//! # The two halves, and the failure each one is for
//!
//! | module | question | the failure it names |
//! |---|---|---|
//! | [`url`] | is what the player typed a build link, and which build | the `#` form, which fetches fine and is always empty |
//! | [`saved_build`] | does this page carry a build, and what is in it | the bootstrap-only response, which is a valid page with nothing in it |
//!
//! Those two failures are the same failure seen from either side of a request, and both of them
//! look like success to anything that only checks for an error. The `#` form returns HTTP 200. So
//! [`url::build_id_from_url`] refuses the fragment BEFORE the fetch, and [`saved_build::parse`]
//! refuses the bootstrap page AFTER it, and neither one relies on a status code.
//!
//! # Provenance
//!
//! `scripts/ds2-soulsplanner.py` established all of this against the live site and is the
//! reference implementation; this crate is that script's three regexes, hand-written so the
//! workspace gains no dependency for a grammar one level deep. Build `253` fetched on 2026-08-28
//! is the fixture both agree on.

pub mod saved_build;
pub mod url;

pub use saved_build::{Build, MAX_STAT, ParseError, Stats};
pub use url::{
    BUILD_HOST, BUILD_URL_PREFIX, BUILD_URL_ROW_HELP, UrlRejection, build_id_from_url, build_path,
};

/// The caption on the pause-menu row that opens the field.
pub const ROW_CAPTION: &str = "Load from URL";
