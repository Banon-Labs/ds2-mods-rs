//! The link the player types, and the build id inside it.
//!
//! # The path form is the only one that works
//!
//! soulsplanner renders the planner from a script the SERVER inlines into the page. Ask for
//! `/darksouls2/253` and the first `<body><script>` carries `savedBuild={...}`; ask for
//! `/darksouls2/#253` and it carries the empty bootstrap `;var plannerId='darksouls2';` forever,
//! because a fragment is never sent to the server and the planner's own JS never reads
//! `location.hash`. Waiting longer does not help -- there is nothing coming.
//!
//! So the fragment form is REFUSED HERE rather than fetched and then found empty. The player gets
//! told which form to use while the link is still in front of them, instead of watching a request
//! go out and come back with nothing. `scripts/ds2-soulsplanner.py` documents the same finding.

/// What the field opens with: the link, up to and including the slash the id follows.
///
/// The player types or pastes the numeric build id after it. This is the whole reason the prefill
/// exists -- everything left of the id is the same for every build on the site.
pub const BUILD_URL_PREFIX: &str = "https://soulsplanner.com/darksouls2/";

/// The one-line help under the row, before anything has been typed.
pub const BUILD_URL_ROW_HELP: &str = "Enter a soulsplanner.com build link";

/// Why a link was not accepted.
///
/// Every variant is a DISTINCT thing the player can do about it, which is the only reason to have
/// more than one: the text of [`Self::indicator`] is what goes under the row, and "that is not a
/// link" would be the same unhelpful sentence for all five.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UrlRejection {
    /// Nothing was entered, or the prefix was accepted unchanged with no id after it.
    Empty,
    /// The link is not a soulsplanner Dark Souls 2 link at all.
    NotSoulsplanner,
    /// The `/darksouls2/#253` form. Fetchable, and always empty -- see the module docs.
    FragmentForm,
    /// Something followed the slash, and it was not a build id.
    IdNotNumeric,
    /// The id does not fit a `u32`. The site's own ids are four digits.
    IdTooLarge,
}

impl UrlRejection {
    /// The sentence that goes under the row. Short, because the row is narrow.
    pub const fn indicator(self) -> &'static str {
        match self {
            UrlRejection::Empty => "Enter a build id after the last slash",
            UrlRejection::NotSoulsplanner => "That is not a soulsplanner.com/darksouls2 link",
            UrlRejection::FragmentForm => "Drop the # -- use /darksouls2/253, not /darksouls2/#253",
            UrlRejection::IdNotNumeric => "The build id must be digits only",
            UrlRejection::IdTooLarge => "That build id is too large to be real",
        }
    }
}

impl std::fmt::Display for UrlRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.indicator())
    }
}

/// The build id in a soulsplanner link, or why there is not one.
///
/// Accepts the link with or without a scheme and with or without `www.`, because a link copied out
/// of a browser's address bar has the scheme and one recited from memory does not. Trailing
/// slashes, query strings and whitespace are tolerated; a fragment is not, and
/// [`UrlRejection::FragmentForm`] says why.
pub fn build_id_from_url(url: &str) -> Result<u32, UrlRejection> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(UrlRejection::Empty);
    }

    // THE HOST IS MATCHED WITHOUT ITS SCHEME so that one function handles both what a browser
    // copies and what a player types. Everything after the game segment is the id.
    let rest = strip_prefix_ignore_ascii_case(trimmed, "https://")
        .or_else(|| strip_prefix_ignore_ascii_case(trimmed, "http://"))
        .unwrap_or(trimmed);
    let rest = strip_prefix_ignore_ascii_case(rest, "www.").unwrap_or(rest);
    let Some(rest) = strip_prefix_ignore_ascii_case(rest, "soulsplanner.com/") else {
        return Err(UrlRejection::NotSoulsplanner);
    };
    let Some(rest) = strip_prefix_ignore_ascii_case(rest, "darksouls2") else {
        return Err(UrlRejection::NotSoulsplanner);
    };
    let rest = rest.strip_prefix('/').unwrap_or(rest);

    // A query string is someone else's business; the id ends where one begins.
    let rest = rest.split('?').next().unwrap_or(rest);
    let rest = rest.trim_end_matches('/').trim();

    if let Some(after_hash) = rest.strip_prefix('#') {
        // Named separately from `IdNotNumeric` because the link is otherwise PERFECTLY VALID and
        // will fetch a page -- just never one with a build in it.
        return Err(if after_hash.trim().is_empty() {
            UrlRejection::Empty
        } else {
            UrlRejection::FragmentForm
        });
    }
    if rest.is_empty() {
        return Err(UrlRejection::Empty);
    }
    if !rest.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(UrlRejection::IdNotNumeric);
    }
    rest.parse::<u32>().map_err(|_| UrlRejection::IdTooLarge)
}

