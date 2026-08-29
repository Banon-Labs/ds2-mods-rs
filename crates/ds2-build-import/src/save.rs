//! Is there a character to aim a build at, and where does the build get written.
//!
//! # Two checks, answering two different questions
//!
//! **Is a character loaded right now** is answered out of the game's own memory:
//! `GameManagerImp -> GameDataManager -> player_data`, and the name at
//! [`ds2_rva::PLAYER_DATA_NAME_OFFSET`]. That field is the right one to test because its CLEARING
//! site is known as well as its writing site -- the delete-data-list flow puts `L""` there -- so an
//! empty name means "no character" rather than "not populated yet". The neighbouring
//! [`ds2_rva::PLAYER_DATA_JOURNEY_OFFSET`] is deliberately NOT used as the test -- and it is worth
//! knowing why, because this crate once called it a "profile loaded" flag: it is the NEW GAME
//! cycle, `1` meaning Journey 1. One literal `1` on load, and nothing writing it back, reads like a
//! sticky boolean right up until someone reaches NG+.
//!
//! **Is there a save to write beside** is answered off disk, with [`ds2_sl2_core::validate`]. It is
//! a structural check -- BND4 magic and an entry table in bounds -- not a decryption, which is all
//! that is needed to tell a real save from a zero-byte file, a half-written one or a wrong folder.
//!
//! The live check runs first: it is far cheaper, and the file check reads eight megabytes.
//!
//! # What is still not knowable here
//!
//! Stats, soul level, soul memory, equipment and inventory are **not mapped in this repo at all**.
//! `PlayerGameData` and `GameDataPlayerInfo` appear in RTTI only inside `FeFunctorJob<...>` template
//! names -- they are non-polymorphic, so there is no vtable to find them by. Applying a build,
//! rather than recording one, waits on that excavation.
//!
//! # Where the build is written
//!
//! Beside the save, as `ds2-build-<id>.json`. Beside the SAVE rather than beside the game, because
//! a build is about a character and the save folder is per-Steam-account -- and because
//! `ds2-save-redirect` exists so a player can point the game at a different save folder, and a
//! build filed next to the wrong one is a mistake nobody would notice.

use std::path::{Path, PathBuf};

use ds2_build_import_core::Build;

/// The save the game writes, under `%APPDATA%`.
const SAVE_NAME: &str = "DS2SOFS0000.sl2";
/// The folder it lives in, with a per-account folder between.
const SAVE_FOLDER: &str = "DarkSoulsII";

/// Why a press was refused before anything was fetched.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Refusal {
    /// `%APPDATA%` could not be read. Not a Wine prefix this understands.
    NoAppData,
    /// No `DS2SOFS0000.sl2` under `%APPDATA%\DarkSoulsII`.
    NoSave,
    /// The save is there and is not a save.
    SaveUnreadable,
    /// The save is structurally a container and holds no character.
    SaveEmpty,
    /// The game has no character loaded -- the title screen, or a profile that was deleted.
    ///
    /// Judged from `player_data` being null. **NOT from the name being empty**, which is also true
    /// of every blank character in a mule save.
    NoCharacter,
}

impl Refusal {
    /// What the row says. Short, because the caption box is narrow.
    pub(crate) const fn caption(self) -> &'static str {
        match self {
            Refusal::NoAppData => "Cannot find the save folder",
            Refusal::NoSave => "No save file found",
            Refusal::SaveUnreadable => "The save is not readable",
            Refusal::SaveEmpty => "The save holds no character",
            Refusal::NoCharacter => "No character is loaded",
        }
    }
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let detail = match self {
            Refusal::NoAppData => "%APPDATA% is unset or unreadable",
            Refusal::NoSave => "no DS2SOFS0000.sl2 under %APPDATA%/DarkSoulsII",
            Refusal::SaveUnreadable => "the save is not a BND4 container",
            Refusal::SaveEmpty => "the save's entry table is empty",
            Refusal::NoCharacter => "player_data is null -- no character is loaded",
        };
        f.write_str(detail)
    }
}

