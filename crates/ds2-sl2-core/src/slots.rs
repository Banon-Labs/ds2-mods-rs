//! Reading the ten character slots out of a `.sl2` without the game.
//!
//! This is the half a file picker needs and [`crate::validate`] does not have: `validate` walks the
//! BND4 entry table and can say "this is a save", which is enough to refuse a renamed jpeg and not
//! enough to put a name on a row. A player choosing between two downloaded containers is choosing
//! between characters, not between files.
//!
//! # Where the list lives
//!
//! Inside a decrypted payload, not in the container header. Each payload is a chain of sections --
//! `id`, `version`, `size`, then `size` bytes -- and the character list is the section with
//! [`SLOT_SECTION_ID`] and exactly [`SLOT_SECTION_SIZE`] bytes: a short prologue followed by ten
//! fixed-stride records. It appears in BOTH global entries (`USER_DATA000` and the last one); they
//! agree, so the first one found is used.
//!
//! # Occupancy is judged from stats, and it has to be
//!
//! The record carries a flags byte, and reading it from a file on disk gives zero for every slot of
//! every real save -- the game derives it when it loads the per-character entries. `ds2-rva`
//! documents the same trap from the runtime side. So a cold read has to judge from CONTENT, which
//! is what [`SlotState`] is.
//!
//! **A slot whose nine stats are all `1` is occupied.** It looks like a blank -- no name, stats
//! under any DS2 starting value -- and treating it as empty was measured wrong: the runtime loaded
//! exactly such a slot and reported its own flags byte with bit 0 set. It is reported as
//! [`SlotState::Blank`] so a caller can label it, and it counts as loadable.
//!
//! # The soul level is the game's own arithmetic, not an estimate
//!
//! [`SaveSlot::soul_level`] is the number the game's character list prints, computed the way the
//! game computes it. `FUN_14038e310` sums the eleven `u16` of the stat block, subtracts the last
//! two -- leaving the nine stats -- subtracts [`LEVEL_ONE_STAT_TOTAL`], clamps to a minimum of one,
//! and stores the result at `PlayerParam+0xD0`; the list's row reads it back from the runtime
//! record and renders it through `L"%d"`. The starting class is not an input.
//!
//! An earlier version of this module refused to report a level, on the stated grounds that DS2
//! derives it from the stats AND the class and the class is not in the record. The first half is
//! wrong, and it was wrong by assumption rather than by reading -- the formula was sitting in the
//! disassembly the whole time. Checked against a character with nine sixes: fifty-four, less
//! fifty-three, is level one.

use crate::{Entry, Sl2Error, cbc, entries};

/// The character-list section's id within a payload's section chain.
pub const SLOT_SECTION_ID: u32 = 4;

/// Its size, which is fixed and is half of what identifies it -- an id alone collides.
pub const SLOT_SECTION_SIZE: u32 = 0x1370;

/// Bytes between the section header and the first record.
const SLOT_PROLOGUE: usize = 16;

/// One record's stride. The same stride the live character-list group uses at runtime.
const SLOT_STRIDE: usize = 0x1F0;

/// Character slots in a DARK SOULS II save, always this many.
pub const SLOT_COUNT: usize = 10;

/// Offset within a record of the eleven `i16`s that begin with the nine stats.
const SLOT_ATTRS_OFFSET: usize = 0x188;

/// How many `i16`s are there. Nine stats and two more this does not name.
const SLOT_ATTRS_COUNT: usize = 11;

/// The nine DS2 stats: vigour, endurance, vitality, attunement, strength, dexterity, adaptability,
/// intelligence, faith. Only these nine decide occupancy; the two after them are not stats.
pub const STAT_COUNT: usize = 9;

/// Offset within a record of the UTF-16LE character name, immediately after the attributes.
const SLOT_NAME_OFFSET: usize = SLOT_ATTRS_OFFSET + SLOT_ATTRS_COUNT * 2;

