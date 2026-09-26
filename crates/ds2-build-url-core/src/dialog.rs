//! The link dialog as bytes and text rules: the template Win32 is handed, and what its answer means.
//!
//! The runtime crate opens a small modal dialog with `DialogBoxIndirectParamW`, owned by the game
//! window, when Steam cannot draw its own text field. Everything about that dialog which is not a
//! Win32 call lives here, so it can be checked on the host: the in-memory `DLGTEMPLATE` the call
//! reads, the text the edit control opens with, and what the text it closes with amounts to.
//!
//! # The template, byte for byte
//!
//! `DialogBoxIndirectParamW` takes a `DLGTEMPLATE` followed by variable-length arrays, and gets
//! every field from its offset rather than from a declared struct. That is why it is built by hand
//! and tested by offset below: a field one word out of place does not fail to compile, it opens a
//! dialog with no controls or does not open one at all.
//!
//! The layout (all integers little-endian):
//!
//! * `DLGTEMPLATE`: style, extended style, item count, then x, y, cx, cy in dialog units.
//! * Menu, class and title, each a UTF-16 string or a zero word for none.
//! * Because `DS_SETFONT` is set: a point size and a typeface name.
//! * Each control, starting on a four-byte boundary: a `DLGITEMTEMPLATE` (style, extended style,
//!   x, y, cx, cy, id), then its class as `0xFFFF` and an atom, its text, and a zero word for no
//!   creation data.
//!
//! # Enter and Escape need no code of ours
//!
//! The dialog manager already maps them: Enter presses the default push button, which is `IDOK`,
//! and Escape sends `IDCANCEL` whether or not a Cancel button exists. The dialog procedure only has
//! to answer those two commands.

use crate::clipboard::{initial_text, normalise_paste};
use ds2_build_import_core::{MAX_UNITS, UrlRejection, build_id_from_url};

/// The dialog's caption.
pub const CAPTION: &str = "Load a build from a URL";

/// The line above the edit control.
pub const PROMPT: &str = "Paste or type a soulsplanner.com/darksouls2 build link:";

/// `IDOK`: the default button, and what Enter presses.
pub const ID_OK: u16 = 1;

/// `IDCANCEL`: the Cancel button, and what Escape sends.
pub const ID_CANCEL: u16 = 2;

/// The edit control's id. Any value clear of `IDOK`/`IDCANCEL` and the static's `-1`.
pub const ID_EDIT: u16 = 100;

/// The static label's id, `-1` as a word: it is never addressed.
const ID_STATIC: u16 = 0xFFFF;

/// The most UTF-16 units the edit control accepts, handed to it as `EM_LIMITTEXT`.
///
/// The same bound the in-game field keeps, so a link that fits one fits the other.
pub const TEXT_LIMIT: usize = MAX_UNITS;

/// `WS_POPUP`.
pub const WS_POPUP: u32 = 0x8000_0000;
/// `WS_CHILD`.
pub const WS_CHILD: u32 = 0x4000_0000;
/// `WS_VISIBLE`.
pub const WS_VISIBLE: u32 = 0x1000_0000;
/// `WS_CAPTION`: a title bar, which is where [`CAPTION`] goes.
pub const WS_CAPTION: u32 = 0x00C0_0000;
/// `WS_BORDER`.
pub const WS_BORDER: u32 = 0x0080_0000;
/// `WS_SYSMENU`: the close box, which the dialog manager turns into `IDCANCEL`.
pub const WS_SYSMENU: u32 = 0x0008_0000;
/// `WS_TABSTOP`.
pub const WS_TABSTOP: u32 = 0x0001_0000;
/// `DS_SETFONT`: a point size and typeface follow the title.
pub const DS_SETFONT: u32 = 0x0040;
/// `DS_MODALFRAME`.
pub const DS_MODALFRAME: u32 = 0x0080;
/// `DS_SETFOREGROUND`: ask for the foreground when the dialog is created.
///
/// Set because the owner is a game that may be fullscreen; a dialog behind it reads as a freeze.
pub const DS_SETFOREGROUND: u32 = 0x0200;
/// `DS_CENTER`: centred on the owner's monitor.
pub const DS_CENTER: u32 = 0x0800;
/// `ES_AUTOHSCROLL`: a link longer than the box scrolls instead of refusing further typing.
pub const ES_AUTOHSCROLL: u32 = 0x0080;
/// `BS_DEFPUSHBUTTON`: the button Enter presses.
pub const BS_DEFPUSHBUTTON: u32 = 0x0001;
/// `SS_LEFT`, which is zero, named so the label's style reads as a choice.
pub const SS_LEFT: u32 = 0x0000;

/// Predefined class atoms for a control's class field.
const ATOM_BUTTON: u16 = 0x0080;
const ATOM_EDIT: u16 = 0x0081;
const ATOM_STATIC: u16 = 0x0082;

