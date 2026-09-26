//! The field from open to finished: characters, backspace, paste, enter, escape.
//!
//! [`ds2_build_import_core::Field`] answers what one character does to the text. It does not know
//! when the field is over, so a caller holding one has to remember that enter was pressed and stop
//! feeding it. [`Editor`] is that memory: once enter or escape arrives it is closed, it reports how
//! it closed exactly once, and every later key is refused without touching the text.
//!
//! # Input arrives as characters
//!
//! DARK SOULS II translates its own messages, so what reaches a hook is `WM_CHAR`: a UTF-16 unit
//! with layout, shift and dead keys already applied. [`Editor::on_char`] takes exactly that. Enter,
//! escape, backspace and Ctrl+V all come through the same door, which is why paste needs no chord
//! tracking here. The named methods ([`Editor::enter`], [`Editor::paste`] and the rest) are the
//! same operations for a caller that has already decoded the key.
//!
//! # Ported semantics
//!
//! From `er-mods-rs`'s link field: the caret starts after the prefill so the first key appends,
//! accept yields the text and back-out yields nothing, and a paste is cleaned before it lands
//! (see [`crate::clipboard::normalise_paste`]). Elden Ring's field had no paste of its own and
//! mirrored the clipboard every frame instead; that polling has no counterpart here because this
//! engine delivers Ctrl+V as a character.

use ds2_build_import_core::{BUILD_URL_PREFIX, Field, Reaction};

use crate::clipboard::normalise_paste;

/// `WM_CHAR` for Ctrl+X, the one control code whose text has to be captured before it is fed.
const CUT: u16 = 0x18;

/// What one input did to the editor.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Event {
    /// The field is still open. The text may or may not have changed; redraw either way.
    Editing,
    /// Ctrl+V. Read the clipboard and hand it to [`Editor::paste`].
    PasteRequested,
    /// Ctrl+C or Ctrl+X. Put this text on the clipboard.
    ///
    /// Carried in the event rather than read back from the editor, because a cut has already
    /// emptied the field by the time the caller could ask.
    Copy(String),
    /// Enter. The field is closed, and this is what it held.
    Submitted(String),
    /// Escape. The field is closed and nothing is to be done with its text.
    Cancelled,
    /// The field had already closed, and this input was ignored.
    Closed,
}

/// Where the editor is in its life.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Open,
    Submitted,
    Cancelled,
}

