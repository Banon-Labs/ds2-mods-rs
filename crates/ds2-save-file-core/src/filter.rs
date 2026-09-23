//! The one string in a common file dialog that cannot be built by concatenation.
//!
//! `OPENFILENAMEW::lpstrFilter` is not a C string. It is a run of NUL-separated UTF-16 strings
//! terminated by an EMPTY one -- so the buffer ends in two NUL units, and a `&str` converted the
//! ordinary way ends in one. A filter with a single terminator makes comdlg32 walk off the end of
//! the buffer looking for the next pair, which is why this is its own module with its own tests
//! rather than a `format!` at the call site.
//!
//! Pairs alternate: a label the player reads, then the patterns that label selects.
//!
//! ```text
//! "DARK SOULS II save (*.sl2)\0*.sl2\0Archive (*.zip;*.7z)\0*.zip;*.7z\0\0"
//! ```

/// One line in the dialog's file-type dropdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilterEntry<'a> {
    /// What the player reads. The pattern list is appended in parentheses, so pass the noun alone
    /// (`"DARK SOULS II save"`) and not a label that already carries one.
    pub label: &'a str,
    /// Extensions WITHOUT their dots. Empty means every file, rendered as `*.*` -- which is a
    /// deliberate escape hatch rather than an accident: a save someone renamed is still a save, and
    /// a player who knows that should be able to reach it.
    pub extensions: &'a [&'a str],
}

impl FilterEntry<'_> {
    /// The pattern half of this entry: `*.sl2;*.zip`, or `*.*` for an entry naming no extension.
    pub fn patterns(&self) -> String {
        if self.extensions.is_empty() {
            return String::from("*.*");
        }
        self.extensions
            .iter()
            .map(|extension| format!("*.{extension}"))
            .collect::<Vec<_>>()
            .join(";")
    }

    /// The label half, with the patterns in parentheses the way every Windows dialog spells it.
    pub fn text(&self) -> String {
        format!("{} ({})", self.label, self.patterns())
    }
}

/// A NUL-terminated UTF-16 buffer, for the dialog fields that really are plain C strings.
pub fn wide_nul(text: &str) -> Vec<u16> {
    let mut units: Vec<u16> = text.encode_utf16().collect();
    units.push(0);
    units
}

/// The whole `lpstrFilter` buffer, terminated by the empty string comdlg32 looks for.
///
/// An entry whose label or patterns contain an interior NUL would end the run early and silently
/// hide every entry after it, so those units are DROPPED rather than passed through. Nothing in
/// this repo can produce one; the filter is still built from `&str`, and `&str` can hold `\0`.
pub fn filter_string(entries: &[FilterEntry<'_>]) -> Vec<u16> {
    let mut units = Vec::new();
    for entry in entries {
        push_field(&mut units, &entry.text());
        push_field(&mut units, &entry.patterns());
    }
    // The terminator, and the reason this function exists: an EMPTY string after the last pair.
    units.push(0);
    units
}

fn push_field(units: &mut Vec<u16>, text: &str) {
    units.extend(text.encode_utf16().filter(|unit| *unit != 0));
    units.push(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(units: &[u16]) -> Vec<String> {
        units
            .split(|unit| *unit == 0)
            .map(String::from_utf16_lossy)
            .collect()
    }

    /// The shape the dialog reads: label, patterns, then an empty string.
    #[test]
    fn a_single_entry_ends_in_an_empty_string() {
        let filter = filter_string(&[FilterEntry {
            label: "DARK SOULS II save",
            extensions: &["sl2"],
        }]);
        assert_eq!(
            decode(&filter),
            vec![
                "DARK SOULS II save (*.sl2)".to_owned(),
                "*.sl2".to_owned(),
                String::new(),
                String::new(),
            ]
        );
        // Two NUL units at the end, not one. This is the whole bug this module prevents.
        assert_eq!(filter[filter.len() - 2..], [0, 0]);
    }

    /// Several extensions share one line, separated the way comdlg32 wants them.
    #[test]
    fn extensions_are_joined_with_semicolons() {
        let entry = FilterEntry {
            label: "Archive",
            extensions: &["zip", "7z", "rar"],
        };
        assert_eq!(entry.patterns(), "*.zip;*.7z;*.rar");
        assert_eq!(entry.text(), "Archive (*.zip;*.7z;*.rar)");
    }

    /// No extensions means every file, and says so in the label too.
    #[test]
    fn no_extensions_is_every_file() {
        let entry = FilterEntry {
            label: "All files",
            extensions: &[],
        };
        assert_eq!(entry.patterns(), "*.*");
        assert_eq!(entry.text(), "All files (*.*)");
    }

    /// Two entries produce four fields and one terminator, in order.
    #[test]
    fn entries_keep_their_order() {
        let filter = filter_string(&[
            FilterEntry {
                label: "Save",
                extensions: &["sl2"],
            },
            FilterEntry {
                label: "All files",
                extensions: &[],
            },
        ]);
        let fields = decode(&filter);
        assert_eq!(fields[0], "Save (*.sl2)");
        assert_eq!(fields[1], "*.sl2");
        assert_eq!(fields[2], "All files (*.*)");
        assert_eq!(fields[3], "*.*");
        assert_eq!(fields[4], "");
    }

    /// An interior NUL cannot be allowed to end the run early.
    #[test]
    fn an_interior_nul_is_dropped_rather_than_passed_through() {
        let filter = filter_string(&[
            FilterEntry {
                label: "Sa\0ve",
                extensions: &["sl2"],
            },
            FilterEntry {
                label: "Second",
                extensions: &["zip"],
            },
        ]);
        let fields = decode(&filter);
        assert_eq!(fields[0], "Save (*.sl2)");
        // The second entry is still reachable, which is what the filter would have lost.
        assert_eq!(fields[2], "Second (*.zip)");
    }

    /// `wide_nul` is for the ordinary C-string fields, and carries exactly one terminator.
    #[test]
    fn wide_nul_terminates_once() {
        let units = wide_nul("ab");
        assert_eq!(units, vec![b'a' as u16, b'b' as u16, 0]);
    }
}