/// The dialog's own style.
pub const DIALOG_STYLE: u32 =
    WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME | DS_SETFONT | DS_SETFOREGROUND | DS_CENTER;

/// The font the template names. The size is in points.
const FONT_POINTS: u16 = 9;
const FONT_FACE: &str = "Segoe UI";

/// One control in the template.
struct Item {
    style: u32,
    rect: [i16; 4],
    id: u16,
    class: u16,
    text: &'static str,
}

/// The controls, in tab order: label, edit, OK, Cancel.
///
/// Rectangles are dialog units within a dialog [`SIZE`] wide and tall.
const ITEMS: [Item; 4] = [
    Item {
        style: WS_CHILD | WS_VISIBLE | SS_LEFT,
        rect: [7, 7, 286, 9],
        id: ID_STATIC,
        class: ATOM_STATIC,
        text: PROMPT,
    },
    Item {
        style: WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
        rect: [7, 19, 286, 14],
        id: ID_EDIT,
        class: ATOM_EDIT,
        text: "",
    },
    Item {
        style: WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON,
        rect: [189, 40, 50, 14],
        id: ID_OK,
        class: ATOM_BUTTON,
        text: "OK",
    },
    Item {
        style: WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        rect: [243, 40, 50, 14],
        id: ID_CANCEL,
        class: ATOM_BUTTON,
        text: "Cancel",
    },
];

/// The dialog's width and height in dialog units.
const SIZE: [i16; 2] = [300, 61];

