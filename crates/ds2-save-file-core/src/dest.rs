//! Where a Save-to-File row is allowed to write, and the one decision that can lose a save.
//!
//! This is the only module in either crate that can destroy something a player owns, so it is the
//! only one written as a set of refusals. Three rules, each of which exists because the obvious
//! implementation gets it wrong:
//!
//! 1. **A name with no extension gets one.** The bytes written are a `.sl2` container; a copy under
//!    no extension is a copy nothing opens. comdlg32 can do this itself through `lpstrDefExt`, and
//!    it is done here anyway -- the flag's behaviour depends on which of eight `OFN_*` bits are set,
//!    and the destination path is wanted for a log line before the write either way.
//! 2. **An existing file is a different operation from a new one.** [`Route`] names them apart so
//!    the caller cannot reach a write without having decided which one it is doing.
//! 3. **The live save container is not a destination.** Copying a file onto itself is a truncation
//!    on most platforms and a no-op on the rest, and neither is what "export my save" means. See
//!    [`is_live_container`].

use std::path::{Path, PathBuf};

/// What writing to a picked destination amounts to.
///
/// Two variants and no `Refused`: whether an overwrite is ALLOWED is the caller's policy, and this
/// type exists so that policy has something to switch on. Collapsing the pair into a `bool` is how
/// an overwrite becomes indistinguishable from a create at the call site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// Nothing is there. The write creates the file and can lose nothing.
    Create,
    /// Something is already there, and writing replaces it.
    Overwrite,
}

impl Route {
    /// Which operation a destination is, given whether it exists.
    ///
    /// Takes the answer rather than the question so this stays testable with no filesystem: the
    /// caller does the `is_file`, which is the part that can only be true at one instant anyway.
    pub const fn of(exists: bool) -> Self {
        if exists {
            Self::Overwrite
        } else {
            Self::Create
        }
    }

    /// Whether this route destroys something.
    pub const fn destroys(self) -> bool {
        matches!(self, Self::Overwrite)
    }
}

/// The picked path with `extension` appended if it has none at all.
///
/// **An extension the player typed is kept, whatever it is.** A save named `backup.old` is a
/// deliberate choice and the bytes are the same container either way; silently renaming it to
/// `backup.old.sl2` would put the file somewhere the player did not ask for and then not mention it.
/// Only a name with NO extension is completed, because that one is not a choice.
///
/// **The separator is spelled out rather than taken from [`Path`], and that is load-bearing.** This
/// code runs inside a Wine prefix on paths comdlg32 built, so the separator is always `\` -- but its
/// tests run on Linux, where `Path` does not treat `\` as one and reads `C:\out\donor` as a single
/// filename. Asking `Path` would make the function behave differently in the test from the way it
/// behaves in the game, which is the one property a test has to have.
pub fn with_extension(picked: &Path, extension: &str) -> PathBuf {
    let text = picked.to_string_lossy().into_owned();
    let leaf_at = text
        .rfind(['\\', '/'])
        .map(|index| index + 1)
        .unwrap_or_default();
    let leaf = &text[leaf_at..];
    if leaf.trim().is_empty() || leaf.contains('.') {
        return picked.to_path_buf();
    }
    PathBuf::from(format!("{text}.{extension}"))
}

/// Whether a destination names the container the game is currently reading and writing.
///
/// **Compared case-insensitively, and that is not thoroughness -- it is correctness.** This runs
/// inside a Wine prefix against a Windows path the game built, and Windows paths are not
/// case-sensitive, so `C:\...\DS2SOFS0000.sl2` and `c:\...\ds2sofs0000.sl2` are one file. A
/// case-sensitive compare would answer "different file" and the copy would truncate the save the
/// player is playing.
///
/// It is still only a comparison of two strings. A destination reached through a junction, a
/// symlink, a mapped drive or `..` is the same file under a different name and this will say they
/// differ -- so the caller canonicalises both sides first where the platform can, and this is the
/// check that survives when it cannot.
pub fn is_live_container(destination: &Path, live: &Path) -> bool {
    let normalise = |path: &Path| {
        path.to_string_lossy()
            .to_lowercase()
            .replace('/', "\\")
            // A trailing separator cannot appear on a file, and a doubled one is the same path.
            .replace("\\\\", "\\")
    };
    normalise(destination) == normalise(live)
}

/// Appended to a destination to name the file an export is written into before it becomes the
/// destination.
///
/// Not ending in `.sl2` is deliberate: the dialog's save filter does not list it, so a staging file
/// left behind by a crash is not offered back to the player as a save.
pub const STAGING_SUFFIX: &str = ".ds2-export";