// The name starts where the attributes end. Written as an assertion rather than as `0x19E` so a
// corrected attribute count cannot leave the name reading from the old place.
const _: () = assert!(SLOT_NAME_OFFSET == 0x19E);

/// Longest name this will read, in bytes, before giving up on finding a terminator.
const SLOT_NAME_MAX_BYTES: usize = 64;

/// The stat value every slot of a named-but-unstarted character carries.
const BLANK_STAT: i16 = 1;

/// What the nine stats of a level-one character add up to, which the game subtracts to get a level.
///
/// A constant in `FUN_14038e310` and not a property of any starting class: every DS2 class begins
/// at soul level one, so every class's nine starting stats sum to this same number. That is the
/// fact which makes a level readable from the file alone.
pub const LEVEL_ONE_STAT_TOTAL: i32 = 53;

/// What one slot holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotState {
    /// All nine stats zero: nothing has ever been written here.
    Empty,
    /// All nine stats `1`: a character the game will load, with nothing rolled yet.
    Blank,
    /// A played character.
    Occupied,
    /// Listed as blank or occupied, but at least one of its two data entries has no end marker, so
    /// the game's section walk would run off the end of it and spin. See [`section_walk`].
    Hollow,
}

impl SlotState {
    /// Whether the game has something here to load. [`SlotState::Blank`] counts, measured;
    /// [`SlotState::Hollow`] does not, because loading it hangs or fails.
    pub fn is_loadable(self) -> bool {
        !matches!(self, SlotState::Empty | SlotState::Hollow)
    }
}

/// A slot's own data lives in two entries, `USER_DATA(slot+1)` and `USER_DATA(slot+11)`.
///
/// The game's pump loads load-content indices slot+8 and slot+0x12, and index 7 is `USER_DATA000`,
/// so index k is `USER_DATA(k-7)`. Measured on a hung load: slot 1's two streams were exactly the
/// length words of `USER_DATA002` and `USER_DATA012`. Mirrors `SLOT_DATA_ENTRY_OFFSETS` in
/// `scripts/ds2-sl2.py`.
const SLOT_DATA_ENTRY_OFFSETS: [usize; 2] = [1, 11];

/// The walk's header: `u32` type at +0, version at +4, size at +8, in 0x20 bytes.
///
/// The same values `ds2-rva` records for the DLL's guard (`SL_SECTION_HEADER_SIZE`,
/// `SL_SECTION_TYPE_END`); this crate carries no game addresses, so the file format is restated
/// here rather than depended on.
const SECTION_HEADER_SIZE: usize = 0x20;

/// The header type that ends the walk.
const SECTION_TYPE_END: u32 = 0xd;

/// Header types the walk accepts: its table at `0x1410da1f0` has fifteen, `0..=0xe`.
const SECTION_TYPE_LIMIT: u32 = 0xf;

/// What the game's section walk does with one data entry's stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionWalk {
    /// It reaches an end-marker header.
    Ends,
    /// It meets a type outside its table and returns an error; the game survives that.
    Bails,
    /// It runs off the end with no end marker and would repeat its last step forever.
    Hangs,
}

/// Emulate the game's section walk (`FUN_1402e47f0` over `DLMemoryInputStream`) on one stream.
///
/// A read copies whatever is left and fails at the end without touching the header; a seek clamps
/// to the end. So a walk that reaches the end before an end marker keeps the last header, seeks
/// nowhere, fails the read again, and never leaves the loop. An all-zero stream gets there because
/// type 0 is a legal type and a size of 0 is a legal size. Mirrors `section_walk` in
/// `scripts/ds2-sl2.py`.
pub fn section_walk(stream: &[u8]) -> SectionWalk {
    if stream.is_empty() {
        // The first header would be uninitialised stack. Nothing can be said except unsafe.
        return SectionWalk::Hangs;
    }
    let mut header = [0u8; SECTION_HEADER_SIZE];
    let mut cursor = 0usize;
    loop {
        if cursor >= stream.len() {
            return SectionWalk::Hangs;
        }
        let chunk = &stream[cursor..stream.len().min(cursor + SECTION_HEADER_SIZE)];
        header[..chunk.len()].copy_from_slice(chunk);
        cursor += chunk.len();
        let field = |at: usize| {
            u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
        };
        let kind = field(0);
        let size = field(8);
        if kind == SECTION_TYPE_END {
            return SectionWalk::Ends;
        }
        if kind >= SECTION_TYPE_LIMIT {
            return SectionWalk::Bails;
        }
        cursor = cursor.saturating_add(size as usize).min(stream.len());
    }
}

