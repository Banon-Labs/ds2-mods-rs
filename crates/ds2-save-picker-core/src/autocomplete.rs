//! What the path field offers while a player types into it, and what a typed path resolves to.
//!
//! Ported from `er-save-picker-core`'s `autocomplete`, with one difference that matters on this
//! host: the Linux-shaped path translation is a PARAMETER here rather than a `cfg(windows)` block.
//! This crate has no Windows code, and a translation that only exists in the build the tests never
//! run is a translation nothing tests. The model carries the drive a leading `/` means (Wine maps
//! `/` to `Z:`) and passes it down; a test that browses real host directories turns it off.
//!
//! # Why the suggestion keeps what was typed
//!
//! A match is found case-insensitively, so `Z:\h` can match a folder really named `Home`. The
//! suggestion is still `Z:\h` + `ome`, not `Z:\Home`: the ghost is drawn behind the live text, so
//! the characters the player typed have to be the characters underneath them or the two runs show
//! as doubled glyphs. Windows resolves paths without regard to case, so the completed text opens
//! the same folder either way.
//!
//! # Folders only
//!
//! The field chooses a folder to browse. Offering a file would complete to something the commit
//! refuses as not a folder, so files are skipped while listing.

use std::path::{Path, PathBuf};

use crate::path::has_drive_prefix;

/// Separators a typed path may use. Players paste both, and Wine accepts both.
const SEPARATORS: [char; 2] = ['\\', '/'];

/// Split typed text into the part naming a folder to list and the partial name being typed.
///
/// The parent keeps its trailing separator, because `Z:\` and `Z:` mean different things and only
/// the first is a folder. Text with no separator has no parent to list and returns `None` rather
/// than guessing at a relative folder.
pub fn split_typed_path(typed: &str) -> Option<(&str, &str)> {
    let index = typed.rfind(SEPARATORS)?;
    Some(typed.split_at(index + 1))
}

/// A typed path in the form the filesystem is asked about.
///
/// With `posix_drive` set, a Linux-shaped absolute path (`/home/banon`) becomes the drive Wine maps
/// `/` to (`Z:\home\banon`): only the root and the separators change, case and spaces survive
/// byte for byte. A bare drive (`Z:`) gains its separator, because `Z:` alone is the current folder
/// ON drive Z to Windows, which is not what a player typing a drive letter means. Anything else
/// passes through untouched.
pub fn resolve_typed_path(typed: &str, posix_drive: Option<char>) -> PathBuf {
    if let Some(drive) = posix_drive
        && typed.starts_with('/')
    {
        return PathBuf::from(format!("{drive}:{}", typed.replace('/', "\\")));
    }
    let bytes = typed.as_bytes();
    if bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return PathBuf::from(format!("{typed}\\"));
    }
    PathBuf::from(typed)
}

/// Whether typed text names a place without reference to some current folder.
///
/// Decided on the TEXT, for the same reason `crate::path` exists: `Path::is_absolute` on a Linux
/// test host says `Z:\home` is relative. A drive prefix, a bare drive, a leading `/` (a POSIX root,
/// or Wine's `Z:` once translated) and a UNC `\\server` all count.
pub fn typed_path_is_absolute(typed: &str) -> bool {
    let bytes = typed.as_bytes();
    has_drive_prefix(typed)
        || (bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        || typed.starts_with('/')
        || typed.starts_with("\\\\")
}

/// The folder a parent fragment names, in the same terms [`resolve_typed_path`] resolves.
fn parent_directory(parent: &str, posix_drive: Option<char>) -> PathBuf {
    let resolved = resolve_typed_path(parent, posix_drive);
    let text = resolved.to_string_lossy();
    // A drive root keeps its separator (`Z:\`); anything longer is trimmed so the platform does
    // not treat the trailing separator as an empty final component.
    let trimmed = text.trim_end_matches(SEPARATORS);
    if trimmed.len() <= 2 {
        resolved.clone()
    } else {
        PathBuf::from(trimmed)
    }
}

/// Complete `typed` from `names`, the child folder names of its parent.
///
/// `None` when there is nothing to offer: no separator, nothing typed after it, no
/// case-insensitive prefix match, or a match that adds no characters -- a suggestion equal to the
/// typed text would draw a ghost the player cannot tell from their own input.
pub fn suggestion_from_names<'a>(
    typed: &str,
    names: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let (parent, partial) = split_typed_path(typed)?;
    if partial.is_empty() {
        return None;
    }
    let partial_folded = partial.to_lowercase();
    let best = names
        .into_iter()
        .filter(|name| name.to_lowercase().starts_with(&partial_folded))
        // Alphabetical without regard to case, with the raw name breaking ties so the choice is
        // stable when two folders differ only in case.
        .min_by(|left, right| {
            left.to_lowercase()
                .cmp(&right.to_lowercase())
                .then_with(|| left.cmp(right))
        })?;
    let remainder: String = best.chars().skip(partial.chars().count()).collect();
    if remainder.is_empty() {
        return None;
    }
    Some(format!("{parent}{partial}{remainder}"))
}

