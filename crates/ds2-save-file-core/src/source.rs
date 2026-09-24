//! What a Load-from-File dialog is allowed to come back with.
//!
//! The dialog's filter is a suggestion -- a player can type a name, or switch the dropdown to every
//! file -- so the pick is checked again here before anything is staged. This is the cheap gate: it
//! only reads the extension, and it exists so the log line for a wrong pick says *"that is not a
//! save"* instead of surfacing a decompression error from three layers down.
//!
//! # Why four extensions and not one
//!
//! `ds2-save-redirect`'s staging step already unwraps `.zip`, `.7z` and `.rar` looking for exactly
//! one `DS2SOFS0000.sl2` inside, because that is the shape a save is in when somebody sends you
//! one. Refusing archives here would refuse the common case in order to be tidy.
//!
//! # The duplication, and what makes it safe
//!
//! This list is `ds2-save-redirect::stage`'s four arms, spelled a second time. That crate is
//! `cfg(windows)` in full, so a host-tested crate cannot import from it -- and a host-tested list is
//! worth more than a shared one, because the list is what a player's pick is judged against.
//! `ds2-save-file`'s Windows build carries the test that the two agree, so a new arm there fails a
//! build rather than silently going unoffered.

use std::path::Path;

/// Every file shape the staging step behind the Load-from-File row can read, without dots.
///
/// `sl2` first, deliberately: it is the pattern the dialog's default dropdown line shows, and a
/// player who has a bare save should not have to find it behind "Archive".
pub const SOURCE_EXTENSIONS: [&str; 4] = ["sl2", "zip", "7z", "rar"];

/// Why a picked path was not staged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceRejection {
    /// The dialog returned nothing, or a name with no characters in it.
    Empty,
    /// A name with no extension at all. Told apart from the arm below so the log can say which,
    /// because a player who typed a folder name and a player who picked a screenshot need
    /// different sentences.
    NoExtension,
    /// An extension none of [`SOURCE_EXTENSIONS`] names. Carries what was seen, lowercased.
    UnknownExtension(String),
}

impl core::fmt::Display for SourceRejection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(f, "no file was named"),
            Self::NoExtension => write!(
                f,
                "that name has no extension; expected one of {}",
                offered()
            ),
            Self::UnknownExtension(seen) => {
                write!(f, "{seen:?} is not a save; expected one of {}", offered())
            }
        }
    }
}

/// The extension list as a player-facing sentence fragment: `.sl2, .zip, .7z or .rar`.
pub fn offered() -> String {
    let mut parts: Vec<String> = SOURCE_EXTENSIONS
        .iter()
        .map(|extension| format!(".{extension}"))
        .collect();
    let last = parts.pop().unwrap_or_default();
    format!("{} or {last}", parts.join(", "))
}

/// The canonical extension of a path the load dialog returned, or why it is not staged.
///
/// Case is folded, because a file named `SAVE.SL2` is a save. The returned `&'static str` is the
/// entry from [`SOURCE_EXTENSIONS`] rather than the player's spelling, so a caller that switches on
/// it has the same four arms the staging step has.
///
/// **The leaf and its dot are found by hand rather than through [`Path`].** The paths this judges
/// are Windows paths from comdlg32 and its tests run on Linux, where `Path` reads
/// `C:\v1.0\donor` as one filename and reports its extension as `0\donor`. A gate that answers
/// differently under test than in the game is not a gate.
pub fn accepts(path: &Path) -> Result<&'static str, SourceRejection> {
    let text = path.to_string_lossy();
    let leaf = match text.rfind(['\\', '/']) {
        Some(index) => &text[index + 1..],
        None => text.as_ref(),
    };
    if leaf.trim().is_empty() {
        return Err(SourceRejection::Empty);
    }
    // A leading dot is a hidden file, not an extension -- `.sl2` alone names nothing.
    let Some(dot) = leaf.rfind('.').filter(|dot| *dot > 0) else {
        return Err(SourceRejection::NoExtension);
    };
    let lowered = leaf[dot + 1..].to_ascii_lowercase();
    SOURCE_EXTENSIONS
        .iter()
        .find(|known| **known == lowered)
        .copied()
        .ok_or(SourceRejection::UnknownExtension(lowered))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_save_is_accepted() {
        assert_eq!(accepts(Path::new(r"C:\saves\DS2SOFS0000.sl2")), Ok("sl2"));
    }

    /// The three archive shapes a downloaded save arrives in.
    #[test]
    fn every_archive_shape_is_accepted() {
        for (name, expected) in [
            ("donor.zip", "zip"),
            ("donor.7z", "7z"),
            ("donor.rar", "rar"),
        ] {
            assert_eq!(accepts(Path::new(name)), Ok(expected), "{name}");
        }
    }

    /// A save someone renamed in shouting case is still a save.
    #[test]
    fn case_is_folded_to_the_canonical_spelling() {
        assert_eq!(accepts(Path::new("SAVE.SL2")), Ok("sl2"));
        assert_eq!(accepts(Path::new("SAVE.Zip")), Ok("zip"));
    }

    /// A dot in a DIRECTORY is not the file's extension, whichever separator the path uses.
    #[test]
    fn a_dot_above_the_leaf_is_not_an_extension() {
        assert_eq!(
            accepts(Path::new(r"C:\v1.0\donor")),
            Err(SourceRejection::NoExtension)
        );
        assert_eq!(accepts(Path::new(r"C:\v1.0\donor.sl2")), Ok("sl2"));
    }

    /// A dotfile names no extension; `.sl2` alone is a hidden file called `sl2`.
    #[test]
    fn a_leading_dot_is_not_an_extension() {
        assert_eq!(
            accepts(Path::new(r"C:\saves\.sl2")),
            Err(SourceRejection::NoExtension)
        );
    }

    /// The two refusals are told apart, because they need different sentences.
    #[test]
    fn the_refusals_are_distinguishable() {
        assert_eq!(
            accepts(Path::new("screenshot.png")),
            Err(SourceRejection::UnknownExtension("png".to_owned()))
        );
        assert_eq!(
            accepts(Path::new("DS2SOFS0000")),
            Err(SourceRejection::NoExtension)
        );
        assert_eq!(accepts(Path::new("")), Err(SourceRejection::Empty));
    }

    /// Every extension offered to the player appears in the refusal text, so a rejected pick says
    /// what would have worked.
    #[test]
    fn the_refusal_names_what_would_have_worked() {
        let text = SourceRejection::UnknownExtension("png".to_owned()).to_string();
        for extension in SOURCE_EXTENSIONS {
            assert!(text.contains(extension), "{text} omits {extension}");
        }
    }
}