/// The bytes the game streams from one decrypted entry: past the `u32` length, that long.
fn entry_stream(plain: &[u8]) -> &[u8] {
    let Some(word) = plain.get(..4) else {
        return &[];
    };
    let length = u32::from_le_bytes([word[0], word[1], word[2], word[3]]) as usize;
    let end = plain.len().min(4usize.saturating_add(length));
    &plain[4..end]
}

/// Whether loading `slot` would send the game's walk off the end of one of its entries.
///
/// A missing entry counts: the game would stream nothing, and the walk's first header would be
/// uninitialised stack.
fn slot_hangs(save: &[u8], table: &[Entry], slot: usize) -> Result<bool, Sl2Error> {
    for delta in SLOT_DATA_ENTRY_OFFSETS {
        let Some(entry) = table.get(slot + delta) else {
            return Ok(true);
        };
        let plain = plaintext(save, entry)?;
        if section_walk(entry_stream(&plain)) == SectionWalk::Hangs {
            return Ok(true);
        }
    }
    Ok(false)
}

/// One character slot, read from the file and from nothing else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveSlot {
    /// Index within the container, `0..SLOT_COUNT`. This is the index the game's own list uses.
    pub slot: usize,
    /// Whether the slot holds a character, and if not, why the record read as empty.
    pub state: SlotState,
    /// The character's name. Empty for an empty or unnamed slot.
    pub name: String,
    /// The nine stats, in the order the record stores them.
    pub stats: [i16; STAT_COUNT],
}

impl SaveSlot {
    /// The nine stats added up.
    pub fn stat_total(&self) -> i32 {
        self.stats.iter().map(|stat| i32::from(*stat)).sum()
    }

    /// The soul level the game's own character list shows for this slot.
    ///
    /// The game's arithmetic, from `FUN_14038e310`: the nine stats, less
    /// [`LEVEL_ONE_STAT_TOTAL`], floored at one. The clamp is the game's and is load-bearing here
    /// rather than defensive -- an empty slot's stats are all zero, which would otherwise read as a
    /// negative level.
    pub fn soul_level(&self) -> i32 {
        (self.stat_total() - LEVEL_ONE_STAT_TOTAL).max(1)
    }
}

/// Classify one slot from its nine stats.
fn classify(stats: &[i16; STAT_COUNT]) -> SlotState {
    if stats.iter().all(|stat| *stat == 0) {
        return SlotState::Empty;
    }
    if stats.iter().all(|stat| *stat == BLANK_STAT) {
        return SlotState::Blank;
    }
    SlotState::Occupied
}

/// Decode a UTF-16LE name, terminating on an **aligned** pair of zero bytes.
///
/// Searching for the two-byte pattern anywhere is the obvious version and it is wrong: in ordinary
/// ASCII-encoded-as-UTF-16 the pattern occurs at an ODD offset one byte before the real terminator
/// (`...6c 00 66 00 00 00`), which eats the last character and leaves half a code unit behind. The
/// same bug in the Python tool cost "Elden Wolf" its f. Stepping two bytes at a time is the fix.
fn wide_name(record: &[u8], offset: usize) -> String {
    let end = record.len().min(offset + SLOT_NAME_MAX_BYTES);
    if offset >= end {
        return String::new();
    }
    let chunk = &record[offset..end];
    let mut length = chunk.len() & !1;
    let mut at = 0;
    while at + 1 < chunk.len() {
        if chunk[at] == 0 && chunk[at + 1] == 0 {
            length = at;
            break;
        }
        at += 2;
    }
    // `length` is forced even above, so the remainder `as_chunks` returns is always empty.
    let (pairs, _) = chunk[..length].as_chunks::<2>();
    let units: Vec<u16> = pairs.iter().copied().map(u16::from_le_bytes).collect();
    String::from_utf16_lossy(&units)
}