/// Complete `typed` by listing its parent folder on this machine.
///
/// An unreadable or absent parent has nothing to offer, which is also what a player half-way
/// through typing a path should see -- so no error is reported.
pub fn suggestion_for(typed: &str, posix_drive: Option<char>) -> Option<String> {
    let (parent, partial) = split_typed_path(typed)?;
    if partial.is_empty() {
        return None;
    }
    let names = directory_child_names(&parent_directory(parent, posix_drive));
    suggestion_from_names(typed, names.iter().map(String::as_str))
}

/// Child folder names of `dir`, unsorted, dot-prefixed ones hidden like the listing hides them.
///
/// The kind comes from stat'ing the target, never from the dirent: under Wine the symlinked and
/// subvolume folders at `Z:\` come back as something other than a folder, and dropping them is how
/// `Z:\h` once offered nothing while `home/` sat one row below it in the listing.
fn directory_child_names(dir: &Path) -> Vec<String> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    read.flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            (!name.starts_with('.') && entry.path().is_dir()).then_some(name)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partial_name_completes_to_the_first_match_without_regard_to_case() {
        assert_eq!(
            suggestion_from_names(r"Z:\h", ["Windows", "Home", "help", "Games"]),
            Some(r"Z:\help".to_owned())
        );
    }

    /// `Home` matched, and the offer keeps the lowercase `h` the player actually typed.
    #[test]
    fn the_typed_characters_survive_so_the_ghost_can_sit_under_them() {
        assert_eq!(
            suggestion_from_names(r"Z:\h", ["Home"]),
            Some(r"Z:\home".to_owned())
        );
    }

    #[test]
    fn nothing_typed_after_the_separator_offers_nothing() {
        assert_eq!(suggestion_from_names(r"Z:\", ["b", "a"]), None);
    }

    #[test]
    fn an_exact_name_offers_nothing_because_the_ghost_would_be_invisible() {
        assert_eq!(suggestion_from_names(r"Z:\Home", ["Home"]), None);
    }

    #[test]
    fn no_prefix_match_offers_nothing_and_no_separator_has_no_parent() {
        assert_eq!(suggestion_from_names(r"Z:\q", ["Home"]), None);
        assert_eq!(split_typed_path("home"), None);
    }

    #[test]
    fn a_forward_slash_path_splits_on_its_own_separator() {
        assert_eq!(
            suggestion_from_names("/home/ba", ["banon", "backup"]),
            Some("/home/backup".to_owned())
        );
    }

    /// The Linux shape a player pastes becomes the drive Wine maps `/` to, and nothing else moves.
    #[test]
    fn a_linux_path_becomes_the_wine_drive_only_when_asked() {
        assert_eq!(
            resolve_typed_path("/home/banon/My Saves", Some('Z')),
            PathBuf::from(r"Z:\home\banon\My Saves")
        );
        assert_eq!(
            resolve_typed_path("/home/banon", None),
            PathBuf::from("/home/banon"),
            "a host test browsing real folders turns the translation off"
        );
        assert_eq!(
            resolve_typed_path(r"C:\Games", Some('Z')),
            PathBuf::from(r"C:\Games")
        );
    }

    #[test]
    fn a_bare_drive_letter_means_the_drive_root() {
        assert_eq!(resolve_typed_path("Z:", None), PathBuf::from(r"Z:\"));
        assert!(typed_path_is_absolute("Z:"));
    }

    #[test]
    fn absolute_is_decided_from_the_text() {
        for absolute in [r"Z:\home", "C:/x", "/home", r"\\server\share"] {
            assert!(typed_path_is_absolute(absolute), "{absolute}");
        }
        for relative in ["home", r"saves\x", "", "1:\\x"] {
            assert!(!typed_path_is_absolute(relative), "{relative}");
        }
    }

    #[test]
    fn the_parent_keeps_the_separator_that_makes_a_drive_root_a_folder() {
        assert_eq!(split_typed_path(r"Z:\h"), Some((r"Z:\", "h")));
        assert_eq!(parent_directory(r"Z:\", None), PathBuf::from(r"Z:\"));
        assert_eq!(
            parent_directory(r"Z:\Games\", None),
            PathBuf::from(r"Z:\Games")
        );
        assert_eq!(
            parent_directory("/home/", Some('Z')),
            PathBuf::from(r"Z:\home")
        );
    }

    #[test]
    fn a_real_folder_is_completed_from_the_filesystem_and_files_are_skipped() {
        let root = crate::picker_scratch_dir("autocomplete-listing");
        std::fs::create_dir_all(root.join("Harvest")).expect("scratch dir must be creatable");
        std::fs::create_dir_all(root.join("hangar")).expect("scratch dir must be creatable");
        std::fs::create_dir_all(root.join(".hidden")).expect("scratch dir must be creatable");
        std::fs::write(root.join("hfile.txt"), b"x").expect("scratch file must be writable");
        std::fs::write(root.join("ha.sl2"), b"x").expect("scratch file must be writable");

        let typed = format!("{}{}h", root.display(), std::path::MAIN_SEPARATOR);
        assert_eq!(
            suggestion_for(&typed, None),
            Some(format!(
                "{}{}hangar",
                root.display(),
                std::path::MAIN_SEPARATOR
            ))
        );
        let dotted = format!("{}{}.h", root.display(), std::path::MAIN_SEPARATOR);
        assert_eq!(
            suggestion_for(&dotted, None),
            None,
            "hidden folders are not offered"
        );
    }
}