/// The active save file, if one can be found.
///
/// `%APPDATA%/DarkSoulsII/<steam id>/DS2SOFS0000.sl2`. The account folder is a hex Steam ID that
/// cannot be known in advance, so the folder is SCANNED rather than composed -- and the first match
/// wins, which is right for the overwhelmingly common one-account prefix and merely arbitrary for
/// the rest.
pub(crate) fn locate() -> Result<PathBuf, Refusal> {
    let app_data = std::env::var_os("APPDATA").ok_or(Refusal::NoAppData)?;
    let root = Path::new(&app_data).join(SAVE_FOLDER);
    let entries = std::fs::read_dir(&root).map_err(|_| Refusal::NoSave)?;
    for entry in entries.flatten() {
        let candidate = entry.path().join(SAVE_NAME);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(Refusal::NoSave)
}

/// The `player_data` block for the character the game currently has loaded.
///
/// `GameManagerImp -> GameDataManager -> player_data`, every hop null-checked and read through the
/// fault-safe readers, because this runs while the player is standing in the pause menu and a wrong
/// pointer here would be their crash.
///
/// **THIS, not the name, is what "a character is loaded" means.** See [`require_live_character`].
pub(crate) fn live_player_data() -> Option<usize> {
    let address = ds2_game_base::mem::game_rva(ds2_rva::GAME_MANAGER_IMP).ok()?;
    // SAFETY: a resolved RVA in the loaded image, then three pointer hops each checked for null and
    // each read through `safe_read_usize`, which reports an unmapped page instead of faulting.
    unsafe {
        let manager = non_null(ds2_game_base::mem::safe_read_usize(address)?)?;
        let data = non_null(ds2_game_base::mem::safe_read_usize(
            manager + ds2_rva::GAME_DATA_MANAGER_OFFSET,
        )?)?;
        non_null(ds2_game_base::mem::safe_read_usize(
            data + ds2_rva::GAME_DATA_PLAYER_DATA_OFFSET,
        )?)
    }
}

/// The name of the character the game currently has loaded, if it has one.
///
/// `player_data + 0x24`, a `wchar_t[0x20]` the profile loader fills from the save slot record.
///
/// # An empty name does NOT mean there is no character
///
/// It reads empty in two completely different situations, and an earlier version of this treated
/// them as one. The delete-data-list flow writes `L""` here, so empty means "deleted" for a profile
/// that once had a name -- which is what the old reasoning was built on, and it is correct as far
/// as it goes. But a BLANK CHARACTER, the all-ones-stats kind a mule save is full of, has never
/// been named and reads empty too, and the game loads it perfectly well. `scripts/ds2-sl2.py`
/// already learned this from the other end: it once classified that shape as "not a character" and
/// the runtime disagreed.
///
/// So this returns `Option` and says nothing about liveness. Ask [`live_player_data`] that.
pub(crate) fn live_character_name() -> Option<String> {
    let player = live_player_data()?;
    let field = player + ds2_rva::PLAYER_DATA_NAME_OFFSET;
    let mut units = Vec::with_capacity(ds2_rva::PLAYER_DATA_NAME_UNITS);
    for index in 0..ds2_rva::PLAYER_DATA_NAME_UNITS {
        // SAFETY: inside the block the pointer chain produced; the read is fault-safe.
        match unsafe { ds2_game_base::mem::safe_read_u16(field + index * 2) } {
            Some(0) | None => break,
            Some(unit) => units.push(unit),
        }
    }
    if units.is_empty() {
        return None;
    }
    String::from_utf16(&units).ok()
}

/// `Some(pointer)` unless it is null.
const fn non_null(pointer: usize) -> Option<usize> {
    if pointer == 0 { None } else { Some(pointer) }
}

/// Refuse the press unless there is a character worth acting on.
///
/// TWO CHECKS, AND THEY ANSWER DIFFERENT QUESTIONS. The live one -- is a character loaded RIGHT NOW
/// -- is the one that matters, and it is read out of the game's own memory. The file one is kept
/// because a build is written next to the save, so a run that passes the live check and has no
/// readable save on disk would fetch happily and then have nowhere to put the result.
///
/// The live check comes FIRST because it is the cheaper of the two and the likelier to fail: the
/// file check reads eight megabytes off disk.
pub(crate) fn require_live_character() -> Result<Option<String>, Refusal> {
    // LIVENESS IS THE POINTER CHAIN, NOT THE NAME. This used to refuse whenever the name read
    // empty, which rejected every blank character in a mule save -- characters the game had loaded
    // and was happily standing in. The name is returned for the log and decides nothing.
    live_player_data().ok_or(Refusal::NoCharacter)?;
    let name = live_character_name();
    let path = locate()?;
    let bytes = std::fs::read(&path).map_err(|_| Refusal::SaveUnreadable)?;
    match ds2_sl2_core::validate(&bytes) {
        Ok(0) => Err(Refusal::SaveEmpty),
        Ok(_) => Ok(name),
        Err(_) => Err(Refusal::SaveUnreadable),
    }
}

/// Write a fetched build beside the save. Returns where it went.
pub(crate) fn record(build: &Build) -> Result<PathBuf, String> {
    let save = locate().map_err(|refusal| refusal.to_string())?;
    let folder = save.parent().ok_or("the save has no parent folder")?;
    let path = folder.join(format!("ds2-build-{}.json", build.id));
    std::fs::write(&path, json(build)).map_err(|error| error.to_string())?;
    Ok(path)
}

/// The build as JSON, hand-written.
///
/// No serializer, for the same reason there is no JSON PARSER in `ds2-build-import-core`: this is
/// one flat object with two value kinds, and a dependency for that is a dependency the whole
/// workspace then carries into a Windows DLL.
fn json(build: &Build) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str("{\n");
    out.push_str(&format!("  \"id\": {},\n", build.id));
    out.push_str(&format!("  \"class\": {},\n", quote(&build.class)));
    out.push_str(&format!("  \"gender\": {},\n", build.gender));
    out.push_str(&format!("  \"covenant\": {},\n", quote(&build.covenant)));
    out.push_str(&format!("  \"grip\": {},\n", build.grip));
    for (name, list) in [
        ("armor", &build.armor),
        ("weapons", &build.weapons),
        ("rings", &build.rings),
        ("spells", &build.spells),
        ("items", &build.items),
    ] {
        let joined = list
            .iter()
            .map(|item| quote(item))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("  \"{name}\": [{joined}],\n"));
    }
    let stats = build
        .stats
        .each()
        .iter()
        .map(|(name, value)| format!("    \"{name}\": {value}"))
        .collect::<Vec<_>>()
        .join(",\n");
    out.push_str(&format!("  \"stats\": {{\n{stats}\n  }}\n"));
    out.push_str("}\n");
    out
}

