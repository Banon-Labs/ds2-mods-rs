//! The text field's editing rules, with no window, no font and no keyboard behind them.
//!
//! # Why this is a state machine over `u16` and not over key codes
//!
//! DARK SOULS II runs its own `TranslateMessage`, so what reaches the game is `WM_CHAR` -- a
//! UTF-16 code unit that the keyboard layout, the shift state and any dead key have ALREADY been
//! applied to. That is the whole gift: this module never asks what key was pressed, only what
//! character came out, so a non-US layout, an accented letter and a shifted digit all arrive
//! correct without one line of layout handling here.
//!
//! The control codes come through the same door and are what the caller acts on:
//!
//! | unit | key | [`Reaction`] |
//! |---|---|---|
//! | `0x08` | Backspace | handled here |
//! | `0x0d` | Enter | [`Reaction::Confirm`] |
//! | `0x1b` | Escape | [`Reaction::Cancel`] |
//! | `0x16` | Ctrl+V | [`Reaction::PasteRequested`] |
//! | `0x03` | Ctrl+C | [`Reaction::CopyRequested`] |
//! | `0x18` | Ctrl+X | [`Reaction::CopyRequested`], then the field clears |
//!
//! Ctrl+V arriving as an ordinary `WM_CHAR` (`SYN`, `0x16`) is why paste needs no chord matching,
//! no modifier tracking and no second hook. Elden Ring's equivalent field has NO PASTE AT ALL and
//! fakes one by polling the clipboard sequence number every frame -- see
//! `er-quit-menu-core/src/build_url_clipboard.rs`. This engine simply tells us.
//!
//! # The caret starts at the END
//!
//! A prefilled field whose caret sits at zero makes the first keystroke PREPEND, which reads as
//! the field having ignored the prefill. Elden Ring hit this and spends its first eight frames
//! pushing the caret back to the end through the engine. Owning the buffer means it is one
//! assignment in [`Field::new`] instead.

/// The longest text the field will hold.
///
/// A soulsplanner link is 40 characters. This is not a limit anyone will reach by typing -- it is
/// a bound on what a PASTE can do, so a clipboard holding a document cannot become a string this
/// process has to draw.
pub const MAX_UNITS: usize = 256;

/// What the caller must do after handing the field a character.
///
/// The clipboard variants exist because the clipboard is Win32 and this crate is not. The field
/// says what it needs; the runtime half fetches it and calls [`Field::paste`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reaction {
    /// The field consumed it. Nothing else to do.
    Handled,
    /// Enter. Take [`Field::text`] and act on it.
    Confirm,
    /// Escape. Throw the field away without acting.
    Cancel,
    /// Ctrl+V. Read the clipboard, then call [`Field::paste`].
    PasteRequested,
    /// Ctrl+C or Ctrl+X. Put [`Field::text`] on the clipboard.
    CopyRequested,
    /// Not a character this field uses -- a tab, a bell, a stray control code.
    ///
    /// Still SWALLOWED rather than passed on: the field is modal, and a key the field ignores must
    /// not reach the game underneath it. The distinction from [`Self::Handled`] is for the log.
    Ignored,
}

/// An editable line of text and a caret in it.
///
/// The buffer is `Vec<char>` rather than `String` because every operation here is positional --
/// insert at the caret, delete before it, step over one character -- and a caret that indexes a
/// `String` is a byte offset that can land inside a character. Rendering converts once, at the
/// edge.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Field {
    units: Vec<char>,
    caret: usize,
    /// A high surrogate waiting for its partner. See [`Field::on_char`].
    pending_surrogate: Option<u16>,
}

impl Field {
    /// A field holding `prefill`, with the caret after it.
    ///
    /// Text past [`MAX_UNITS`] is truncated rather than refused -- a caller's prefill is not the
    /// player's mistake, and a field that silently opened empty would be worse than a short one.
    pub fn new(prefill: &str) -> Self {
        let units: Vec<char> = prefill.chars().take(MAX_UNITS).collect();
        let caret = units.len();
        Self {
            units,
            caret,
            pending_surrogate: None,
        }
    }

    /// The text as it stands.
    pub fn text(&self) -> String {
        self.units.iter().collect()
    }

    /// How many characters are in the field.
    pub fn len(&self) -> usize {
        self.units.len()
    }

