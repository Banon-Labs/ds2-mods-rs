//! Why a picked path is not something this picker will hand to the loader, in words a player can
//! act on.
//!
//! # Two gates, and the order they run in
//!
//! The cheap one is `ds2_save_file_core::accepts`: it reads the extension and nothing else, so a
//! screenshot is refused without the file ever being opened. The expensive one is
//! `ds2_sl2_core::slots`: it decrypts the container's payloads and comes back with the ten
//! character records, which is the only way to know whether there is anything in there to load.
//!
//! The cheap gate runs FIRST, and deliberately before the model touches the filesystem at all.
//! That ordering is what makes a wrong extension say *"that is not a save"* instead of *"that
//! file does not exist"* -- and it is what lets the whole refusal path be exercised on Linux
//! against Windows paths that no host will ever stat.
//!
//! # Archives are accepted without a claim about their characters
//!
//! `.zip`, `.7z` and `.rar` are offered because that is the shape a save arrives in when somebody
//! sends you one, and `ds2-save-redirect`'s staging step already unwraps them. Nothing here can
//! read the characters inside one without unpacking it, so this does not pretend to: an archive
//! comes back as [`PickedSource::Archive`] and the pick ends at the file. Saying anything more
//! confident than that would be inventing a fact about a file nobody has opened.
//!
//! # Every container failure is one refusal
//!
//! `ds2_sl2_core::slots` fails several ways -- not a BND4, a truncated entry table, a payload
//! that is not block-aligned, no character list in any payload -- and all of them mean the same
//! thing to a player standing in front of a list of files: this file is not a save this can read.
//! They collapse to [`PickRejection::NotBnd4`]. [`PickRejection::NoLoadableCharacter`] is kept
//! for the case that is genuinely different and genuinely common: the container IS readable, its
//! ten slots were read, and every one of them is empty.

use std::path::Path;

use ds2_sl2_core::{SaveSlot, slots};

/// Why a candidate path is not something this picker would offer.
///
/// Discriminants are explicit and start at 1 so `as usize` can be reported where 0 means "no
/// rejection recorded".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickRejection {
    /// Nothing is there, or what is there is not a file.
    NotAFile = 1,
    /// An extension outside `ds2_save_file_core::SOURCE_EXTENSIONS`, or no extension at all.
    WrongExtension = 2,
    /// The bytes could not be read.
    Unreadable = 3,
    /// Read, and not a container this can walk. Covers every structural failure; see module docs.
    NotBnd4 = 4,
    /// A readable container whose ten slots are all empty.
    NoLoadableCharacter = 5,
    /// The path did not round-trip through UTF-8, so nothing downstream can name it.
    PathNotUtf8 = 6,
}

/// User-facing picker text: a headline a surface can put in front of the reason, and one line
/// that says what to do about it.
///
/// The words are for the player, in the second person. Log wording and any counters stay with
/// whoever is doing the logging -- this carries no formatting a surface has to undo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerStatusMessage {
    headline: String,
    detail: String,
}

impl PickerStatusMessage {
    /// Build one from the two lines a surface will show.
    pub fn new(headline: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            headline: headline.into(),
            detail: detail.into(),
        }
    }

    /// The first line: what happened, in the player's terms.
    pub fn headline(&self) -> &str {
        &self.headline
    }

    /// The second line: why, in enough detail to act on.
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl PickRejection {
    /// The reporting code. 0 is reserved for "nothing was rejected".
    pub const fn as_code(self) -> usize {
        self as usize
    }

    /// What the picker puts on screen when this refusal happens.
    pub fn status_message(self) -> PickerStatusMessage {
        match self {
            Self::NotAFile => PickerStatusMessage::new(
                "SAVE NOT FOUND",
                "That path is missing, or it is not a file. Choose another.",
            ),
            Self::WrongExtension => PickerStatusMessage::new(
                "WRONG FILE TYPE",
                format!("Choose a save: {}.", ds2_save_file_core::source::offered()),
            ),
            Self::Unreadable => PickerStatusMessage::new(
                "SAVE UNREADABLE",
                "The file is there, and it could not be read. Choose another.",
            ),
            Self::NotBnd4 => PickerStatusMessage::new(
                "NOT A DARK SOULS II SAVE",
                "That file is not a save container this can read. Choose another.",
            ),
            Self::NoLoadableCharacter => PickerStatusMessage::new(
                "NO CHARACTER IN THAT SAVE",
                "Every slot in that save is empty. Choose a save with a character in it.",
            ),
            Self::PathNotUtf8 => PickerStatusMessage::new(
                "PATH NOT SUPPORTED",
                "That path cannot be named safely by the save picker. Choose another.",
            ),
        }
    }
}

/// What a picked path turned out to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickedSource {
    /// A bare `.sl2`, and the ten slots read out of it -- empties included, in slot order.
    Container(Vec<SaveSlot>),
    /// An archive. Its characters are unknowable until the staging step unwraps it, so the pick
    /// ends here and the whole file is the answer.
    Archive,
}

