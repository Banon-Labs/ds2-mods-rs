//! Who lives in a save file, said on its row before the player opens it.
//!
//! A listing of `DS2SOFS0000.sl2`, `DS2SOFS0000 (1).sl2` and `backup.sl2` tells a player nothing
//! about which one holds the character they want. The row says instead: how many characters the
//! container can load, the first two by name and soul level, and when the file was last written.
//!
//! # Lazy, cached, and only for containers
//!
//! Reading the characters costs a full read and a decrypt of every payload, so the listing does
//! not pay it (see `SavePickerModel::refresh` on why the cheap gate is the only one there). The
//! model asks for a summary only for a row it is about to show, keeps it per path, and throws the
//! cache away on the next listing so a file rewritten since is read again. An archive gets no
//! summary: nothing can name its characters without unpacking it, which is `reason`'s rule too.
//!
//! # The time is UTC
//!
//! There is no date-time dependency in this workspace and the host's zone is not something a pure
//! crate can ask for portably, so the stamp is UTC and says so nowhere on screen. It exists to
//! tell two copies of one save apart, which an offset does not change. The civil-date arithmetic
//! is Hinnant's `days_from_civil` inverse, the same one `er-save-picker-core` uses.

use std::path::Path;
use std::time::SystemTime;

use ds2_sl2_core::{SaveSlot, slots};

use crate::text::character_name;

/// How many characters a row names before it falls back to a count.
pub const SUMMARY_NAMED_CHARACTERS: usize = 2;

/// Separates two named characters on a row.
pub const SUMMARY_SEPARATOR: &str = " / ";

/// What a row says when the file is a container that could not be read.
pub const SUMMARY_UNREADABLE_TEXT: &str = "unreadable";

/// What a row says when the container was read and not one slot can be loaded.
pub const SUMMARY_NO_CHARACTERS_TEXT: &str = "no characters";

/// What one container file holds, as far as a row needs to know.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileSummary {
    /// `None` when the file could not be read as a container at all.
    characters: Option<Vec<(String, i32)>>,
}

impl FileSummary {
    /// A container that could not be read.
    pub fn unreadable() -> Self {
        Self { characters: None }
    }

    /// Summarise ten slots as read: the loadable ones, in slot order, by name and soul level.
    pub fn from_slots(found: &[SaveSlot]) -> Self {
        Self {
            characters: Some(
                found
                    .iter()
                    .filter(|slot| slot.state.is_loadable())
                    .map(|slot| (character_name(slot).to_owned(), slot.soul_level()))
                    .collect(),
            ),
        }
    }

    /// Read `path` and summarise it. Every failure is [`FileSummary::unreadable`]: a row has no
    /// room for the difference between a truncated table and a bad cipher, and the pick itself
    /// will say which it was.
    pub fn read(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| slots(&bytes).ok())
            .map_or_else(Self::unreadable, |found| Self::from_slots(&found))
    }

    /// Whether the file read as a container.
    pub fn readable(&self) -> bool {
        self.characters.is_some()
    }

    /// Loadable characters in it. Zero for an unreadable file as well as an empty one;
    /// [`FileSummary::readable`] tells them apart.
    pub fn character_count(&self) -> usize {
        self.characters.as_ref().map_or(0, Vec::len)
    }

    /// The row's line: `Vendrick SL136 / Lucatiel SL40 +2`.
    pub fn text(&self) -> String {
        let Some(characters) = &self.characters else {
            return SUMMARY_UNREADABLE_TEXT.to_owned();
        };
        if characters.is_empty() {
            return SUMMARY_NO_CHARACTERS_TEXT.to_owned();
        }
        let named: Vec<String> = characters
            .iter()
            .take(SUMMARY_NAMED_CHARACTERS)
            .map(|(name, level)| format!("{name} SL{level}"))
            .collect();
        let mut line = named.join(SUMMARY_SEPARATOR);
        let rest = characters.len().saturating_sub(SUMMARY_NAMED_CHARACTERS);
        if rest > 0 {
            line.push_str(&format!(" +{rest}"));
        }
        line
    }
}

/// Civil date-time fields, in UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CivilDateTime {
    /// Proleptic Gregorian year.
    pub year: i64,
    /// `1..=12`.
    pub month: u32,
    /// `1..=31`.
    pub day: u32,
    /// `0..=23`.
    pub hour: u32,
    /// `0..=59`.
    pub minute: u32,
}

const SECONDS_PER_DAY: i64 = 86_400;