/// Append a little-endian word.
fn word(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Append a little-endian double word.
fn dword(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Append a UTF-16 string and its terminator.
fn wide(out: &mut Vec<u8>, text: &str) {
    for unit in text.encode_utf16() {
        word(out, unit);
    }
    word(out, 0);
}

/// Pad with zero bytes to the next four-byte boundary.
fn align4(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

/// The whole in-memory template, padded to a multiple of four bytes.
///
/// The caller must copy it into four-byte-aligned memory before handing it to Win32: the template
/// and every control in it are required to start on a `DWORD` boundary, and the offsets here are
/// only correct relative to such a start.
#[must_use]
pub fn template() -> Vec<u8> {
    let mut out = Vec::with_capacity(512);
    dword(&mut out, DIALOG_STYLE);
    dword(&mut out, 0);
    word(&mut out, ITEMS.len() as u16);
    for value in [0, 0, SIZE[0], SIZE[1]] {
        word(&mut out, value as u16);
    }
    word(&mut out, 0); // no menu
    word(&mut out, 0); // the default dialog class
    wide(&mut out, CAPTION);
    word(&mut out, FONT_POINTS);
    wide(&mut out, FONT_FACE);
    for item in &ITEMS {
        align4(&mut out);
        dword(&mut out, item.style);
        dword(&mut out, 0);
        for value in item.rect {
            word(&mut out, value as u16);
        }
        word(&mut out, item.id);
        word(&mut out, 0xFFFF);
        word(&mut out, item.class);
        wide(&mut out, item.text);
        word(&mut out, 0); // no creation data
    }
    align4(&mut out);
    out
}

/// What the edit control opens with: the clipboard's text when it is a soulsplanner build link,
/// and the bare link prefix otherwise.
///
/// A thin name over [`initial_text`] so the dialog's prefill and the in-game field's cannot drift.
#[must_use]
pub fn prefill(clipboard: Option<&str>) -> String {
    initial_text(clipboard)
}

/// What the player left in the edit control when they pressed OK, cleaned and judged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The text with surrounding whitespace and control characters removed.
    pub text: String,
    /// The build id in it, or why there is not one.
    pub build: Result<u32, UrlRejection>,
}

/// Clean the returned text and read a build id out of it.
///
/// Cleaned the way a paste is, because the text very often was one: a link copied out of a chat
/// window brings its newline, and the edit control keeps it.
#[must_use]
pub fn read_entry(raw: &str) -> Entry {
    let text = normalise_paste(raw);
    let build = build_id_from_url(&text);
    Entry { text, build }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ds2_build_import_core::BUILD_URL_PREFIX;

    fn u16_at(bytes: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([bytes[at], bytes[at + 1]])
    }

    fn u32_at(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    /// Read a terminated UTF-16 string at `at`; returns it and the offset just past its terminator.
    fn wide_at(bytes: &[u8], mut at: usize) -> (String, usize) {
        let mut units = Vec::new();
        loop {
            let unit = u16_at(bytes, at);
            at += 2;
            if unit == 0 {
                break;
            }
            units.push(unit);
        }
        (String::from_utf16(&units).unwrap(), at)
    }

    /// Walk the template the way the dialog manager does, and get back what was put in.
    #[test]
    fn the_template_parses_back_into_the_dialog_it_describes() {
        let bytes = template();
        assert!(bytes.len().is_multiple_of(4));
        assert_eq!(u32_at(&bytes, 0), DIALOG_STYLE);
        assert_eq!(u32_at(&bytes, 4), 0);
        assert_eq!(u16_at(&bytes, 8) as usize, ITEMS.len());
        assert_eq!(u16_at(&bytes, 14) as i16, SIZE[0]);
        assert_eq!(u16_at(&bytes, 16) as i16, SIZE[1]);
        assert_eq!(u16_at(&bytes, 18), 0, "menu");
        assert_eq!(u16_at(&bytes, 20), 0, "class");
        let (title, at) = wide_at(&bytes, 22);
        assert_eq!(title, CAPTION);
        assert_eq!(u16_at(&bytes, at), FONT_POINTS);
        let (face, mut at) = wide_at(&bytes, at + 2);
        assert_eq!(face, FONT_FACE);

        let mut seen = Vec::new();
        for _ in 0..ITEMS.len() {
            at = at.next_multiple_of(4);
            let style = u32_at(&bytes, at);
            let id = u16_at(&bytes, at + 16);
            assert_eq!(u16_at(&bytes, at + 18), 0xFFFF, "class by atom");
            let class = u16_at(&bytes, at + 20);
            let (text, next) = wide_at(&bytes, at + 22);
            assert_eq!(u16_at(&bytes, next), 0, "creation data");
            at = next + 2;
            seen.push((style, id, class, text));
        }
        assert_eq!(
            at.next_multiple_of(4),
            bytes.len(),
            "nothing after the last control"
        );

        let find = |id: u16| seen.iter().find(|entry| entry.1 == id).unwrap();
        assert_eq!(find(ID_EDIT).2, ATOM_EDIT);
        assert_eq!(find(ID_OK).2, ATOM_BUTTON);
        assert_eq!(find(ID_OK).3, "OK");
        assert_eq!(find(ID_CANCEL).3, "Cancel");
        assert_eq!(find(ID_STATIC).3, PROMPT);
    }

    /// Enter must land on OK and nothing else may claim to be the default.
    #[test]
    fn ok_is_the_only_default_button() {
        let defaults: Vec<u16> = ITEMS
            .iter()
            .filter(|item| item.class == ATOM_BUTTON && item.style & BS_DEFPUSHBUTTON != 0)
            .map(|item| item.id)
            .collect();
        assert_eq!(defaults, [ID_OK]);
    }

    /// The edit control takes focus first in tab order, scrolls, and can be tabbed to.
    #[test]
    fn the_edit_control_is_the_first_tab_stop() {
        let first = ITEMS
            .iter()
            .find(|item| item.style & WS_TABSTOP != 0)
            .unwrap();
        assert_eq!(first.id, ID_EDIT);
        assert_ne!(first.style & ES_AUTOHSCROLL, 0);
    }

    /// Every control fits inside the dialog, so none is clipped.
    #[test]
    fn every_control_fits_the_dialog() {
        for item in &ITEMS {
            let [x, y, cx, cy] = item.rect;
            assert!(x >= 0 && y >= 0);
            assert!(x + cx <= SIZE[0], "{:?}", item.text);
            assert!(y + cy <= SIZE[1], "{:?}", item.text);
        }
    }

    /// The edit control can hold the prefix with an id after it.
    #[test]
    fn the_limit_holds_a_whole_link() {
        assert!(TEXT_LIMIT > BUILD_URL_PREFIX.len() + 10);
    }

    #[test]
    fn the_prefill_is_a_build_link_or_the_prefix() {
        assert_eq!(
            prefill(Some("https://soulsplanner.com/darksouls2/253\r\n")),
            "https://soulsplanner.com/darksouls2/253"
        );
        assert_eq!(prefill(Some("https://example.com/")), BUILD_URL_PREFIX);
        assert_eq!(prefill(Some(BUILD_URL_PREFIX)), BUILD_URL_PREFIX);
        assert_eq!(prefill(None), BUILD_URL_PREFIX);
    }

    #[test]
    fn a_returned_link_is_trimmed_and_read() {
        assert_eq!(
            read_entry("  https://soulsplanner.com/darksouls2/253\r\n"),
            Entry {
                text: "https://soulsplanner.com/darksouls2/253".to_owned(),
                build: Ok(253),
            }
        );
    }

    #[test]
    fn a_returned_prefix_or_fragment_is_refused_with_its_reason() {
        assert_eq!(read_entry(BUILD_URL_PREFIX).build, Err(UrlRejection::Empty));
        assert_eq!(read_entry("").build, Err(UrlRejection::Empty));
        assert_eq!(
            read_entry("https://soulsplanner.com/darksouls2/#253").build,
            Err(UrlRejection::FragmentForm)
        );
        assert_eq!(
            read_entry("https://soulsplanner.com/eldenring/253").build,
            Err(UrlRejection::NotSoulsplanner)
        );
    }
}
