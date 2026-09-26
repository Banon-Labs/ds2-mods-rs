//! One press of the row, across however many times the field has to be re-opened.
//!
//! A submitted link that does not validate is not an error to show and forget. The player is put
//! back in the field with what they typed still there and the reason on the row, so a typo is
//! corrected rather than retyped. That is `er-mods-rs`'s rule, and it came with two refinements
//! this keeps:
//!
//! * An empty submit is not a typo. Re-opening an empty box leaves nothing to build on, so it
//!   restarts from the clipboard link or the prefix instead (see [`reopen_text`]).
//! * The chain is bounded by [`MAX_REJECTED_REOPENS`]. Escape always ends it, so the bound is not
//!   the way out for a player; it is what stops a submit that keeps failing for a reason the
//!   player cannot fix from re-arming the field forever.

use ds2_build_import_core::{UrlRejection, build_id_from_url};

use crate::clipboard::initial_text;

/// How many times one press may re-open the field after a refused link before it stops asking.
pub const MAX_REJECTED_REOPENS: usize = 8;

/// What to do with a submitted link.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Import this build. `url` is the submitted text, trimmed.
    Accept {
        /// The build id from the path form.
        build_id: u32,
        /// The link as submitted, surrounding whitespace removed.
        url: String,
    },
    /// Open the field again on `text`, and show `rejection` on the row.
    Reopen {
        /// What the new field opens with.
        text: String,
        /// Why the last submit was refused.
        rejection: UrlRejection,
    },
    /// Stop re-opening. The row shows `rejection`; the player can press it again.
    GiveUp {
        /// Why the last submit was refused.
        rejection: UrlRejection,
    },
}

/// The state one row press carries between field openings.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Session {
    reopens: usize,
}

impl Session {
    /// A fresh press: no refusals yet.
    #[must_use]
    pub const fn new() -> Self {
        Self { reopens: 0 }
    }

    /// How many times this press has re-opened after a refusal.
    #[must_use]
    pub const fn reopens(&self) -> usize {
        self.reopens
    }

    /// Judge a submitted link.
    ///
    /// `clipboard` is the clipboard as the runtime last read it, used only when the submit was
    /// empty and the field needs something to re-open with.
    pub fn on_submitted(&mut self, text: &str, clipboard: Option<&str>) -> Verdict {
        match build_id_from_url(text) {
            Ok(build_id) => Verdict::Accept {
                build_id,
                url: text.trim().to_owned(),
            },
            Err(rejection) => {
                self.reopens += 1;
                if self.reopens > MAX_REJECTED_REOPENS {
                    Verdict::GiveUp { rejection }
                } else {
                    Verdict::Reopen {
                        text: reopen_text(text, clipboard),
                        rejection,
                    }
                }
            }
        }
    }
}

/// What a refused submit should put back in the field.
///
/// The text itself, unchanged, unless it was blank -- then whatever a first opening would have
/// used, from [`initial_text`].
#[must_use]
pub fn reopen_text(rejected: &str, clipboard: Option<&str>) -> String {
    if rejected.trim().is_empty() {
        initial_text(clipboard)
    } else {
        rejected.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ds2_build_import_core::BUILD_URL_PREFIX;

    /// Ported: a player cleared the field and accepted, and the refusal re-opened an empty box.
    #[test]
    fn an_empty_accept_reopens_with_something_to_build_on() {
        for cleared in ["", "   ", "\t\n"] {
            for clipboard in [
                None,
                Some("nonsense"),
                Some("https://soulsplanner.com/darksouls2/253"),
            ] {
                let reopened = reopen_text(cleared, clipboard);
                assert!(!reopened.trim().is_empty(), "{cleared:?} {clipboard:?}");
                assert!(
                    reopened == BUILD_URL_PREFIX || crate::build_id(&reopened).is_some(),
                    "the re-open text must be the prefix or a build link, got {reopened:?}"
                );
            }
        }
    }

    /// Ported: a real typo is carried back verbatim, which is the point of re-opening at all.
    #[test]
    fn a_typo_is_carried_back_verbatim_for_correction() {
        for typo in [
            "https://soulsplanner.com/darksouls2/",
            "https://soulsplanner.com/darksouls2/25x",
            "https://soulsplanner.com/darksouls2/#253",
            "nonsense",
        ] {
            assert_eq!(reopen_text(typo, None), typo);
        }
    }

    #[test]
    fn a_path_link_is_accepted_trimmed() {
        let mut session = Session::new();
        assert_eq!(
            session.on_submitted(" https://soulsplanner.com/darksouls2/253\n", None),
            Verdict::Accept {
                build_id: 253,
                url: "https://soulsplanner.com/darksouls2/253".to_owned()
            }
        );
        assert_eq!(session.reopens(), 0);
    }

    #[test]
    fn a_fragment_link_reopens_with_the_reason() {
        let mut session = Session::new();
        let link = "https://soulsplanner.com/darksouls2/#253";
        assert_eq!(
            session.on_submitted(link, None),
            Verdict::Reopen {
                text: link.to_owned(),
                rejection: UrlRejection::FragmentForm
            }
        );
    }

    #[test]
    fn the_bare_prefix_reopens_as_empty() {
        let mut session = Session::new();
        assert_eq!(
            session.on_submitted(BUILD_URL_PREFIX, None),
            Verdict::Reopen {
                text: BUILD_URL_PREFIX.to_owned(),
                rejection: UrlRejection::Empty
            }
        );
    }

    /// Ported: the re-open chain is finite, and the refusal after the last allowed re-open says so.
    #[test]
    fn the_reopen_chain_is_bounded() {
        let mut session = Session::new();
        for round in 1..=MAX_REJECTED_REOPENS {
            assert!(
                matches!(
                    session.on_submitted("nonsense", None),
                    Verdict::Reopen { .. }
                ),
                "round {round}"
            );
        }
        assert_eq!(
            session.on_submitted("nonsense", None),
            Verdict::GiveUp {
                rejection: UrlRejection::NotSoulsplanner
            }
        );
    }

    #[test]
    fn a_good_link_is_accepted_even_after_refusals() {
        let mut session = Session::new();
        session.on_submitted("nonsense", None);
        assert!(matches!(
            session.on_submitted("soulsplanner.com/darksouls2/253", None),
            Verdict::Accept { build_id: 253, .. }
        ));
    }
}
