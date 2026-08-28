//! Turning one configured file into a save directory the game can load.
//!
//! # What `path` points at
//!
//! A **file**, not a directory, because that is what a file manager's "copy full path" produces.
//! Four shapes are accepted and told apart by extension:
//!
//! | extension | handling |
//! | --- | --- |
//! | `.sl2` | read directly |
//! | `.zip` | find the one `DS2SOFS0000.sl2` inside |
//! | `.7z` | as above |
//! | `.rar` | as above |
//!
//! Inside an archive the member may be at any depth -- `DarkSoulsII/<steamid>/DS2SOFS0000.sl2` and
//! a bare `DS2SOFS0000.sl2` at the root are both ordinary, and both occur among real downloads.
//! Only the final path component is matched, and **exactly one** match is required: zero means the
//! archive holds no SOTFS save, more than one means nobody can say which was meant. Both are
//! refused by name rather than resolved by picking the first.
//!
//! # Every launch re-stages, and that is the feature
//!
//! The staged copy is what the game reads AND writes, so progress made in a redirected run lives
//! in the staging directory. Re-extracting on every launch therefore resets it -- which is the
//! intended behaviour for a read-only source: `path` names an archive or a file outside the game
//! directory, and "start from this save" is what that means. A run that should keep its progress
//! is a run that should not be pointing at an archive.
//!
//! # The Steam ID is not configured
//!
//! It arrives as the detour's own second argument -- the game fetches the running account's ID to
//! build the folder name and hands it straight to the function this crate replaces. So the rebind
//! that makes a donor save loadable needs nothing from the user at all.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use ds2_sl2_core::{Rebound, Sl2Error};

/// The only save file name SOTFS builds. The game appends it to whatever directory the hooked
/// function leaves behind, so a staged file under any other name is invisible to it.
pub const SAVE_FILE_NAME: &str = "DS2SOFS0000.sl2";

/// What staging failed at, in terms a log line can name and a reader can act on.
#[derive(Debug)]
pub enum StageError {
    /// The configured path does not exist, or could not be read.
    Unreadable(String),
    /// The extension is not one of the four handled here.
    UnknownKind(String),
    /// The archive contains no `DS2SOFS0000.sl2`.
    NoSaveInArchive,
    /// The archive contains more than one, so the choice is ambiguous.
    ManySavesInArchive(usize),
    /// The archive itself could not be opened or decompressed.
    Archive(String),
    /// The save decrypted, but the rebind refused it.
    Sl2(Sl2Error),
    /// The staging directory could not be created or written.
    Write(String),
}

impl core::fmt::Display for StageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unreadable(why) => write!(f, "cannot read the configured path: {why}"),
            Self::UnknownKind(ext) => {
                write!(
                    f,
                    "unhandled file type {ext:?}; expected .sl2, .zip, .7z or .rar"
                )
            }
            Self::NoSaveInArchive => write!(f, "no {SAVE_FILE_NAME} inside the archive"),
            Self::ManySavesInArchive(n) => {
                write!(
                    f,
                    "{n} copies of {SAVE_FILE_NAME} inside the archive; which one is meant?"
                )
            }
            Self::Archive(why) => write!(f, "archive error: {why}"),
            Self::Sl2(why) => write!(f, "save error: {why}"),
            Self::Write(why) => write!(f, "cannot write the staged save: {why}"),
        }
    }
}

/// What [`stage`] produced.
#[derive(Debug)]
pub struct Staged {
    /// The directory to hand the game. Contains exactly [`SAVE_FILE_NAME`].
    pub directory: PathBuf,
    /// How the source was read, for the log: `sl2`, `zip`, `7z` or `rar`.
    pub kind: &'static str,
    /// Size of the save that was staged.
    pub bytes: usize,
    /// What the rebind changed.
    pub rebound: Rebound,
}

/// True when this archive member is the save, matched on its final component only.
///
/// Archive members always use `/`, but a member written on Windows can carry `\` in its name, so
/// both are treated as separators rather than trusting the producer.
fn is_save_member(name: &str) -> bool {
    name.rsplit(['/', '\\'])
        .next()
        .is_some_and(|leaf| leaf.eq_ignore_ascii_case(SAVE_FILE_NAME))
}

/// Pick the single matching member out of a list of `(name, index)`, or say why not.
fn only_match<T>(mut found: Vec<T>) -> Result<T, StageError> {
    match found.len() {
        0 => Err(StageError::NoSaveInArchive),
        1 => Ok(found.remove(0)),
        n => Err(StageError::ManySavesInArchive(n)),
    }
}

