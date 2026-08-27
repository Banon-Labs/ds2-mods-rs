//! `key = value` under `[section]` headers -- the SYNTAX half of a config file, with no opinion
//! whatsoever about what any key means.
//!
//! # Why this exists instead of a TOML dependency
//!
//! The config files in this repo are a dozen scalars. A TOML crate brings a parser for arrays,
//! inline tables, datetimes, multi-line strings and dotted keys so that four booleans can be
//! read, and it brings it into a DLL that is otherwise dependency-free by design -- see this
//! crate's `Cargo.toml`. What is actually needed is the four decisions below: split on the first
//! `=`, remember the last `[section]`, ignore blanks and `#` comments, and report everything
//! else.
//!
//! What being a strict *subset* of TOML does buy: the files this reads are valid TOML. Anyone who
//! reaches for a real parser later, or an editor with TOML syntax highlighting, gets the same
//! answer this does. Nothing here accepts something TOML would reject.
//!
//! # Syntax, in full
//!
//! ```text
//! # a comment. Blank lines are fine too.
//! [section]                 # everything below belongs to `section`, until the next header
//! key = value               # value is the trimmed rest of the line
//! quoted = "with spaces"    # one matched pair of double quotes is removed
//! number = 1000             # unquoted values may carry a trailing ` # comment`
//! ```
//!
//! Keys before the first header live in the section named `""`.
//!
//! # A line that is not one of those is REPORTED, never skipped
//!
//! This is the whole reason [`Rejected`] exists. A config reader that silently ignores what it
//! cannot understand turns `enbaled = true` into a feature that is simply off, and leaves the
//! person who typed it with nothing to look at. Every unusable line comes back with its number
//! and its text so the caller can put it in a log, which is the same discipline
//! [`crate::binding::BindingUpdate::Rejected`] applies to values it cannot parse.
//!
//! A duplicate key is a rejection too, and the FIRST occurrence is the one that stands. TOML
//! itself refuses a duplicate key outright; taking one of the two silently would mean an edit to
//! the wrong copy of a line does nothing and says nothing.
//!
//! # What this module does NOT do
//!
//! It does not know a schema. It hands back strings; whether `enabled` is a boolean, which
//! section it belongs in, what a missing one defaults to and what a malformed one costs are all
//! the caller's, because those are the parts that differ per DLL. See
//! `crates/ds2-loader/src/arxan_probe.rs` for a caller that spells all four out.

/// Why a line could not be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    /// `[section` with no closing bracket. Guessing where the name ended would be a guess.
    UnclosedSection,
    /// `= value` with nothing to the left of the `=`.
    EmptyKey,
    /// Not blank, not a comment, not a header, and carrying no `=` at all.
    NotAssignment,
    /// This section already had this key. The first one stands.
    DuplicateKey,
}

impl RejectReason {
    /// The phrase a log line uses. Written to be read as the tail of `... -- <phrase>`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnclosedSection => "section header is missing its closing `]`",
            Self::EmptyKey => "no key to the left of the `=`",
            Self::NotAssignment => {
                "expected `key = value`, `[section]`, `# comment` or a blank line"
            }
            Self::DuplicateKey => "this key was already set; the FIRST value is the one in force",
        }
    }
}

/// A line that could not be used, kept verbatim so a log can quote it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejected {
    /// 1-based line number, which is what an editor shows.
    pub line: usize,
    /// The line as written, trimmed. Quoting it back is how a typo becomes findable.
    pub text: String,
    /// Why it could not be used.
    pub reason: RejectReason,
}

/// One `key = value` that a file actually carried.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    section: String,
    key: String,
    value: String,
}

/// The usable content of a `key = value` file, plus everything in it that was not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyValues {
    entries: Vec<Entry>,
    rejected: Vec<Rejected>,
}

