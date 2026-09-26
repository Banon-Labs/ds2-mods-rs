//! The Load from URL field on the quit tab, as behaviour with no window, font or keyboard behind it.
//!
//! The row opens a one-line field prefilled with [`BUILD_URL_PREFIX`]. The player types or pastes
//! a build id after it; enter hands the text back, escape throws it away. Everything in this crate
//! is the part of that which can be decided without the game: what a keystroke does to the text,
//! when the field is finished, what a refused link re-opens with, and whether a latched request is
//! still real.
//!
//! # What it builds on, and what it adds
//!
//! `ds2_build_import_core` already owns the two rules this sits between, and they are used as they
//! are rather than copied:
//!
//! * [`ds2_build_import_core::Field`] is the character-level model: caret, surrogate pairs, the
//!   length bound, control codes turned into reactions.
//! * [`ds2_build_import_core::build_id_from_url`] is the link grammar, and says why a link failed.
//!
//! What this crate adds is the lifecycle around them, ported from the engine-free half of
//! `er-mods-rs`'s link field:
//!
//! | module | question |
//! |---|---|
//! | [`editor`] | is the field still open, and if not, did it end in a submit or a cancel |
//! | [`clipboard`] | is pasted or copied text a link worth putting in the field |
//! | [`session`] | after a submit: import it, re-open it for correction, or stop asking |
//! | [`row`] | is a latched request still backed by a field, and has a queued one gone unsubmitted |
//!
//! # The path form, not the fragment form
//!
//! soulsplanner inlines the build into the page it serves for `/darksouls2/<id>`. A fragment is
//! never sent to a server, so `/darksouls2/#<id>` fetches the empty planner every time.
//! [`build_id`] refuses it before any request is made. `scripts/ds2-soulsplanner.py` is where that
//! was established against the live site.

#![forbid(unsafe_code)]

pub mod clipboard;
pub mod editor;
pub mod row;
pub mod session;

pub use ds2_build_import_core::{BUILD_URL_PREFIX, MAX_UNITS as MAX_LEN, UrlRejection};
pub use editor::{Editor, Event};
pub use session::{MAX_REJECTED_REOPENS, Session, Verdict};

/// The soulsplanner build id in `url`, or `None` when there is not one.
///
/// Accepts the path form `/darksouls2/<id>`, with or without a scheme, `www.`, a trailing slash or
/// a query string. Refuses the fragment form `/darksouls2/#<id>`, another host, a bare prefix and
/// an id that is not all digits. [`ds2_build_import_core::build_id_from_url`] gives the reason
/// where this gives only the answer.
#[must_use]
pub fn build_id(url: &str) -> Option<u32> {
    ds2_build_import_core::build_id_from_url(url).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_path_form_yields_its_id() {
        assert_eq!(
            build_id("https://soulsplanner.com/darksouls2/253"),
            Some(253)
        );
        assert_eq!(build_id("soulsplanner.com/darksouls2/253/"), Some(253));
        assert_eq!(
            build_id("  https://www.soulsplanner.com/darksouls2/253?x=1\n"),
            Some(253)
        );
    }

    #[test]
    fn the_fragment_form_is_refused() {
        assert_eq!(build_id("https://soulsplanner.com/darksouls2/#253"), None);
        assert_eq!(build_id("https://soulsplanner.com/darksouls2#253"), None);
    }

    #[test]
    fn the_prefix_alone_is_not_a_build() {
        assert_eq!(build_id(BUILD_URL_PREFIX), None);
        assert_eq!(build_id(""), None);
    }

    #[test]
    fn other_links_and_ids_are_refused() {
        assert_eq!(build_id("https://soulsplanner.com/eldenring/253"), None);
        assert_eq!(build_id("https://example.com/darksouls2/253"), None);
        assert_eq!(build_id("https://soulsplanner.com/darksouls2/25a3"), None);
        assert_eq!(
            build_id("https://soulsplanner.com/darksouls2/99999999999"),
            None
        );
    }

    #[test]
    fn the_prefix_is_the_link_up_to_the_id() {
        assert_eq!(BUILD_URL_PREFIX, "https://soulsplanner.com/darksouls2/");
        assert_eq!(build_id(&format!("{BUILD_URL_PREFIX}253")), Some(253));
    }
}
