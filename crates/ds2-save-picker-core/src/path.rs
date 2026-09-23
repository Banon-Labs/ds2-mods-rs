//! Leaf and parent, found by hand rather than through [`Path`].
//!
//! # Why this exists at all
//!
//! The paths this picker moves around are WINDOWS paths -- `Z:\home\banon\saves`,
//! `S:\steamapps\common\...` -- and every test in this repo runs on LINUX, where [`Path`] has no
//! idea that `\` separates anything. `Path::file_name` on `C:\v1.0\donor.sl2` answers with the
//! whole string, and `Path::extension` on it answers `0\donor`. A model that decided anything
//! from those answers would behave one way under test and another way in the game, which is the
//! same defect `ds2-save-file-core`'s `accepts` already refuses to have: a gate that answers
//! differently under test than in the game is not a gate.
//!
//! So both separators are handled explicitly, and a drive prefix is recognised by its shape
//! (`X:` followed by a separator) rather than by asking the host what an absolute path looks
//! like. POSIX paths keep working because the same scan finds `/` just as well -- these
//! functions are correct on the machine the tests run on AND on the machine the game runs on,
//! which is the entire point.

use std::path::{Path, PathBuf};

/// Both separators, always. Wine accepts either in a Windows path and players paste both.
const SEPARATORS: [char; 2] = ['\\', '/'];