/// Where an export is written before it replaces `destination`: the same folder, so the final
/// rename cannot cross a volume, under a name that is not a save.
///
/// # Why an export needs a staging file at all
///
/// The game's save is a read-modify-write of the whole container. `SLBindOperation`
/// (`FUN_140a91650`) rebuilds all twenty-three entries on every save, and an in-game save supplies
/// only four of them: the character list, its mirror, and the loaded slot's two entries. Every other
/// entry is read back out of the file being saved to (`FUN_140a9302b` mounts it), and when that mount
/// fails the entry is written as its length word followed by zeros. Measured 2026-09-26: an export to
/// a name that did not exist logged six diverted read opens answering `handle=-1`, then the write,
/// and the file it produced had every slot but the loaded one zero-filled while the character list
/// still named them.
///
/// So the file the game is pointed at must already hold the live container when the save starts.
/// Seeding the destination itself would do that, but it would destroy an existing destination before
/// the game had written anything, and an export that then never lands would leave the player with
/// neither their old file nor the new one. A staging file is seeded instead, the game writes that,
/// and it replaces the destination only once the write has been observed.
pub fn staging_path(destination: &Path) -> PathBuf {
    let mut text = destination.as_os_str().to_os_string();
    text.push(STAGING_SUFFIX);
    PathBuf::from(text)
}

/// How an export ended, decided from what is on disk rather than from what was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportVerdict {
    /// The game wrote the file and it is now at the destination, which changed.
    Written,
    /// The game never wrote the file it was pointed at before the deadline. Nothing at the
    /// destination is new.
    NeverWritten,
    /// The game wrote the staging file, and it could not be moved over the destination.
    NotPromoted,
    /// Every step reported success, and the destination is nevertheless not different from how it
    /// was before the press. A success line here would be a claim about a write that did not
    /// happen, so this is a failure of its own.
    Unchanged,
}