/// A one-line field that ends in a submit or a cancel.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Editor {
    field: Field,
    phase: Phase,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    /// A field holding [`BUILD_URL_PREFIX`], caret at the end, open.
    #[must_use]
    pub fn new() -> Self {
        Self::with_text(BUILD_URL_PREFIX)
    }

    /// A field holding `text`, caret at the end, open. What a re-open after a refusal uses.
    #[must_use]
    pub fn with_text(text: &str) -> Self {
        Self {
            field: Field::new(text),
            phase: Phase::Open,
        }
    }

    /// The text as it stands.
    #[must_use]
    pub fn text(&self) -> String {
        self.field.text()
    }

    /// The caret, in characters from the start.
    #[must_use]
    pub fn caret(&self) -> usize {
        self.field.caret()
    }

    /// The text with `mark` drawn where the caret is, for a surface with no caret of its own.
    #[must_use]
    pub fn text_with_caret(&self, mark: char) -> String {
        self.field.text_with_caret(mark)
    }

    /// Whether the field still takes input.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.phase == Phase::Open
    }

    /// Feed one `WM_CHAR` unit.
    pub fn on_char(&mut self, unit: u16) -> Event {
        if !self.is_open() {
            return Event::Closed;
        }
        // A cut empties the field inside `Field::on_char`, so its text is taken first.
        let before_cut = (unit == CUT).then(|| self.field.text());
        match self.field.on_char(unit) {
            Reaction::Handled | Reaction::Ignored => Event::Editing,
            Reaction::PasteRequested => Event::PasteRequested,
            Reaction::CopyRequested => Event::Copy(before_cut.unwrap_or_else(|| self.field.text())),
            Reaction::Confirm => self.enter(),
            Reaction::Cancel => self.escape(),
        }
    }

    /// Insert one printable character at the caret.
    ///
    /// A control character is ignored here; enter, escape and backspace have their own methods, so
    /// a stray `'\r'` in the input cannot submit the field by accident.
    pub fn insert_char(&mut self, character: char) -> Event {
        if !self.is_open() {
            return Event::Closed;
        }
        if character.is_control() {
            return Event::Editing;
        }
        let mut units = [0_u16; 2];
        for unit in character.encode_utf16(&mut units) {
            self.field.on_char(*unit);
        }
        Event::Editing
    }

    /// Delete the character before the caret.
    pub fn backspace(&mut self) -> Event {
        if !self.is_open() {
            return Event::Closed;
        }
        self.field.backspace();
        Event::Editing
    }

    /// Insert clipboard text at the caret, cleaned by [`normalise_paste`] and cut at the length
    /// bound.
    pub fn paste(&mut self, clipboard: &str) -> Event {
        if !self.is_open() {
            return Event::Closed;
        }
        self.field.paste(&normalise_paste(clipboard));
        Event::Editing
    }

    /// Close the field and yield its text.
    pub fn enter(&mut self) -> Event {
        if !self.is_open() {
            return Event::Closed;
        }
        self.phase = Phase::Submitted;
        Event::Submitted(self.field.text())
    }

    /// Close the field and yield nothing.
    pub fn escape(&mut self) -> Event {
        if !self.is_open() {
            return Event::Closed;
        }
        self.phase = Phase::Cancelled;
        Event::Cancelled
    }

    /// Whether the field closed by submitting, as opposed to being open or cancelled.
    #[must_use]
    pub fn was_submitted(&self) -> bool {
        self.phase == Phase::Submitted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_LEN;

    const ENTER: u16 = 0x0d;
    const ESCAPE: u16 = 0x1b;
    const BACKSPACE: u16 = 0x08;
    const PASTE: u16 = 0x16;
    const COPY: u16 = 0x03;

    fn type_str(editor: &mut Editor, text: &str) {
        for unit in text.encode_utf16() {
            assert_eq!(editor.on_char(unit), Event::Editing);
        }
    }

    #[test]
    fn it_opens_on_the_prefix_with_the_caret_at_the_end() {
        let editor = Editor::new();
        assert_eq!(editor.text(), BUILD_URL_PREFIX);
        assert_eq!(editor.caret(), BUILD_URL_PREFIX.chars().count());
        assert!(editor.is_open());
    }

    /// Ported: a prefilled field whose caret sat at the start made the first key prepend.
    #[test]
    fn typing_appends_to_the_prefill() {
        let mut editor = Editor::new();
        type_str(&mut editor, "253");
        assert_eq!(editor.text(), format!("{BUILD_URL_PREFIX}253"));
    }

    #[test]
    fn enter_submits_the_text_once() {
        let mut editor = Editor::new();
        type_str(&mut editor, "253");
        assert_eq!(
            editor.on_char(ENTER),
            Event::Submitted(format!("{BUILD_URL_PREFIX}253"))
        );
        assert!(!editor.is_open());
        assert!(editor.was_submitted());
        assert_eq!(editor.on_char(ENTER), Event::Closed);
        assert_eq!(editor.enter(), Event::Closed);
    }

    /// Ported: backing out applies nothing.
    #[test]
    fn escape_cancels_and_yields_nothing() {
        let mut editor = Editor::new();
        type_str(&mut editor, "253");
        assert_eq!(editor.on_char(ESCAPE), Event::Cancelled);
        assert!(!editor.is_open());
        assert!(!editor.was_submitted());
    }

    #[test]
    fn nothing_changes_the_text_after_the_field_closes() {
        let mut editor = Editor::new();
        editor.escape();
        let frozen = editor.text();
        assert_eq!(editor.on_char(u16::from(b'9')), Event::Closed);
        assert_eq!(editor.insert_char('9'), Event::Closed);
        assert_eq!(editor.backspace(), Event::Closed);
        assert_eq!(editor.paste("253"), Event::Closed);
        assert_eq!(editor.on_char(ENTER), Event::Closed);
        assert_eq!(editor.text(), frozen);
    }

    #[test]
    fn backspace_removes_the_last_character_and_stops_at_the_start() {
        let mut editor = Editor::with_text("ab");
        assert_eq!(editor.on_char(BACKSPACE), Event::Editing);
        assert_eq!(editor.text(), "a");
        editor.backspace();
        editor.backspace();
        assert_eq!(editor.text(), "");
        assert_eq!(editor.caret(), 0);
    }

    #[test]
    fn ctrl_v_asks_for_the_clipboard_and_paste_inserts_it_cleaned() {
        let mut editor = Editor::new();
        assert_eq!(editor.on_char(PASTE), Event::PasteRequested);
        assert_eq!(
            editor.text(),
            BUILD_URL_PREFIX,
            "the request alone changes nothing"
        );
        editor.paste(" 253\r\n");
        assert_eq!(editor.text(), format!("{BUILD_URL_PREFIX}253"));
        assert_eq!(
            editor.enter(),
            Event::Submitted("https://soulsplanner.com/darksouls2/253".to_owned())
        );
    }

    #[test]
    fn a_whole_link_pasted_over_a_cleared_field_submits_as_is() {
        let mut editor = Editor::with_text("");
        editor.paste("https://soulsplanner.com/darksouls2/253\n");
        assert_eq!(
            editor.enter(),
            Event::Submitted("https://soulsplanner.com/darksouls2/253".to_owned())
        );
    }

    #[test]
    fn a_paste_stops_at_the_length_bound() {
        let mut editor = Editor::with_text("");
        editor.paste(&"7".repeat(MAX_LEN * 3));
        assert_eq!(editor.text().chars().count(), MAX_LEN);
    }

    #[test]
    fn typing_stops_at_the_length_bound() {
        let mut editor = Editor::with_text(&"1".repeat(MAX_LEN));
        editor.insert_char('2');
        assert_eq!(editor.text().chars().count(), MAX_LEN);
        assert!(!editor.text().contains('2'));
    }

    #[test]
    fn copy_carries_the_text() {
        let mut editor = Editor::with_text("abc");
        assert_eq!(editor.on_char(COPY), Event::Copy("abc".to_owned()));
        assert_eq!(editor.text(), "abc");
    }

    /// The underlying field empties on a cut before the caller could read it back, so the event
    /// has to carry what was cut.
    #[test]
    fn cut_carries_the_text_it_removed() {
        let mut editor = Editor::with_text("abc");
        assert_eq!(editor.on_char(CUT), Event::Copy("abc".to_owned()));
        assert_eq!(editor.text(), "");
        assert!(editor.is_open());
    }

    #[test]
    fn insert_char_takes_printable_characters_and_ignores_control_ones() {
        let mut editor = Editor::with_text("");
        for character in ['2', '5', '3'] {
            assert_eq!(editor.insert_char(character), Event::Editing);
        }
        assert_eq!(editor.insert_char('\r'), Event::Editing);
        assert!(editor.is_open(), "a stray carriage return must not submit");
        assert_eq!(editor.text(), "253");
    }

    #[test]
    fn a_character_outside_the_basic_plane_is_one_character() {
        let mut editor = Editor::with_text("");
        editor.insert_char('\u{1f525}');
        assert_eq!(editor.text(), "\u{1f525}");
        assert_eq!(editor.caret(), 1);
        editor.backspace();
        assert_eq!(editor.text(), "");
    }

    #[test]
    fn stray_control_codes_are_swallowed_without_effect() {
        let mut editor = Editor::with_text("x");
        for unit in [0x00, 0x07, 0x09, 0x7f] {
            assert_eq!(editor.on_char(unit), Event::Editing);
        }
        assert_eq!(editor.text(), "x");
        assert!(editor.is_open());
    }

    #[test]
    fn the_caret_is_drawn_where_the_next_character_goes() {
        let editor = Editor::with_text("ab");
        assert_eq!(editor.text_with_caret('|'), "ab|");
    }
}
