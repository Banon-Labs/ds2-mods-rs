//! The region music player: see which track is playing, seek it, stop it repeating, and give each
//! region a playlist of any music the game ships, kept in a file beside the game.
//!
//! `docs/DS2-REGION-MUSIC-PLAYLIST.md` is the design and the evidence behind it.
//!
//! # How
//!
//! The game calls FMOD's event API through `jmp [rip+IAT]` thunks and never through a direct
//! `call [IAT]`, so replacing an import slot catches every call. The slots are pointer writes in
//! `.idata`, so no Arxan-checked `.text` is patched. Four are fronted: `Event::start`,
//! `Event::stop`, `Event::setPaused` and `Event::getState`. The game makes all of them on its sound
//! thread, and `getState` many times a second, so the player does all of its own FMOD work inside
//! that call, on that thread, the way the game does; the panel only queues commands.
//!
//! # The model
//!
//! A region is the looping music event the game starts there (`m100400001` in Majula), keyed by its
//! bank and name ([`playlist::TrackKey`]). A region with no entry in the file, or one that says
//! nothing different from the game, is left alone. Otherwise the game's own event is muted and the
//! player starts its own instance of each playlist track in turn, and follows the game's instance:
//! the game pausing or stopping its track (a boss, a warp, a cutscene) pauses or stops ours, so the
//! game's own stops are never fought. When the default track is the one playing, the game's own
//! event plays it, unmuted.
//!
//! Repeat off is `Channel::setLoopCount(0)`: the track plays to its end and its channel is gone,
//! which is what the player waits for before moving to the next track.

/// What every line this crate writes begins with, so its lines can be grepped out of the shared log.
pub const LOG_PREFIX: &str = "ds2-music-probe:";

/// How many events of any name to log at the start of a run, before only music is logged.
///
/// Music is named by bare sound id (`m100400001`); logging the first few events whatever they are
/// is what would show a different name format for music instead of an empty log.
pub const LOG_FIRST_ANY_EVENTS: u32 = 16;

/// Milliseconds between two position lines for the track that is playing.
pub const SAMPLE_EVERY_MS: u64 = 5_000;

/// The playlists file, beside `DarkSoulsII.exe`. Not `ds2-mods.toml`, which `scripts/ds2-run.py`
/// rewrites on every launch.
pub const PLAYLIST_FILE_NAME: &str = "ds2-music-playlist.toml";

/// The key that opens the Music panel when the config names none.
pub const DEFAULT_KEY: &str = "F10";

pub mod playlist;

/// Whether an FMOD event name is a music cue.
///
/// Sound ids are a type letter and nine digits (`"%c%09d"` at `0x1411d78c0`), and music is type
/// letter `m`. A few named music events (`m_death`, `m_training`) are `m_` plus a word. The name may
/// arrive with a group path in front of it, so the last `/`-separated part is what is tested.
pub fn is_music_name(name: &str) -> bool {
    let leaf = name.rsplit('/').next().unwrap_or(name);
    let bytes = leaf.as_bytes();
    if bytes.first() != Some(&b'm') {
        return false;
    }
    match bytes.get(1) {
        Some(b'_') => bytes.len() > 2,
        Some(_) => bytes.len() >= 10 && bytes[1..10].iter().all(u8::is_ascii_digit),
        None => false,
    }
}

/// `m:ss` for a millisecond count.
pub fn minutes_seconds(ms: u32) -> String {
    let seconds = ms / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(windows)]
mod engine;
#[cfg(windows)]
mod fmod;
#[cfg(windows)]
mod install;
#[cfg(windows)]
mod panel;
#[cfg(windows)]
mod store;

#[cfg(windows)]
pub use install::{LogFn, Outcome, install, set_logger};
#[cfg(windows)]
pub use panel::install_panel;

#[cfg(test)]
mod tests {
    use super::{is_music_name, minutes_seconds};

    #[test]
    fn music_ids_and_named_music_pass() {
        assert!(is_music_name("m100400001"));
        assert!(is_music_name("m202100004_andeal"));
        assert!(is_music_name("m_death"));
        assert!(is_music_name("frpg2_sm/m1004/m100400001"));
    }

    #[test]
    fn other_sound_types_do_not() {
        assert!(!is_music_name("s000000157"));
        assert!(!is_music_name("f100000006"));
        assert!(!is_music_name("m"));
        assert!(!is_music_name("m_"));
        assert!(!is_music_name("m1004"));
        assert!(!is_music_name("mute"));
        assert!(!is_music_name(""));
    }

    #[test]
    fn minutes_and_seconds() {
        assert_eq!(minutes_seconds(198_299), "3:18");
        assert_eq!(minutes_seconds(5_013), "0:05");
    }
}
