//! Clipboard text, judged before it goes anywhere near the field.
//!
//! Reading and writing the clipboard is Win32 and lives in the runtime crate. What is decided here
//! is what to do with the text once it has been read, which is the part `er-mods-rs` found worth
//! getting right: a clipboard holding a paragraph must not become the field, and a link copied out
//! of a chat window arrives with a newline on the end.
//!
//! Two gates, for two moments:
//!
//! * [`normalise_paste`] is for Ctrl+V. The player asked for the clipboard, so whatever is there
//!   goes in, cleaned: surrounding whitespace trimmed, control characters dropped, length bounded.
//! * [`clipboard_build_url`] is for the prefill when the field opens. Nobody asked, so only a link
//!   that already names a build is used; anything else leaves the field at the bare prefix.

use ds2_build_import_core::{BUILD_URL_PREFIX, MAX_UNITS};

/// Longest clipboard text looked at, in characters.
///
/// Anything past it is not a link someone meant to paste. The bound is on how much of a large
/// clipboard gets walked, not on what the field keeps; the field's own bound is [`MAX_UNITS`].
pub const MAX_CLIPBOARD_CHARS: usize = MAX_UNITS * 2;

/// Clipboard text as the field should receive it from a paste.
///
/// Only the first [`MAX_CLIPBOARD_CHARS`] characters are considered. Of those, surrounding
/// whitespace is trimmed and every control character is dropped rather than turned into a space: a
/// space inside a link is a character that fails validation for a reason the player cannot see.
#[must_use]
pub fn normalise_paste(text: &str) -> String {
    let bounded: String = text.chars().take(MAX_CLIPBOARD_CHARS).collect();
    bounded
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .collect()
}

/// The clipboard's text when it is a link that names a build, trimmed; `None` otherwise.
///
/// Used only when validation passes, so an unrelated copy can never fill the field with something
/// the player then has to clear before typing.
#[must_use]
pub fn clipboard_build_url(text: &str) -> Option<String> {
    let cleaned = normalise_paste(text);
    crate::build_id(&cleaned)?;
    Some(cleaned)
}

/// What the field should open with: the clipboard when it holds a build link, else the bare prefix.
///
/// `clipboard` is whatever the runtime managed to read, or `None` when it read nothing.
#[must_use]
pub fn initial_text(clipboard: Option<&str>) -> String {
    clipboard
        .and_then(clipboard_build_url)
        .unwrap_or_else(|| BUILD_URL_PREFIX.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_from_a_chat_window_loses_its_newline() {
        assert_eq!(
            normalise_paste("https://soulsplanner.com/darksouls2/253\r\n"),
            "https://soulsplanner.com/darksouls2/253"
        );
    }

    #[test]
    fn control_characters_inside_are_dropped_not_spaced() {
        assert_eq!(normalise_paste("25\t3"), "253");
        assert_eq!(normalise_paste("2\u{7f}5\u{0}3"), "253");
    }

    #[test]
    fn a_huge_clipboard_is_bounded_before_it_is_walked() {
        let huge = "9".repeat(MAX_CLIPBOARD_CHARS * 4);
        assert_eq!(normalise_paste(&huge).chars().count(), MAX_CLIPBOARD_CHARS);
    }

    #[test]
    fn only_a_build_link_prefills_the_field() {
        assert_eq!(
            clipboard_build_url("  https://soulsplanner.com/darksouls2/253 \n").as_deref(),
            Some("https://soulsplanner.com/darksouls2/253")
        );
        for other in [
            "",
            "a paragraph someone copied",
            "https://soulsplanner.com/darksouls2/",
            "https://soulsplanner.com/darksouls2/#253",
        ] {
            assert_eq!(clipboard_build_url(other), None, "{other:?}");
        }
    }

    /// Ported: the field opens with the clipboard's link, or with the bare prefix -- never with
    /// arbitrary clipboard text and never empty.
    #[test]
    fn the_initial_text_is_the_link_or_the_prefix() {
        assert_eq!(
            initial_text(Some("https://soulsplanner.com/darksouls2/253")),
            "https://soulsplanner.com/darksouls2/253"
        );
        assert_eq!(initial_text(Some("nonsense")), BUILD_URL_PREFIX);
        assert_eq!(initial_text(None), BUILD_URL_PREFIX);
    }
}