/// A JSON string literal.
///
/// The planner's names are barewords with underscores, so nothing here is expected to fire -- which
/// is exactly why it is here. An escaper that never runs is an escaper nobody notices is missing,
/// until the site allows a quote in an item name and the file stops parsing.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every refusal says a different thing, in the log and on the row.
    #[test]
    fn each_refusal_reads_differently() {
        let all = [
            Refusal::NoAppData,
            Refusal::NoSave,
            Refusal::SaveUnreadable,
            Refusal::SaveEmpty,
            Refusal::NoCharacter,
        ];
        for (index, one) in all.iter().enumerate() {
            assert!(one.caption().len() <= 30, "{one:?} will not fit the row");
            for other in &all[index + 1..] {
                assert_ne!(one.caption(), other.caption());
                assert_ne!(one.to_string(), other.to_string());
            }
        }
    }

    /// A build turns into JSON that is well formed at the edges.
    #[test]
    fn the_recorded_build_is_json_shaped() {
        let build = Build {
            id: 253,
            class: "swordsman".to_owned(),
            covenant: "Brotherhood_of_Blood".to_owned(),
            armor: vec!["Black_Hood".to_owned()],
            ..Build::default()
        };
        let text = json(&build);
        assert!(text.starts_with("{\n") && text.ends_with("}\n"));
        assert!(text.contains("\"id\": 253"));
        assert!(text.contains("\"class\": \"swordsman\""));
        assert!(text.contains("\"armor\": [\"Black_Hood\"]"));
        assert!(text.contains("\"vigor\": 0"));
        // Braces balance, which is the cheapest whole-document check there is without a parser.
        assert_eq!(
            text.chars().filter(|c| *c == '{').count(),
            text.chars().filter(|c| *c == '}').count()
        );
    }

    /// A name carrying JSON's own punctuation does not produce invalid JSON.
    #[test]
    fn a_quote_in_a_name_is_escaped() {
        assert_eq!(quote("plain"), "\"plain\"");
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote("a\\b"), "\"a\\\\b\"");
        assert_eq!(quote("a\nb"), "\"a\\nb\"");
        // Anything else below space becomes a `\u` escape rather than a raw control byte.
        assert_eq!(quote("a\u{1}b"), "\"a\\u0001b\"");
    }
}