/// Refuse a container whose every slot is empty, and pass any other reading through unchanged.
///
/// Split out from [`accepts_pick`] because it is the one container decision that owes nothing to
/// the filesystem: a synthetic reading of ten empty slots exercises it exactly, and a synthetic
/// `.sl2` cannot be built from outside `ds2-sl2-core` (its key and its cipher are private, which
/// is the right shape for them and means this decision would otherwise go untested).
///
/// A blank slot -- nine stats of `1`, no name -- COUNTS as loadable. That is measured, not
/// assumed: `ds2_sl2_core::slots` documents the runtime loading exactly such a slot and setting
/// its own occupied bit. Refusing it here would hide a character the game is perfectly willing
/// to load.
///
/// # Errors
///
/// `PickRejection::NoLoadableCharacter` when not one of the ten slots can be loaded.
pub fn accept_slots(slots: Vec<SaveSlot>) -> Result<Vec<SaveSlot>, PickRejection> {
    if slots.iter().any(|slot| slot.state.is_loadable()) {
        Ok(slots)
    } else {
        Err(PickRejection::NoLoadableCharacter)
    }
}

/// Judge a path the picker is about to commit to, reading it if it has to.
///
/// The order of the checks is load-bearing and is asserted in the tests below: UTF-8, then the
/// extension, then the filesystem. Anything that can be decided from the TEXT of the path is
/// decided before the path is stat'd, so the answer is the same on a Linux test host as it is in
/// the game -- and so the common refusal (a file that is simply not a save) never depends on a
/// disk the test does not have.
///
/// # Errors
///
/// The [`PickRejection`] for the first check that failed, in that order: `PathNotUtf8`,
/// `WrongExtension`, `NotAFile`, then whatever reading the container itself produced.
pub fn accepts_pick(path: &Path) -> Result<PickedSource, PickRejection> {
    if path.to_str().is_none() {
        return Err(PickRejection::PathNotUtf8);
    }
    // The cheap gate, and the hand-parsed one. `accepts` finds the leaf and its dot with
    // `rfind(['\\', '/'])` precisely so a Windows path judged on Linux gets the game's answer.
    let extension = ds2_save_file_core::accepts(path).map_err(|_| PickRejection::WrongExtension)?;
    if !path.is_file() {
        return Err(PickRejection::NotAFile);
    }
    let Ok(bytes) = std::fs::read(path) else {
        return Err(PickRejection::Unreadable);
    };
    if extension != ds2_save_file_core::SAVE_EXTENSION {
        return Ok(PickedSource::Archive);
    }
    let Ok(found) = slots(&bytes) else {
        return Err(PickRejection::NotBnd4);
    };
    accept_slots(found).map(PickedSource::Container)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ds2_save_file_core::SOURCE_EXTENSIONS;
    use ds2_sl2_core::SlotState;
    use std::path::PathBuf;

    fn slot(index: usize, state: SlotState, stats: [i16; 9]) -> SaveSlot {
        SaveSlot {
            slot: index,
            state,
            name: String::new(),
            stats,
        }
    }

    /// THE ORDERING TEST. On this host neither path exists, so if the filesystem check ran first
    /// both would answer `NotAFile` and the player would be told their save is missing when what
    /// they actually picked was a screenshot.
    #[test]
    fn a_windows_path_is_judged_by_its_name_before_any_disk_is_touched() {
        assert_eq!(
            accepts_pick(Path::new(r"Z:\home\banon\screenshot.png")),
            Err(PickRejection::WrongExtension)
        );
        assert_eq!(
            accepts_pick(Path::new(r"Z:\home\banon\DS2SOFS0000.sl2")),
            Err(PickRejection::NotAFile),
            "past the name gate, a path that is not on disk is missing -- not the wrong type"
        );
    }

    /// The dot that decides is the one in the LEAF. `Path::extension` would read `0\donor` here.
    #[test]
    fn a_dot_in_a_folder_name_does_not_pass_the_extension_gate() {
        assert_eq!(
            accepts_pick(Path::new(r"C:\v1.0\donor")),
            Err(PickRejection::WrongExtension)
        );
    }

    /// The bare-save extension is one of the shapes the listing offers, which is what makes the
    /// character sub-stage reachable at all.
    #[test]
    fn the_bare_save_extension_is_one_of_the_offered_shapes() {
        assert!(SOURCE_EXTENSIONS.contains(&ds2_save_file_core::SAVE_EXTENSION));
    }

    #[test]
    fn a_container_with_one_blank_slot_is_loadable_and_ten_empties_are_not() {
        let empties: Vec<SaveSlot> = (0..10)
            .map(|index| slot(index, SlotState::Empty, [0; 9]))
            .collect();
        assert_eq!(
            accept_slots(empties.clone()),
            Err(PickRejection::NoLoadableCharacter)
        );

        let mut one_blank = empties.clone();
        one_blank[4] = slot(4, SlotState::Blank, [1; 9]);
        assert_eq!(
            accept_slots(one_blank).map(|kept| kept.len()),
            Ok(10),
            "a blank slot is a character the game loads, and the empties keep their rows"
        );

        let mut one_played = empties;
        one_played[9] = slot(9, SlotState::Occupied, [12, 10, 8, 9, 11, 14, 6, 5, 4]);
        assert!(accept_slots(one_played).is_ok());
    }

    /// The refusal must not quietly drop the empty slots: their row indexes are the game's slot
    /// numbers, and a list that skips them renumbers every character below.
    #[test]
    fn accepting_a_container_preserves_every_slot_in_order() {
        let mut found: Vec<SaveSlot> = (0..10)
            .map(|index| slot(index, SlotState::Empty, [0; 9]))
            .collect();
        found[3] = slot(3, SlotState::Occupied, [20; 9]);
        let kept = accept_slots(found).expect("one occupied slot is enough");
        assert_eq!(
            kept.iter().map(|kept| kept.slot).collect::<Vec<_>>(),
            (0..10).collect::<Vec<_>>()
        );
        assert_eq!(kept[3].state, SlotState::Occupied);
    }

    #[test]
    fn a_file_that_is_not_a_container_is_refused_as_not_a_save() {
        let dir = crate::picker_scratch_dir("reason-junk");
        let path = dir.join("donor.sl2");
        std::fs::write(&path, b"this is not a BND4").expect("scratch file must be writable");
        assert_eq!(accepts_pick(&path), Err(PickRejection::NotBnd4));
    }

    /// An archive is taken at face value: accepted, with nothing claimed about what is inside.
    #[test]
    fn an_archive_is_accepted_without_reading_characters_out_of_it() {
        let dir = crate::picker_scratch_dir("reason-archive");
        for leaf in ["donor.zip", "donor.7z", "donor.rar"] {
            let path = dir.join(leaf);
            std::fs::write(&path, b"PK\x03\x04 not really").expect("scratch file must be writable");
            assert_eq!(accepts_pick(&path), Ok(PickedSource::Archive), "{leaf}");
        }
    }

    #[test]
    fn a_directory_is_not_a_file_even_when_it_is_named_like_a_save() {
        let dir = crate::picker_scratch_dir("reason-dir");
        let masquerading = dir.join("DS2SOFS0000.sl2");
        std::fs::create_dir_all(&masquerading).expect("scratch dir must be creatable");
        assert_eq!(accepts_pick(&masquerading), Err(PickRejection::NotAFile));
    }

    /// Every refusal has to say something, and say something DIFFERENT -- two refusals sharing a
    /// headline is two refusals the player cannot tell apart.
    #[test]
    fn every_refusal_has_its_own_words() {
        let every = [
            PickRejection::NotAFile,
            PickRejection::WrongExtension,
            PickRejection::Unreadable,
            PickRejection::NotBnd4,
            PickRejection::NoLoadableCharacter,
            PickRejection::PathNotUtf8,
        ];
        let mut headlines: Vec<String> = Vec::new();
        for rejection in every {
            let message = rejection.status_message();
            assert!(
                !message.headline().is_empty(),
                "{rejection:?} has no headline"
            );
            assert!(!message.detail().is_empty(), "{rejection:?} has no detail");
            headlines.push(message.headline().to_owned());
        }
        let mut unique = headlines.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            unique.len(),
            headlines.len(),
            "two refusals share a headline"
        );
    }

    /// A refused pick has to name what WOULD have worked, or the player is guessing.
    #[test]
    fn the_wrong_type_refusal_names_every_extension_that_would_have_worked() {
        let detail = PickRejection::WrongExtension.status_message();
        for extension in SOURCE_EXTENSIONS {
            assert!(
                detail.detail().contains(extension),
                "{} omits {extension}",
                detail.detail()
            );
        }
    }

    /// Codes are stable and none of them is 0, which is reserved for "nothing was rejected".
    #[test]
    fn no_refusal_takes_the_code_that_means_no_refusal() {
        for rejection in [
            PickRejection::NotAFile,
            PickRejection::WrongExtension,
            PickRejection::Unreadable,
            PickRejection::NotBnd4,
            PickRejection::NoLoadableCharacter,
            PickRejection::PathNotUtf8,
        ] {
            assert_ne!(rejection.as_code(), 0);
        }
        assert_eq!(PickRejection::NotAFile.as_code(), 1);
    }

    /// A path with no name at all is refused by the cheap gate rather than by a `read_dir`.
    #[test]
    fn an_empty_path_never_reaches_the_filesystem() {
        assert_eq!(
            accepts_pick(&PathBuf::new()),
            Err(PickRejection::WrongExtension)
        );
    }
}