impl KeyValues {
    /// Read `text`. Never fails: a file of pure nonsense parses to no entries and one
    /// [`Rejected`] per line, which is a far more useful thing to log than one error about the
    /// first of them.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut parsed = Self::default();
        let mut section = String::new();

        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix('[') {
                match rest.strip_suffix(']') {
                    Some(name) => section = name.trim().to_owned(),
                    None => parsed.reject(line, trimmed, RejectReason::UnclosedSection),
                }
                continue;
            }
            let Some((key, value)) = trimmed.split_once('=') else {
                parsed.reject(line, trimmed, RejectReason::NotAssignment);
                continue;
            };
            let key = key.trim();
            if key.is_empty() {
                parsed.reject(line, trimmed, RejectReason::EmptyKey);
                continue;
            }
            if parsed.get(&section, key).is_some() {
                parsed.reject(line, trimmed, RejectReason::DuplicateKey);
                continue;
            }
            parsed.entries.push(Entry {
                section: section.clone(),
                key: key.to_owned(),
                value: clean_value(value),
            });
        }
        parsed
    }

    fn reject(&mut self, line: usize, text: &str, reason: RejectReason) {
        self.rejected.push(Rejected {
            line,
            text: text.to_owned(),
            reason,
        });
    }

    /// The value of `key` under `[section]`, or `None` if the file does not mention it.
    ///
    /// An absent key is not an error here and must not be treated as one by the caller either: a
    /// config that leaves a setting at its default is a correct config.
    #[must_use]
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|entry| entry.section == section && entry.key == key)
            .map(|entry| entry.value.as_str())
    }

    /// Every line that could not be used. Log all of them -- see the module docs.
    #[must_use]
    pub fn rejected(&self) -> &[Rejected] {
        &self.rejected
    }

    /// The keys this file set under `[section]`, in the order they appeared.
    ///
    /// For the caller that knows its whole schema and wants to say so: a MISSPELLED key parses
    /// perfectly -- `enbaled = true` is a valid assignment to a key called `enbaled` -- so it can
    /// never appear in [`Self::rejected`]. Only the caller knows that no such key exists. Without
    /// this, the sole evidence of the typo is the real key reading as absent, which tells someone
    /// what did not happen but not why.
    pub fn keys<'a>(&'a self, section: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.entries
            .iter()
            .filter(move |entry| entry.section == section)
            .map(|entry| entry.key.as_str())
    }

    /// How many `key = value` pairs were read. Zero, next to a file that exists and is not empty,
    /// is itself a finding.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the file carried no usable pairs at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The text to the right of the `=`, as the caller should see it.
