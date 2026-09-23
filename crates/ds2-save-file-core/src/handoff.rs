//! The one-line file that carries a picked save from one launch to the next.
//!
//! # Why the next launch and not this one
//!
//! DARK SOULS II keeps ONE save container per Steam account, so "load a character from a file" means
//! pointing the game at a different container -- which is what `ds2-save-redirect` does. Doing that
//! mid-session does not work, and the reason is worth writing down because it is not obvious and it
//! cost a design:
//!
//! **DS2 saves on the way out.** The pause menu's Quit Game row is
//! `FeGroupInGameReturnTitleCheck`, which persists the character before it returns to the title. So
//! a session that re-points the save directory and then quits to the title writes the CURRENT
//! character into the staged copy, and the LOAD GAME that follows reads back the character the player
//! was trying to replace. The swap has to land on a process that has not played anything yet.
//!
//! So the row writes this file, and the loader reads it during `DLL_PROCESS_ATTACH` on the next
//! launch -- before the game has built a save path, let alone saved into one.
//!
//! # It is consumed, not remembered
//!
//! The loader DELETES the file after reading it. That is the whole safety property: a handoff that
//! persisted would leave a player in somebody else's save on every launch until they found a text
//! file to delete, and the failure mode of a mod that quietly keeps loading the wrong save is a
//! player who thinks their character is gone.
//!
//! One line of text, because the thing being carried is one path and a format with room for a second
//! field is a format somebody will put a second field in.

/// The handoff file's name. Written next to `DarkSoulsII.exe`, beside the loader's own log.
///
/// Named for what it does rather than for the crate that writes it: a player who finds this in their
/// game directory should be able to tell from the name alone that deleting it is safe.
pub const HANDOFF_FILE_NAME: &str = "ds2-load-next-save.txt";

/// The file's whole contents for `path`, terminator included.
///
/// A leading comment line, because this lands in the player's game directory and an unexplained file
/// full of a path is the kind of thing that gets deleted in a panic or kept forever by mistake.
pub fn encode(path: &str) -> String {
    format!(
        "# {HANDOFF_FILE_NAME}: the save DARK SOULS II loads on its NEXT launch, and only that one.\n\
         # Written by the pause menu's Load Character from File row. Deleting this file cancels it.\n\
         {}\n",
        path.trim()
    )
}

/// The path a handoff file names, or `None` if it names none.
///
/// The first line that is neither blank nor a comment. Everything after it is ignored rather than
/// refused: a file with two paths in it is a file somebody edited by hand, and taking the first is
/// the same answer as taking the only one in the case that actually happens.
pub fn decode(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What is written is what is read back, including a path with spaces in it.
    #[test]
    fn a_path_survives_the_round_trip() {
        let path = r"C:\Users\p\Downloads\a donor save\DS2SOFS0000.sl2";
        assert_eq!(decode(&encode(path)).as_deref(), Some(path));
    }

    /// The comment lines are comments, not the answer.
    #[test]
    fn the_explanation_is_not_mistaken_for_the_path() {
        let text = encode("Z:/home/p/donor.sl2");
        assert!(text.starts_with('#'), "the file explains itself first");
        assert_eq!(decode(&text).as_deref(), Some("Z:/home/p/donor.sl2"));
    }

    /// A file with nothing in it cancels rather than arming something empty.
    #[test]
    fn an_empty_file_names_nothing() {
        assert_eq!(decode(""), None);
        assert_eq!(decode("\n\n   \n"), None);
        assert_eq!(decode("# only a comment\n"), None);
    }

    /// Surrounding whitespace is not part of a path.
    #[test]
    fn whitespace_is_trimmed_from_both_sides() {
        assert_eq!(
            decode("\n   C:\\saves\\donor.sl2   \n").as_deref(),
            Some(r"C:\saves\donor.sl2")
        );
    }

    /// A hand-edited file with two paths takes the first rather than refusing both.
    #[test]
    fn the_first_path_wins() {
        assert_eq!(
            decode("first.sl2\nsecond.sl2\n").as_deref(),
            Some("first.sl2")
        );
    }
}