/// Read one entry's plaintext.
fn plaintext(save: &[u8], entry: &Entry) -> Result<Vec<u8>, Sl2Error> {
    let iv: [u8; 16] = save[entry.offset + 16..entry.offset + 32]
        .try_into()
        .map_err(|_| Sl2Error::Truncated)?;
    let mut plain = save[entry.offset + 32..entry.offset + entry.size].to_vec();
    cbc(&mut plain, &iv, false)?;
    Ok(plain)
}

/// Find the character-list section in one payload and read its ten records.
fn slots_in_payload(plain: &[u8]) -> Option<Vec<SaveSlot>> {
    // The chain starts four bytes in; each link is `id`, `version`, `size`.
    let mut at = 4usize;
    while at + 12 <= plain.len() {
        let id = u32::from_le_bytes(plain[at..at + 4].try_into().ok()?);
        let size = u32::from_le_bytes(plain[at + 8..at + 12].try_into().ok()?);
        if id == SLOT_SECTION_ID && size == SLOT_SECTION_SIZE {
            return Some(read_records(plain, at + 12 + SLOT_PROLOGUE));
        }
        // A zero-size link cannot be stepped over, and walking on would loop forever.
        if size == 0 {
            return None;
        }
        at = at.checked_add(12 + size as usize)?;
    }
    None
}

/// Read the ten fixed-stride records starting at `base`.
fn read_records(plain: &[u8], base: usize) -> Vec<SaveSlot> {
    let mut out = Vec::with_capacity(SLOT_COUNT);
    for slot in 0..SLOT_COUNT {
        let start = base + slot * SLOT_STRIDE;
        let Some(record) = plain.get(start..start + SLOT_STRIDE) else {
            break;
        };
        let mut stats = [0i16; STAT_COUNT];
        for (index, stat) in stats.iter_mut().enumerate() {
            let at = SLOT_ATTRS_OFFSET + index * 2;
            *stat = i16::from_le_bytes([record[at], record[at + 1]]);
        }
        out.push(SaveSlot {
            slot,
            state: classify(&stats),
            name: wide_name(record, SLOT_NAME_OFFSET),
            stats,
        });
    }
    out
}