///
/// One matched pair of double quotes comes off, so `key = "F7"` and `key = F7` mean the same
/// thing -- the quoted spelling is the one TOML wants for a string and the one this crate's own
/// key names are written in. An UNQUOTED value may carry a trailing ` # comment`, which is where
/// TOML puts one; a quoted value may not, because a `#` inside quotes is part of the string and
/// cutting there would silently truncate it.
fn clean_value(value: &str) -> String {
    let trimmed = value.trim();
    if let Some(inner) = trimmed
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    {
        return inner.to_owned();
    }
    match trimmed.split_once(" #") {
        Some((before, _)) => before.trim_end().to_owned(),
        None => trimmed.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole feature in one assertion.
    #[test]
    fn a_key_under_a_section_is_read() {
        let parsed = KeyValues::parse("[arxan_probe]\nenabled = true\n");
        assert_eq!(parsed.get("arxan_probe", "enabled"), Some("true"));
        assert_eq!(parsed.len(), 1);
        assert!(parsed.rejected().is_empty());
    }

    /// Sections separate keys that share a name. Without this, two mods in one file collide the
    /// way the two Elden Ring DLLs in this crate's own origin story collided.
    #[test]
    fn the_same_key_in_two_sections_is_two_keys() {
        let parsed = KeyValues::parse("[a]\nenabled = true\n[b]\nenabled = false\n");
        assert_eq!(parsed.get("a", "enabled"), Some("true"));
        assert_eq!(parsed.get("b", "enabled"), Some("false"));
        assert_eq!(parsed.get("c", "enabled"), None);
    }

    /// A key nobody asked about, and a section nobody asked about, are simply absent. This is the
    /// case where a future mod's section shares the file with this one.
    #[test]
    fn an_unmentioned_key_is_absent_not_an_error() {
        let parsed = KeyValues::parse("[arxan_probe]\nenabled = true\n[some_future_mod]\nx = 1\n");
        assert_eq!(parsed.get("arxan_probe", "skip_neuter"), None);
        assert!(
            parsed.rejected().is_empty(),
            "another mod's section is not an error"
        );
    }

    /// Keys before any header are readable rather than lost, under the empty section name.
    #[test]
    fn a_key_before_the_first_header_lives_in_the_empty_section() {
        let parsed = KeyValues::parse("loose = 1\n[s]\ntight = 2\n");
        assert_eq!(parsed.get("", "loose"), Some("1"));
        assert_eq!(parsed.get("s", "tight"), Some("2"));
    }

    /// Comments and blank lines are not content and are not rejections either.
    #[test]
    fn comments_and_blank_lines_are_ignored_silently() {
        let text = "# written by scripts/ds2-run.py\n\n  \n[arxan_probe]\n\n# on purpose\nenabled = true\n";
        let parsed = KeyValues::parse(text);
        assert_eq!(parsed.get("arxan_probe", "enabled"), Some("true"));
        assert!(parsed.rejected().is_empty());
    }

    /// Whitespace around every part of a line is noise, including a file written on Windows.
    #[test]
    fn whitespace_and_crlf_are_trimmed_off() {
        let parsed = KeyValues::parse("  [ arxan_probe ]  \r\n   enabled   =   true   \r\n");
        assert_eq!(parsed.get("arxan_probe", "enabled"), Some("true"));
    }

    /// A quoted value loses exactly its quotes -- the spelling TOML wants for a string, and the
    /// one this crate's key names are written in.
    #[test]
    fn one_matched_pair_of_quotes_comes_off() {
        let parsed = KeyValues::parse("key = \"F7\"\nbare = F7\nspaced = \"Left Shift\"\n");
        assert_eq!(parsed.get("", "key"), Some("F7"));
        assert_eq!(parsed.get("", "bare"), Some("F7"));
        assert_eq!(parsed.get("", "spaced"), Some("Left Shift"));
    }

    /// A trailing comment on an unquoted value is TOML's shape and must not become part of the
    /// value -- `true # on` failing to parse as a boolean is the difference between working and
    /// silently off.
    #[test]
    fn a_trailing_comment_on_an_unquoted_value_is_not_part_of_it() {
        let parsed = KeyValues::parse("enabled = true # arm A\nms = 1000  # a second\n");
        assert_eq!(parsed.get("", "enabled"), Some("true"));
        assert_eq!(parsed.get("", "ms"), Some("1000"));
    }

    /// ...and inside quotes a `#` is content. Cutting there would truncate a string silently.
    #[test]
    fn a_hash_inside_quotes_is_content() {
        let parsed = KeyValues::parse("label = \"arm #2\"\n");
        assert_eq!(parsed.get("", "label"), Some("arm #2"));
    }

    /// A value with an `=` in it keeps it: the split is on the FIRST `=` only.
    #[test]
    fn only_the_first_equals_splits_the_line() {
        let parsed = KeyValues::parse("expr = a=b\n");
        assert_eq!(parsed.get("", "expr"), Some("a=b"));
    }

    /// An empty value is a value. Whether that is usable is the caller's rule, not this one's --
    /// and the caller can only apply its rule if the empty string reaches it.
    #[test]
    fn an_empty_value_is_handed_back_empty_rather_than_dropped() {
        let parsed = KeyValues::parse("enabled =\n");
        assert_eq!(parsed.get("", "enabled"), Some(""));
        assert!(parsed.rejected().is_empty());
    }

    /// THE RULE THAT MATTERS. A typo is not silently skipped; it comes back with its line number
    /// and its text, because a reader that ignores what it cannot understand leaves the person
    /// who typed it with nothing to look at.
    #[test]
    fn an_unusable_line_is_reported_with_its_number_and_its_text() {
        let parsed = KeyValues::parse("[arxan_probe]\nenabled = true\nenbaled true\n");
        assert_eq!(parsed.get("arxan_probe", "enabled"), Some("true"));
        assert_eq!(
            parsed.rejected(),
            [Rejected {
                line: 3,
                text: "enbaled true".to_owned(),
                reason: RejectReason::NotAssignment,
            }]
        );
    }

    /// A rejected line does not stop the ones after it. A config with one typo in it is still a
    /// config, and dropping the rest would turn one typo into every setting reverting.
    #[test]
    fn a_rejected_line_does_not_discard_the_rest_of_the_file() {
        let parsed = KeyValues::parse("[s]\nbroken\nenabled = true\nskip = false\n");
        assert_eq!(parsed.get("s", "enabled"), Some("true"));
        assert_eq!(parsed.get("s", "skip"), Some("false"));
        assert_eq!(parsed.rejected().len(), 1);
    }

    /// A duplicate takes the FIRST value and says so. Silently taking one of the two means an
    /// edit to the wrong copy of a line does nothing and reports nothing.
    #[test]
    fn a_duplicate_key_keeps_the_first_and_is_reported() {
        let parsed = KeyValues::parse("[s]\nenabled = true\nenabled = false\n");
        assert_eq!(parsed.get("s", "enabled"), Some("true"));
        assert_eq!(parsed.rejected().len(), 1);
        assert_eq!(parsed.rejected()[0].reason, RejectReason::DuplicateKey);
        assert_eq!(parsed.rejected()[0].line, 3);
    }

    /// The same key in a section reopened later is still a duplicate of the same key.
    #[test]
    fn a_reopened_section_cannot_smuggle_a_duplicate_past() {
        let parsed = KeyValues::parse("[s]\nenabled = true\n[t]\nx = 1\n[s]\nenabled = false\n");
        assert_eq!(parsed.get("s", "enabled"), Some("true"));
        assert_eq!(parsed.rejected()[0].reason, RejectReason::DuplicateKey);
    }

    #[test]
    fn a_header_missing_its_bracket_is_reported_rather_than_guessed_at() {
        let parsed = KeyValues::parse("[arxan_probe\nenabled = true\n");
        assert_eq!(parsed.rejected()[0].reason, RejectReason::UnclosedSection);
        // The section never opened, so the key landed where it actually is rather than where the
        // author meant. Reporting the header is what lets them see why.
        assert_eq!(parsed.get("", "enabled"), Some("true"));
        assert_eq!(parsed.get("arxan_probe", "enabled"), None);
    }

    #[test]
    fn an_assignment_with_no_key_is_reported() {
        let parsed = KeyValues::parse("  = true\n");
        assert_eq!(parsed.rejected()[0].reason, RejectReason::EmptyKey);
        assert!(parsed.is_empty());
    }

    /// An empty file is empty, not broken. This is the shape a caller must NOT confuse with a
    /// missing file -- see the loader, which logs those two differently on purpose.
    #[test]
    fn an_empty_file_parses_to_nothing_and_complains_about_nothing() {
        let parsed = KeyValues::parse("");
        assert!(parsed.is_empty());
        assert!(parsed.rejected().is_empty());
        assert_eq!(parsed.get("arxan_probe", "enabled"), None);
    }

    /// A misspelled key parses perfectly and can never be a rejection, so the caller needs to be
    /// able to see what the file actually set in order to name it.
    #[test]
    fn the_keys_of_a_section_can_be_listed_so_a_typo_can_be_named() {
        let parsed = KeyValues::parse("[s]\nenbaled = true\nskip = false\n[other]\nx = 1\n");
        assert!(parsed.rejected().is_empty(), "a typo is a VALID assignment");
        assert_eq!(parsed.keys("s").collect::<Vec<_>>(), ["enbaled", "skip"]);
        assert_eq!(parsed.keys("other").collect::<Vec<_>>(), ["x"]);
        assert_eq!(parsed.keys("nope").count(), 0);
    }

    /// Every reason has a phrase, and it reads as the tail of `... -- <phrase>`.
    #[test]
    fn every_reject_reason_has_a_phrase_a_log_can_print() {
        for reason in [
            RejectReason::UnclosedSection,
            RejectReason::EmptyKey,
            RejectReason::NotAssignment,
            RejectReason::DuplicateKey,
        ] {
            assert!(!reason.as_str().is_empty());
        }
    }
}