fn read_zip(path: &Path) -> Result<Vec<u8>, StageError> {
    let file = fs::File::open(path).map_err(|e| StageError::Unreadable(e.to_string()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| StageError::Archive(e.to_string()))?;
    let indices: Vec<usize> = (0..archive.len())
        .filter(|i| {
            archive
                .by_index_raw(*i)
                .map(|e| !e.is_dir() && is_save_member(e.name()))
                .unwrap_or(false)
        })
        .collect();
    let index = only_match(indices)?;
    let mut entry = archive
        .by_index(index)
        .map_err(|e| StageError::Archive(e.to_string()))?;
    let mut out = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut out)
        .map_err(|e| StageError::Archive(e.to_string()))?;
    Ok(out)
}

fn read_7z(path: &Path) -> Result<Vec<u8>, StageError> {
    let mut reader = sevenz_rust::SevenZReader::open(path, sevenz_rust::Password::empty())
        .map_err(|e| StageError::Archive(e.to_string()))?;
    let mut hits: Vec<Vec<u8>> = Vec::new();
    reader
        .for_each_entries(|entry, rest| {
            if !entry.is_directory() && is_save_member(entry.name()) {
                let mut buf = Vec::new();
                rest.read_to_end(&mut buf)?;
                hits.push(buf);
            }
            // Keep walking: finding a second copy is what makes the choice ambiguous, and a
            // reader that stopped at the first would silently pick one.
            Ok(true)
        })
        .map_err(|e| StageError::Archive(e.to_string()))?;
    only_match(hits)
}

fn read_rar(path: &Path) -> Result<Vec<u8>, StageError> {
    let file = fs::File::open(path).map_err(|e| StageError::Unreadable(e.to_string()))?;
    let mut archive =
        unrar_rs::RarArchive::open(file).map_err(|e| StageError::Archive(e.to_string()))?;
    // Collected as owned names first: `member_names` borrows the archive and `extract_member`
    // needs it mutably.
    let names: Vec<String> = archive
        .member_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let wanted: Vec<String> = names.into_iter().filter(|n| is_save_member(n)).collect();
    let name = only_match(wanted)?;
    let index = archive
        .find_member(&name)
        .ok_or(StageError::NoSaveInArchive)?;
    let member = archive
        .extract_member(index, &unrar_rs::ExtractOptions::default(), None)
        .map_err(|e| StageError::Archive(e.to_string()))?;
    member
        .to_bytes()
        .map_err(|e| StageError::Archive(e.to_string()))
}

/// Read the save bytes out of whatever the configured path points at.
fn read_source(path: &Path) -> Result<(&'static str, Vec<u8>), StageError> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "sl2" => {
            let bytes = fs::read(path).map_err(|e| StageError::Unreadable(e.to_string()))?;
            Ok(("sl2", bytes))
        }
        "zip" => Ok(("zip", read_zip(path)?)),
        "7z" => Ok(("7z", read_7z(path)?)),
        "rar" => Ok(("rar", read_rar(path)?)),
        other => Err(StageError::UnknownKind(other.to_owned())),
    }
}

/// Resolve `source` into `staging_root`, rebound to `steam_id`, and return the directory to use.
///
/// `steam_id` is the running account's, sixteen hex characters, exactly as the game spells it when
/// it builds its own folder name.
pub fn stage(source: &Path, steam_id: &str, staging_root: &Path) -> Result<Staged, StageError> {
    let (kind, mut save) = read_source(source)?;
    let rebound = ds2_sl2_core::rebind(&mut save, steam_id).map_err(StageError::Sl2)?;
    fs::create_dir_all(staging_root).map_err(|e| StageError::Write(e.to_string()))?;
    let destination = staging_root.join(SAVE_FILE_NAME);
    fs::write(&destination, &save).map_err(|e| StageError::Write(e.to_string()))?;
    Ok(Staged {
        directory: staging_root.to_path_buf(),
        kind,
        bytes: save.len(),
        rebound,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_save_at_any_depth_and_on_either_separator() {
        assert!(is_save_member("DS2SOFS0000.sl2"));
        assert!(is_save_member(
            "DarkSoulsII/01100001526d6d84/DS2SOFS0000.sl2"
        ));
        assert!(is_save_member(
            "DarkSoulsII\\01100001526d6d84\\DS2SOFS0000.sl2"
        ));
        // Vanilla Dark Souls II. A different game with a different key; never a match.
        assert!(!is_save_member("DARKSII0000.sl2"));
        // A name that merely ends with the save's name is not the save.
        assert!(!is_save_member("notDS2SOFS0000.sl2"));
    }

    #[test]
    fn ambiguity_is_refused_rather_than_resolved() {
        assert!(matches!(
            only_match::<u8>(vec![]),
            Err(StageError::NoSaveInArchive)
        ));
        assert!(matches!(only_match(vec![7u8]), Ok(7)));
        assert!(matches!(
            only_match(vec![1u8, 2]),
            Err(StageError::ManySavesInArchive(2))
        ));
    }
}