    /// Whether the field is empty.
    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    /// Where the caret is, as a character index in `0..=len()`.
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// Feed the field one `WM_CHAR` code unit.
    ///
    /// Surrogates are PAIRED HERE rather than dropped. A character outside the basic plane arrives
    /// as two messages, and treating each as a character on its own would put two replacement
    /// marks in the buffer. No soulsplanner link contains one -- but a paste-adjacent field that
    /// mangles emoji is a field that mangles whatever else the player had in the clipboard, and
    /// the fix is eight lines.
    pub fn on_char(&mut self, unit: u16) -> Reaction {
        // `take()` runs either way, so an unpaired high surrogate is DROPPED here and this unit is
        // then judged on its own merits rather than inheriting the orphan's fate.
        if let Some(high) = self.pending_surrogate.take()
            && (0xdc00..0xe000).contains(&unit)
        {
            // A complete pair. `char::decode_utf16` is the only decoder that will not guess.
            let decoded = char::decode_utf16([high, unit]).next().and_then(Result::ok);
            return match decoded {
                Some(character) => self.insert(character),
                None => Reaction::Ignored,
            };
        }
        if (0xd800..0xdc00).contains(&unit) {
            self.pending_surrogate = Some(unit);
            return Reaction::Handled;
        }

        match unit {
            0x08 => {
                self.backspace();
                Reaction::Handled
            }
            0x0d => Reaction::Confirm,
            0x1b => Reaction::Cancel,
            0x16 => Reaction::PasteRequested,
            0x03 => Reaction::CopyRequested,
            0x18 => {
                // Cut is copy, then clear. The caller still has to do the copy half.
                self.units.clear();
                self.caret = 0;
                Reaction::CopyRequested
            }
            // Everything below space is a control code the field has no use for, and 0x7f is DEL.
            0x00..=0x1f | 0x7f => Reaction::Ignored,
            _ => match char::decode_utf16([unit]).next().and_then(Result::ok) {
                Some(character) => self.insert(character),
                None => Reaction::Ignored,
            },
        }
    }

    /// Insert text at the caret, as a paste does.
    ///
    /// Newlines and tabs become nothing rather than becoming spaces: a link copied out of a chat
    /// window arrives with a trailing newline, and a space is the one character that would make it
    /// fail validation for a reason the player cannot see.
    pub fn paste(&mut self, text: &str) {
        for character in text.chars() {
            if character.is_control() {
                continue;
            }
            if self.insert(character) == Reaction::Ignored {
                break;
            }
        }
    }

    /// Delete the character before the caret.
    pub fn backspace(&mut self) {
        if self.caret > 0 {
            self.caret -= 1;
            self.units.remove(self.caret);
        }
    }

    /// Delete the character after the caret.
    pub fn delete(&mut self) {
        if self.caret < self.units.len() {
            self.units.remove(self.caret);
        }
    }

    /// Step the caret one character left.
    pub fn move_left(&mut self) {
        self.caret = self.caret.saturating_sub(1);
    }

    /// Step the caret one character right.
    pub fn move_right(&mut self) {
        self.caret = (self.caret + 1).min(self.units.len());
    }

    /// Put the caret before the first character.
    pub fn move_home(&mut self) {
        self.caret = 0;
    }

    /// Put the caret after the last character.
    pub fn move_end(&mut self) {
        self.caret = self.units.len();
    }

    /// Replace everything, caret to the end. What a rejected link comes back as.
    pub fn set_text(&mut self, text: &str) {
        self.units = text.chars().take(MAX_UNITS).collect();
        self.caret = self.units.len();
        self.pending_surrogate = None;
    }

    /// The text with a caret mark in it, for drawing.
    ///
    /// The field is drawn by handing a string to `FeElement::setText`, which has no notion of a
    /// caret -- so the caret has to BE a character. `mark` is what to draw it as.
    pub fn text_with_caret(&self, mark: char) -> String {
        let mut out = String::with_capacity(self.units.len() + 1);
        out.extend(&self.units[..self.caret]);
        out.push(mark);
        out.extend(&self.units[self.caret..]);
        out
    }