/// Every character slot in `save`, in the order the game's own list shows them.
///
/// Returns all ten, empty ones included, because a picker's rows are the game's slots and a row
/// that silently disappears is a row whose index no longer means what the game means by it.
///
/// This is a reading of a file, not a promise about the game: the runtime applies an ownership
/// check this cannot see, and a redirect that stages a foreign container rebinds the Steam ID
/// precisely because of it. See [`crate::rebind`].
/// # Errors
///
/// Whatever reading the container's entry table produced -- a slot that reads as empty is a
/// [`SlotState`], not an error.
pub fn slots(save: &[u8]) -> Result<Vec<SaveSlot>, Sl2Error> {
    let table = entries(save)?;
    for entry in &table {
        let plain = plaintext(save, entry)?;
        if let Some(mut found) = slots_in_payload(&plain) {
            // The list says who is there; the slot's own entries say whether the game can load it.
            for slot in &mut found {
                if slot.state != SlotState::Empty && slot_hangs(save, &table, slot.slot)? {
                    slot.state = SlotState::Hollow;
                }
            }
            return Ok(found);
        }
    }
    // A structurally sound container with no character list in any payload is not this layout.
    Err(Sl2Error::NoSteamId)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zeroed_record_is_empty_and_all_ones_is_a_loadable_blank() {
        assert_eq!(classify(&[0; STAT_COUNT]), SlotState::Empty);
        assert_eq!(classify(&[1; STAT_COUNT]), SlotState::Blank);
        assert_eq!(
            classify(&[12, 10, 8, 9, 11, 14, 6, 5, 4]),
            SlotState::Occupied
        );
        assert!(!SlotState::Empty.is_loadable());
        // Measured: the runtime loaded an all-ones slot and set its own occupied bit.
        assert!(SlotState::Blank.is_loadable());
        assert!(SlotState::Occupied.is_loadable());
    }

    /// A single non-zero stat is a character, not an empty slot.
    #[test]
    fn one_nonzero_stat_is_not_an_empty_slot() {
        let mut stats = [0i16; STAT_COUNT];
        stats[3] = 7;
        assert_eq!(classify(&stats), SlotState::Occupied);
    }

    /// THE `f` BUG. The terminator must be found on an even offset or the last character is eaten.
    #[test]
    fn a_name_keeps_its_last_character() {
        let mut record = vec![0u8; SLOT_NAME_OFFSET + SLOT_NAME_MAX_BYTES];
        let encoded: Vec<u8> = "Elden Wolf"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        record[SLOT_NAME_OFFSET..SLOT_NAME_OFFSET + encoded.len()].copy_from_slice(&encoded);
        assert_eq!(wide_name(&record, SLOT_NAME_OFFSET), "Elden Wolf");
    }

    /// A name that fills the window with no terminator must still decode, not panic.
    #[test]
    fn an_unterminated_name_is_truncated_rather_than_fatal() {
        let record: Vec<u8> = std::iter::repeat_n(b'A', SLOT_NAME_OFFSET)
            .chain(
                "AB".repeat(SLOT_NAME_MAX_BYTES)
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes),
            )
            .collect();
        let name = wide_name(&record, SLOT_NAME_OFFSET);
        assert_eq!(name.chars().count(), SLOT_NAME_MAX_BYTES / 2);
    }

    #[test]
    fn an_empty_slot_has_no_name() {
        let record = vec![0u8; SLOT_STRIDE];
        assert_eq!(wide_name(&record, SLOT_NAME_OFFSET), "");
    }

    #[test]
    fn stats_add_up_to_what_a_row_can_show() {
        let slot = SaveSlot {
            slot: 0,
            state: SlotState::Occupied,
            name: "Elden Wolf".into(),
            stats: [50, 40, 30, 20, 40, 40, 30, 10, 9],
        };
        assert_eq!(slot.stat_total(), 269);
        assert_eq!(slot.soul_level(), 269 - LEVEL_ONE_STAT_TOTAL);
    }

    /// EVERY DS2 CLASS STARTS AT SOUL LEVEL ONE, which is what makes the level readable without
    /// knowing the class. Nine sixes is a real character out of this machine's save.
    #[test]
    fn a_fresh_character_is_level_one_whatever_its_class() {
        let fresh = SaveSlot {
            slot: 1,
            state: SlotState::Occupied,
            name: "Shit Pit".into(),
            stats: [6; STAT_COUNT],
        };
        assert_eq!(fresh.stat_total(), LEVEL_ONE_STAT_TOTAL + 1);
        assert_eq!(fresh.soul_level(), 1);
    }

    /// The clamp is the game's, and it is what stops an empty slot reporting a negative level.
    #[test]
    fn an_empty_slot_never_reports_a_level_below_one() {
        let empty = SaveSlot {
            slot: 9,
            state: SlotState::Empty,
            name: String::new(),
            stats: [0; STAT_COUNT],
        };
        assert_eq!(empty.soul_level(), 1);
        let blank = SaveSlot {
            stats: [BLANK_STAT; STAT_COUNT],
            ..empty
        };
        assert_eq!(blank.soul_level(), 1);
    }

    #[test]
    fn a_section_chain_that_never_terminates_is_refused_rather_than_looped() {
        // id 0, size 0: stepping over it would advance by nothing.
        let mut plain = vec![0u8; 64];
        plain[4..8].copy_from_slice(&7u32.to_le_bytes());
        assert!(slots_in_payload(&plain).is_none());
    }

    #[test]
    fn ten_records_are_read_when_the_section_is_present() {
        // A synthetic payload: four bytes of lead-in, one matching section header, prologue, then
        // ten records whose first stat is the slot index plus one.
        let body = SLOT_PROLOGUE + SLOT_COUNT * SLOT_STRIDE;
        let mut plain = vec![0u8; 4 + 12 + body];
        plain[4..8].copy_from_slice(&SLOT_SECTION_ID.to_le_bytes());
        plain[12..16].copy_from_slice(&SLOT_SECTION_SIZE.to_le_bytes());
        let base = 4 + 12 + SLOT_PROLOGUE;
        for slot in 0..SLOT_COUNT {
            let at = base + slot * SLOT_STRIDE + SLOT_ATTRS_OFFSET;
            plain[at..at + 2].copy_from_slice(&((slot as i16) + 1).to_le_bytes());
        }
        let found = slots_in_payload(&plain).expect("the section is right there");
        assert_eq!(found.len(), SLOT_COUNT);
        assert_eq!(found[0].stats[0], 1);
        assert_eq!(found[9].slot, 9);
        assert!(found.iter().all(|slot| slot.state == SlotState::Occupied));
    }

    fn header(kind: u32, size: u32) -> Vec<u8> {
        let mut out = vec![0u8; SECTION_HEADER_SIZE];
        out[0..4].copy_from_slice(&kind.to_le_bytes());
        out[8..12].copy_from_slice(&size.to_le_bytes());
        out
    }

    /// The same cases as `scripts/ds2-sl2.py --selftest`, so the two agree on every outcome.
    #[test]
    fn the_walk_agrees_with_the_python_classifier() {
        let good = [
            header(0, 4),
            b"abcd".to_vec(),
            header(5, 0),
            header(SECTION_TYPE_END, 0),
        ]
        .concat();
        // Slot 1 of the save that hard-locked: 111292 zero bytes, type 0 size 0 to the end.
        assert_eq!(section_walk(&vec![0u8; 111_292]), SectionWalk::Hangs);
        assert_eq!(section_walk(&[]), SectionWalk::Hangs);
        assert_eq!(section_walk(&good), SectionWalk::Ends);
        // A size past the end clamps there, and the next read fails for good.
        let past_end = [header(0, 10_000), header(SECTION_TYPE_END, 0)].concat();
        assert_eq!(section_walk(&past_end), SectionWalk::Hangs);
        assert_eq!(section_walk(&header(0x20, 0)), SectionWalk::Bails);
        // The end marker can arrive in a partial read, which copies what is left.
        let short = [header(0, 0), SECTION_TYPE_END.to_le_bytes().to_vec()].concat();
        assert_eq!(section_walk(&short), SectionWalk::Ends);
    }

    #[test]
    fn the_stream_starts_past_the_length_word_and_is_that_long() {
        let good = [header(0, 0), header(SECTION_TYPE_END, 0)].concat();
        let length = u32::try_from(good.len()).expect("small");
        let payload = [length.to_le_bytes().to_vec(), good.clone(), vec![0u8; 4]].concat();
        assert_eq!(entry_stream(&payload), good.as_slice());
        assert_eq!(entry_stream(&[1, 2]), &[] as &[u8]);
    }

    #[test]
    fn a_hollow_slot_is_not_loadable() {
        assert!(!SlotState::Hollow.is_loadable());
    }

    #[test]
    fn not_a_container_is_refused() {
        assert!(matches!(slots(b"not a save"), Err(Sl2Error::NotBnd4)));
    }
}