/// The path to GET for a build id: what [`build_id_from_url`] accepted, in its canonical form.
///
/// Host and path are separate because that is how `ds2_game_base::http::get` takes them.
pub fn build_path(build_id: u32) -> String {
    format!("/darksouls2/{build_id}")
}

/// The host every build lives on.
pub const BUILD_HOST: &str = "soulsplanner.com";

/// `str::strip_prefix`, case-insensitively, for ASCII.
///
/// Hand-written because the only alternative is allocating a lowercase copy of the whole link to
/// test five bytes of it, and this crate has no allocator to spare and no dependencies to ask.
fn strip_prefix_ignore_ascii_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The prefill is a prefix of a real link, and on its own it is not one yet.
    #[test]
    fn the_prefill_is_the_link_up_to_the_id() {
        assert!(BUILD_URL_PREFIX.ends_with('/'));
        assert_eq!(
            build_id_from_url(BUILD_URL_PREFIX),
            Err(UrlRejection::Empty)
        );
        assert_eq!(
            build_id_from_url(&format!("{BUILD_URL_PREFIX}253")),
            Ok(253)
        );
    }

    /// The id survives every spelling a player can arrive with.
    #[test]
    fn one_id_is_reached_by_every_spelling_of_the_link() {
        for spelling in [
            "https://soulsplanner.com/darksouls2/253",
            "http://soulsplanner.com/darksouls2/253",
            "https://www.soulsplanner.com/darksouls2/253",
            "soulsplanner.com/darksouls2/253",
            "SoulsPlanner.COM/DarkSouls2/253",
            "  https://soulsplanner.com/darksouls2/253  ",
            "https://soulsplanner.com/darksouls2/253/",
            "https://soulsplanner.com/darksouls2/253?share=1",
        ] {
            assert_eq!(build_id_from_url(spelling), Ok(253), "{spelling}");
        }
    }

    /// The fragment form is refused BEFORE the fetch, and says what to do instead.
    ///
    /// It is the one rejection that would otherwise cost a round trip and return an empty page --
    /// the exact failure `scripts/ds2-soulsplanner.py` exists to document.
    #[test]
    fn the_fragment_form_is_refused_by_name() {
        assert_eq!(
            build_id_from_url("https://soulsplanner.com/darksouls2/#253"),
            Err(UrlRejection::FragmentForm)
        );
        assert!(UrlRejection::FragmentForm.indicator().contains('#'));
        // An empty fragment is an empty id, not a lecture about fragments.
        assert_eq!(
            build_id_from_url("https://soulsplanner.com/darksouls2/#"),
            Err(UrlRejection::Empty)
        );
    }

    /// Every other refusal names its own cause.
    #[test]
    fn each_refusal_is_a_different_thing_to_do_about_it() {
        assert_eq!(build_id_from_url(""), Err(UrlRejection::Empty));
        assert_eq!(build_id_from_url("   "), Err(UrlRejection::Empty));
        assert_eq!(
            build_id_from_url("https://example.com/darksouls2/253"),
            Err(UrlRejection::NotSoulsplanner)
        );
        assert_eq!(
            build_id_from_url("https://soulsplanner.com/eldenring/253"),
            Err(UrlRejection::NotSoulsplanner)
        );
        assert_eq!(
            build_id_from_url("https://soulsplanner.com/darksouls2/abc"),
            Err(UrlRejection::IdNotNumeric)
        );
        assert_eq!(
            build_id_from_url("https://soulsplanner.com/darksouls2/99999999999"),
            Err(UrlRejection::IdTooLarge)
        );
        // No two indicators read the same, or the variant split bought nothing.
        let all = [
            UrlRejection::Empty,
            UrlRejection::NotSoulsplanner,
            UrlRejection::FragmentForm,
            UrlRejection::IdNotNumeric,
            UrlRejection::IdTooLarge,
        ];
        for (index, one) in all.iter().enumerate() {
            for other in &all[index + 1..] {
                assert_ne!(one.indicator(), other.indicator());
            }
        }
    }

    /// A short link cannot panic the prefix test by slicing a `char` in half.
    #[test]
    fn a_multibyte_link_does_not_split_a_char() {
        assert_eq!(build_id_from_url("h"), Err(UrlRejection::NotSoulsplanner));
        assert_eq!(build_id_from_url("é"), Err(UrlRejection::NotSoulsplanner));
        assert_eq!(
            build_id_from_url("https://soulsplanner.com/darksouls2/2é3"),
            Err(UrlRejection::IdNotNumeric)
        );
    }

    /// The fetch path is the canonical form of what was accepted.
    #[test]
    fn the_path_is_the_form_that_carries_a_build() {
        assert_eq!(build_path(253), "/darksouls2/253");
        assert_eq!(
            build_id_from_url(&format!("{BUILD_HOST}{}", build_path(253))),
            Ok(253)
        );
    }
}