/// True when `text` opens with a drive prefix: a letter, a colon, then a separator.
///
/// Checked on BYTES so the indexing below stays inside character boundaries -- the prefix is
/// ASCII by construction, whatever the rest of the path turns out to be.
fn has_drive_prefix(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

/// The last component of `path`, or `None` when it names no component.
///
/// `None` for a drive root (`Z:\` is a place, not a file), for the POSIX root, and for a path
/// that does not round-trip through UTF-8 -- a path nothing downstream could name anyway.
pub fn leaf(path: &Path) -> Option<&str> {
    let text = path.to_str()?;
    let trimmed = text.trim_end_matches(SEPARATORS);
    if has_drive_prefix(text) && trimmed.len() <= 2 {
        return None;
    }
    match trimmed.rfind(SEPARATORS) {
        Some(index) => trimmed.get(index + 1..).filter(|name| !name.is_empty()),
        None => (!trimmed.is_empty()).then_some(trimmed),
    }
}

/// The directory `path` lives in, or `None` when there is nowhere further up.
///
/// A drive root reports `None` rather than reporting itself: the picker's `[..]` row exists only
/// while there is somewhere to go, and a row that navigates to where you already are is a row
/// that looks broken. The drive root keeps its trailing separator (`Z:\`, not `Z:`) because
/// `Z:` alone is a different thing to Windows -- the current directory on drive Z -- and the
/// picker would be handing that to `read_dir`.
pub fn parent(path: &Path) -> Option<PathBuf> {
    let text = path.to_str()?;
    let trimmed = text.trim_end_matches(SEPARATORS);
    if has_drive_prefix(text) {
        if trimmed.len() <= 2 {
            return None;
        }
        let index = trimmed.rfind(SEPARATORS)?;
        // At or before the colon means the parent IS the drive root, which keeps its separator.
        return Some(if index <= 2 {
            PathBuf::from(&text[..3])
        } else {
            PathBuf::from(&trimmed[..index])
        });
    }
    let index = trimmed.rfind(SEPARATORS)?;
    if index == 0 {
        // A child of the POSIX root. The root is `/`; the empty string is not a directory.
        return (trimmed.len() > 1).then(|| PathBuf::from("/"));
    }
    Some(PathBuf::from(&trimmed[..index]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole reason this module is not `Path::file_name`, stated as an assertion.
    ///
    /// THE STANDARD LIBRARY'S ANSWER DEPENDS ON WHO COMPILED IT, which is exactly the trap this
    /// module exists to step around. `std::path` is target-flavoured: the same `Path::file_name`
    /// call splits on backslashes when built for Windows and does not when built for the host. The
    /// repo gate runs BOTH -- `cargo test` on the host and the MSVC test binary under wine -- so an
    /// unconditional claim about `file_name` is true in one pass and false in the other, and the
    /// wine pass is where it was caught. `leaf` is asserted for both because the point of the
    /// module is that it does not care.
    #[test]
    fn a_windows_leaf_is_found_on_a_host_that_has_never_heard_of_backslashes() {
        let save = Path::new(r"Z:\home\banon\saves\DS2SOFS0000.sl2");
        assert_eq!(leaf(save), Some("DS2SOFS0000.sl2"));
        let from_std = save.file_name().and_then(|name| name.to_str());
        #[cfg(not(windows))]
        assert_ne!(
            from_std,
            Some("DS2SOFS0000.sl2"),
            "if this ever passes, the host learned Windows paths and this test is the wrong shape"
        );
        #[cfg(windows)]
        assert_eq!(
            from_std,
            Some("DS2SOFS0000.sl2"),
            "built for Windows, so std agrees -- and `leaf` must agree with it, not diverge"
        );
    }

    /// A dot in a FOLDER is not the file's extension, and the leaf is where that gets decided.
    #[test]
    fn a_dot_above_the_leaf_stays_above_the_leaf() {
        assert_eq!(leaf(Path::new(r"C:\v1.0\donor.sl2")), Some("donor.sl2"));
        assert_eq!(leaf(Path::new(r"C:\v1.0\donor")), Some("donor"));
    }

    /// Forward slashes in a drive-prefixed path are separators too; Wine and players both use them.
    #[test]
    fn either_separator_works_in_a_drive_prefixed_path() {
        assert_eq!(leaf(Path::new("C:/saves/donor.sl2")), Some("donor.sl2"));
        assert_eq!(
            parent(Path::new("C:/saves/donor.sl2")),
            Some("C:/saves".into())
        );
    }

    #[test]
    fn a_drive_root_names_nothing_and_goes_nowhere() {
        assert_eq!(leaf(Path::new(r"Z:\")), None);
        assert_eq!(parent(Path::new(r"Z:\")), None);
        assert_eq!(leaf(Path::new("/")), None);
        assert_eq!(parent(Path::new("/")), None);
    }

    /// The step from a first-level folder lands on the root WITH its separator, not on `Z:`.
    #[test]
    fn the_last_step_up_keeps_the_drive_separator() {
        assert_eq!(parent(Path::new(r"Z:\home")), Some(PathBuf::from(r"Z:\")));
        assert_eq!(parent(Path::new(r"Z:\home\")), Some(PathBuf::from(r"Z:\")));
        assert_eq!(parent(Path::new("/home")), Some(PathBuf::from("/")));
    }

    #[test]
    fn walking_up_a_windows_path_ends_at_the_drive_root() {
        let mut at = PathBuf::from(r"Z:\home\banon\saves");
        let mut seen = vec![at.clone()];
        while let Some(up) = parent(&at) {
            at = up;
            seen.push(at.clone());
        }
        assert_eq!(
            seen,
            vec![
                PathBuf::from(r"Z:\home\banon\saves"),
                PathBuf::from(r"Z:\home\banon"),
                PathBuf::from(r"Z:\home"),
                PathBuf::from(r"Z:\"),
            ]
        );
    }

    /// A trailing separator is punctuation, not a component.
    #[test]
    fn a_trailing_separator_does_not_become_an_empty_leaf() {
        assert_eq!(leaf(Path::new(r"Z:\home\banon\")), Some("banon"));
        assert_eq!(leaf(Path::new("/home/banon/")), Some("banon"));
    }

    /// `1:\x` is not a drive: the prefix needs a LETTER, or every `12:30\note` becomes a root.
    #[test]
    fn a_digit_before_the_colon_is_not_a_drive() {
        assert_eq!(parent(Path::new(r"1:\x")), Some(PathBuf::from("1:")));
    }
}
