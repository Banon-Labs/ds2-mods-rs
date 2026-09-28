//! Log every music event DARK SOULS II starts, pauses and stops, with its FMOD handle, and the map
//! index the player is in when it starts.
//!
//! Step 1 of the region music playlist in `docs/DS2-REGION-MUSIC-PLAYLIST.md`. It changes no sound:
//! every detour calls the original with the arguments it was given and returns what it returned.
//!
//! # How
//!
//! The game calls FMOD's event API through `jmp [rip+IAT]` thunks and never through a direct
//! `call [IAT]`, so replacing the import slot catches every call. The slots are pointer writes in
//! `.idata`, so no Arxan-checked `.text` is patched. Four are fronted: `Event::start`,
//! `Event::stop`, `Event::setPaused`, and `Event::getState`. The last is only a clock: the game
//! calls these on its sound thread, so a detour there is a place to sample the live music event's
//! channel position on that same thread without a thread of our own calling into FMOD.
//!
//! Every other FMOD function this calls is read out of the game's own import slot (the pointer the
//! Windows loader resolved) or, for the four the game never imports, looked up in `fmodex64.dll` by
//! its exported name.

/// What every line this crate writes begins with, so its lines can be grepped out of the shared log.
pub const LOG_PREFIX: &str = "ds2-music-probe:";

/// How many events of any name to log at the start of a run, before only music is logged.
///
/// The Frida run behind this crate saw bare sound ids (`s000000157`, `f100000006`) but no music
/// event, because the music had started before it attached. Logging the first few events whatever
/// they are is what would show a different name format for music instead of an empty log.
pub const LOG_FIRST_ANY_EVENTS: u32 = 16;

/// Milliseconds between two position samples of the live music event.
pub const SAMPLE_EVERY_MS: u64 = 5_000;

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

#[cfg(windows)]
mod install;

#[cfg(windows)]
pub use install::{LogFn, Outcome, install, set_logger};

#[cfg(test)]
mod tests {
    use super::is_music_name;

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
}