/// Judge an export. `stamp` is whatever the caller uses to tell two states of a file apart --
/// length and modified time in the game -- and `None` means the file could not be read.
///
/// * `flushed` -- the file the game was pointed at changed after it was seeded.
/// * `promoted` -- `None` when there was no staging file (the destination is the live container,
///   so the game was pointed straight at it); otherwise whether the rename onto the destination
///   succeeded.
/// * `before` / `after` -- the destination as it was when the row was pressed, and as it is now.
///
/// Success needs all three, and the last is not redundant. On 2026-09-26 the row logged an export
/// with a byte count, and a note that the flush was never observed, for a destination whose mtime
/// never moved: the line reported the size of a file that was already there. What the player is
/// promised is that the destination now holds this save, so that is what is checked last.
pub fn export_verdict<S: PartialEq>(
    flushed: bool,
    promoted: Option<bool>,
    before: Option<&S>,
    after: Option<&S>,
) -> ExportVerdict {
    if !flushed {
        return ExportVerdict::NeverWritten;
    }
    if promoted == Some(false) {
        return ExportVerdict::NotPromoted;
    }
    match after {
        Some(after) if before != Some(after) => ExportVerdict::Written,
        _ => ExportVerdict::Unchanged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The staging file sits beside the destination, keeps its whole name, and is not a save.
    #[test]
    fn the_staging_file_is_beside_the_destination_and_not_a_save() {
        let staged = staging_path(Path::new(r"Z:\saves\mule.sl2"));
        assert_eq!(staged, PathBuf::from(r"Z:\saves\mule.sl2.ds2-export"));
        assert!(
            !staged.to_string_lossy().ends_with(".sl2"),
            "the dialog's filter would offer it"
        );
        assert!(!is_live_container(&staged, Path::new(r"Z:\saves\mule.sl2")));
    }

    /// A save the game never wrote is never reported as an export, whatever the destination holds.
    #[test]
    fn an_unobserved_write_is_never_a_success() {
        let old = (8_251_680u64, 1u64);
        // The 2026-09-26 row: the destination existed, nothing moved, and the line said "exported".
        assert_eq!(
            export_verdict(false, Some(true), Some(&old), Some(&old)),
            ExportVerdict::NeverWritten
        );
        assert_eq!(
            export_verdict::<(u64, u64)>(false, None, None, None),
            ExportVerdict::NeverWritten
        );
    }

    /// A written staging file that could not replace the destination is not an export either.
    #[test]
    fn a_failed_rename_is_reported_as_such() {
        assert_eq!(
            export_verdict(true, Some(false), None, Some(&(1u64, 2u64))),
            ExportVerdict::NotPromoted
        );
    }

    /// The destination has to be different from how it was at the press, and has to exist.
    #[test]
    fn success_requires_the_destination_to_have_changed() {
        let before = (8_251_680u64, 1u64);
        let after = (8_251_680u64, 2u64);
        assert_eq!(
            export_verdict(true, Some(true), Some(&before), Some(&after)),
            ExportVerdict::Written
        );
        // Created: nothing was there before.
        assert_eq!(
            export_verdict(true, Some(true), None, Some(&after)),
            ExportVerdict::Written
        );
        // Written straight into the live container, no staging file.
        assert_eq!(
            export_verdict(true, None, Some(&before), Some(&after)),
            ExportVerdict::Written
        );
        assert_eq!(
            export_verdict(true, Some(true), Some(&before), Some(&before)),
            ExportVerdict::Unchanged
        );
        assert_eq!(
            export_verdict(true, Some(true), Some(&before), None),
            ExportVerdict::Unchanged
        );
    }

    /// The route is named by what is there, and only the overwrite destroys.
    #[test]
    fn an_existing_file_is_an_overwrite() {
        assert_eq!(Route::of(true), Route::Overwrite);
        assert_eq!(Route::of(false), Route::Create);
        assert!(Route::of(true).destroys());
        assert!(!Route::of(false).destroys());
    }

    /// A bare name is completed; a name that already carries any extension is left alone.
    #[test]
    fn only_a_bare_name_gains_an_extension() {
        assert_eq!(
            with_extension(Path::new(r"C:\out\donor"), "sl2"),
            PathBuf::from(r"C:\out\donor.sl2")
        );
        assert_eq!(
            with_extension(Path::new(r"C:\out\donor.sl2"), "sl2"),
            PathBuf::from(r"C:\out\donor.sl2")
        );
        // The player's own choice of extension survives, rather than gaining a second one.
        assert_eq!(
            with_extension(Path::new(r"C:\out\donor.old"), "sl2"),
            PathBuf::from(r"C:\out\donor.old")
        );
    }

    /// Nothing to complete stays untouched rather than becoming a name made of one dot.
    ///
    /// Both separators, because the function must not consult the host's idea of which one counts:
    /// these tests run on Linux and the game runs on `\`.
    #[test]
    fn an_empty_name_is_not_completed() {
        assert_eq!(
            with_extension(Path::new(r"C:\out\"), "sl2"),
            PathBuf::from(r"C:\out\")
        );
        assert_eq!(
            with_extension(Path::new("/out/"), "sl2"),
            PathBuf::from("/out/")
        );
    }

    /// A dot in a DIRECTORY name is not the file's extension, and completing the leaf anyway would
    /// write `C:\v1.0\donor` -- a file with no extension -- while claiming it had one.
    #[test]
    fn a_dot_in_a_directory_does_not_count_as_the_leafs_extension() {
        assert_eq!(
            with_extension(Path::new(r"C:\v1.0\donor"), "sl2"),
            PathBuf::from(r"C:\v1.0\donor.sl2")
        );
    }

    /// The whole reason this is not `==`: a Windows path in a Wine prefix.
    #[test]
    fn the_live_container_is_matched_regardless_of_case() {
        let live = Path::new(r"C:\Users\p\AppData\Roaming\DarkSoulsII\011\DS2SOFS0000.sl2");
        assert!(is_live_container(
            Path::new(r"c:\users\p\appdata\roaming\darksoulsii\011\ds2sofs0000.sl2"),
            live
        ));
        assert!(is_live_container(live, live));
    }

    /// A separator written the other way round is the same file inside a prefix.
    #[test]
    fn slashes_and_backslashes_are_the_same_separator() {
        assert!(is_live_container(
            Path::new("C:/saves/DS2SOFS0000.sl2"),
            Path::new(r"C:\saves\DS2SOFS0000.sl2")
        ));
    }

    /// And a genuinely different file is still different -- the check has to be able to say no.
    #[test]
    fn a_different_file_is_not_the_live_container() {
        assert!(!is_live_container(
            Path::new(r"D:\backups\DS2SOFS0000.sl2"),
            Path::new(r"C:\saves\DS2SOFS0000.sl2")
        ));
        assert!(!is_live_container(
            Path::new(r"C:\saves\DS2SOFS0001.sl2"),
            Path::new(r"C:\saves\DS2SOFS0000.sl2")
        ));
    }
}