    /// Insert one character at the caret, refusing past [`MAX_UNITS`].
    fn insert(&mut self, character: char) -> Reaction {
        if self.units.len() >= MAX_UNITS {
            return Reaction::Ignored;
        }
        self.units.insert(self.caret, character);
        self.caret += 1;
        Reaction::Handled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The prefill is there and the caret is AFTER it, so the first keystroke appends.
    ///
    /// The whole point. A caret left at zero makes typing prepend, which reads as the prefill
    /// having been ignored -- the bug Elden Ring's field spends eight frames working around.
    #[test]
    fn typing_after_a_prefill_appends_rather_than_prepends() {
        let mut field = Field::new(crate::BUILD_URL_PREFIX);
        assert_eq!(field.caret(), field.len());
        for unit in "253".encode_utf16() {
            assert_eq!(field.on_char(unit), Reaction::Handled);
        }
        assert_eq!(field.text(), format!("{}253", crate::BUILD_URL_PREFIX));
        assert_eq!(crate::build_id_from_url(&field.text()), Ok(253));
    }

    /// Each control code reaches the caller as the thing it means.
    #[test]
    fn the_control_codes_say_what_the_player_pressed() {
        let mut field = Field::new("x");
        assert_eq!(field.on_char(0x0d), Reaction::Confirm);
        assert_eq!(field.on_char(0x1b), Reaction::Cancel);
        assert_eq!(field.on_char(0x16), Reaction::PasteRequested);
        assert_eq!(field.on_char(0x03), Reaction::CopyRequested);
        // None of those edited anything.
        assert_eq!(field.text(), "x");
        // Tab and bell are swallowed, not passed to the game underneath.
        assert_eq!(field.on_char(0x09), Reaction::Ignored);
        assert_eq!(field.on_char(0x07), Reaction::Ignored);
        assert_eq!(field.text(), "x");
    }

    /// Cut copies and clears in one press.
    #[test]
    fn cut_asks_for_the_copy_and_empties_the_field() {
        let mut field = Field::new("gone");
        assert_eq!(field.on_char(0x18), Reaction::CopyRequested);
        assert!(field.is_empty());
        assert_eq!(field.caret(), 0);
    }

    /// Backspace deletes before the caret and stops at the start rather than panicking.
    #[test]
    fn backspace_stops_at_the_start() {
        let mut field = Field::new("ab");
        field.backspace();
        assert_eq!(field.text(), "a");
        field.backspace();
        assert!(field.is_empty());
        field.backspace();
        assert!(field.is_empty());
    }

    /// The caret moves within bounds and edits land where it is.
    #[test]
    fn an_edit_lands_where_the_caret_is() {
        let mut field = Field::new("ac");
        field.move_left();
        assert_eq!(field.caret(), 1);
        field.on_char(u16::from(b'b'));
        assert_eq!(field.text(), "abc");
        field.move_home();
        field.move_left();
        assert_eq!(field.caret(), 0);
        field.delete();
        assert_eq!(field.text(), "bc");
        field.move_end();
        field.move_right();
        assert_eq!(field.caret(), field.len());
        field.delete();
        assert_eq!(field.text(), "bc");
    }

    /// A paste lands at the caret and drops the newline a copied link arrives with.
    ///
    /// A trailing newline turned into a space is the one corruption the player cannot see, and it
    /// fails validation for a reason that looks like the site's fault.
    #[test]
    fn a_pasted_link_loses_its_control_characters() {
        let mut field = Field::new("");
        field.paste("https://soulsplanner.com/darksouls2/253\n");
        assert_eq!(field.text(), "https://soulsplanner.com/darksouls2/253");
        assert_eq!(crate::build_id_from_url(&field.text()), Ok(253));
    }

    /// A paste cannot make the field longer than it will draw.
    #[test]
    fn a_document_on_the_clipboard_cannot_become_the_field() {
        let mut field = Field::new("");
        field.paste(&"x".repeat(MAX_UNITS * 4));
        assert_eq!(field.len(), MAX_UNITS);
        // And typing past the ceiling is refused rather than silently wrapping.
        assert_eq!(field.on_char(u16::from(b'y')), Reaction::Ignored);
        assert_eq!(field.len(), MAX_UNITS);
    }

    /// A character outside the basic plane survives its two messages.
    #[test]
    fn a_surrogate_pair_arrives_as_one_character() {
        let mut field = Field::new("");
        let pair: Vec<u16> = "🔥".encode_utf16().collect();
        assert_eq!(pair.len(), 2, "the fixture must actually be a pair");
        assert_eq!(field.on_char(pair[0]), Reaction::Handled);
        assert_eq!(field.on_char(pair[1]), Reaction::Handled);
        assert_eq!(field.text(), "🔥");
        assert_eq!(field.len(), 1, "one character, not two halves");
    }

    /// An unpaired high surrogate does not poison the character after it.
    #[test]
    fn an_orphaned_surrogate_does_not_eat_the_next_character() {
        let mut field = Field::new("");
        field.on_char(0xd83d);
        field.on_char(u16::from(b'a'));
        assert_eq!(field.text(), "a");
    }

    /// The drawn string carries the caret as a character, because `setText` has no other notion.
    #[test]
    fn the_caret_is_drawn_as_a_character() {
        let mut field = Field::new("ab");
        assert_eq!(field.text_with_caret('|'), "ab|");
        field.move_home();
        assert_eq!(field.text_with_caret('|'), "|ab");
        field.move_right();
        assert_eq!(field.text_with_caret('|'), "a|b");
    }

    /// Re-opening after a refusal keeps the text and puts the caret where editing resumes.
    #[test]
    fn a_refused_link_comes_back_editable_at_its_end() {
        let mut field = Field::new(crate::BUILD_URL_PREFIX);
        field.paste("abc");
        assert_eq!(
            crate::build_id_from_url(&field.text()),
            Err(crate::UrlRejection::IdNotNumeric)
        );
        field.set_text(&field.text());
        assert_eq!(field.caret(), field.len());
        field.backspace();
        field.backspace();
        field.backspace();
        field.paste("253");
        assert_eq!(crate::build_id_from_url(&field.text()), Ok(253));
    }
}