/// Split seconds since the Unix epoch into UTC civil fields. `None` before the epoch -- a save
/// dated before 1970 is a broken clock, not a date worth printing.
pub fn civil_from_unix_seconds(secs: i64) -> Option<CivilDateTime> {
    if secs < 0 {
        return None;
    }
    let days = secs.div_euclid(SECONDS_PER_DAY);
    let secs_of_day = secs.rem_euclid(SECONDS_PER_DAY);
    // Shift the epoch to 0000-03-01 so a leap day lands at the end of its 400-year era.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).ok()?;
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).ok()?;
    let year = yoe + era * 400 + i64::from(month <= 2);
    Some(CivilDateTime {
        year,
        month,
        day,
        hour: u32::try_from(secs_of_day / 3_600).ok()?,
        minute: u32::try_from(secs_of_day % 3_600 / 60).ok()?,
    })
}

/// `YYYY-MM-DD HH:MM`, UTC. `None` for a time before the epoch or past what `i64` seconds hold.
pub fn format_modified(time: SystemTime) -> Option<String> {
    let secs = i64::try_from(time.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs()).ok()?;
    let at = civil_from_unix_seconds(secs)?;
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        at.year, at.month, at.day, at.hour, at.minute
    ))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ds2_sl2_core::SlotState;

    use super::*;

    fn slot(index: usize, state: SlotState, name: &str, stat: i16) -> SaveSlot {
        SaveSlot {
            slot: index,
            state,
            name: name.to_owned(),
            stats: [stat; 9],
        }
    }

    fn ten(occupied: &[(usize, &str, i16)]) -> Vec<SaveSlot> {
        (0..10)
            .map(
                |index| match occupied.iter().find(|(at, ..)| *at == index) {
                    Some((_, name, stat)) => slot(index, SlotState::Occupied, name, *stat),
                    None => slot(index, SlotState::Empty, "", 0),
                },
            )
            .collect()
    }

    /// Two names, then a count, in slot order. The levels are the game's, not the stat sums.
    #[test]
    fn a_row_names_two_characters_and_counts_the_rest() {
        let found = ten(&[
            (1, "Vendrick", 20),
            (4, "Lucatiel", 10),
            (7, "Benhart", 12),
            (9, "Gilligan", 8),
        ]);
        let summary = FileSummary::from_slots(&found);
        assert_eq!(summary.character_count(), 4);
        assert_eq!(
            summary.text(),
            format!(
                "Vendrick SL{} / Lucatiel SL{} +2",
                found[1].soul_level(),
                found[4].soul_level()
            )
        );
    }

    #[test]
    fn one_character_is_named_with_no_count() {
        let summary = FileSummary::from_slots(&ten(&[(3, "Solaire", 10)]));
        assert_eq!(summary.text(), "Solaire SL37");
    }

    /// Hollow slots cannot be loaded, so they are not counted and not named; blank ones can.
    #[test]
    fn only_loadable_slots_are_counted() {
        let mut found = ten(&[]);
        found[2] = slot(2, SlotState::Hollow, "Ghost", 10);
        found[5] = slot(5, SlotState::Blank, "", 1);
        let summary = FileSummary::from_slots(&found);
        assert_eq!(summary.character_count(), 1);
        assert_eq!(summary.text(), "[ unnamed ] SL1");
    }

    #[test]
    fn an_empty_container_and_an_unreadable_one_say_different_things() {
        let empty = FileSummary::from_slots(&ten(&[]));
        assert!(empty.readable());
        assert_eq!(empty.text(), SUMMARY_NO_CHARACTERS_TEXT);
        let broken = FileSummary::unreadable();
        assert!(!broken.readable());
        assert_eq!(broken.character_count(), 0);
        assert_eq!(broken.text(), SUMMARY_UNREADABLE_TEXT);
    }

    #[test]
    fn a_file_that_is_not_a_container_reads_as_unreadable() {
        let dir = crate::picker_scratch_dir("summary-junk");
        let path = dir.join("junk.sl2");
        std::fs::write(&path, b"not a BND4").expect("scratch file must be writable");
        assert_eq!(FileSummary::read(&path), FileSummary::unreadable());
        assert_eq!(
            FileSummary::read(&dir.join("missing.sl2")),
            FileSummary::unreadable()
        );
    }

    #[test]
    fn the_stamp_is_utc_and_zero_padded() {
        assert_eq!(
            format_modified(SystemTime::UNIX_EPOCH).as_deref(),
            Some("1970-01-01 00:00")
        );
        // 2024-02-29 23:59:30 UTC: a leap day, and seconds are dropped rather than rounded.
        let leap = SystemTime::UNIX_EPOCH + Duration::from_secs(1_709_251_170);
        assert_eq!(format_modified(leap).as_deref(), Some("2024-02-29 23:59"));
        // 2026-09-27 08:03 UTC.
        let today = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_496_180);
        assert_eq!(format_modified(today).as_deref(), Some("2026-09-27 08:03"));
    }

    #[test]
    fn a_time_before_the_epoch_is_not_printed() {
        assert_eq!(civil_from_unix_seconds(-1), None);
        let before = SystemTime::UNIX_EPOCH - Duration::from_secs(60);
        assert_eq!(format_modified(before), None);
    }
}
