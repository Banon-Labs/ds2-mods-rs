//! One press: find a link, say what is happening, fetch what it names.
//!
//! # Where the link comes from, and why it is the clipboard
//!
//! The first design asked STEAM for a text field. That work is kept ([`crate::steam`]) and it is
//! correct -- it obtains `SteamUtils007`, it interlocks against the game's own keyboard, and on a
//! Steam Deck or in Big Picture mode it draws a prefilled field. **On a desktop Steam it does not**,
//! and that is measured rather than assumed: nine presses on the game thread, `overlay=true`, and
//! `ShowGamepadTextInput` returned `false` every time. It raises the BIG PICTURE dialog, which a
//! desktop Steam outside Big Picture does not have. The game itself never built its own keyboard
//! either (`game-keyboard-built=false`), which is why DARK SOULS II ships a `SimpleEditBox`
//! fallback for exactly this case.
//!
//! So the link is read off the CLIPBOARD, which is the half of "copy and paste" that works
//! everywhere: copy a link in a browser, press the row. Steam is still tried first, so the field
//! appears for anyone who has it, and the clipboard is what happens when it declines.
//!
//! # What the row says while it works
//!
//! A row that fetches over the network and then says nothing is indistinguishable from a row that
//! did nothing. Every step rewrites the row's own caption through
//! [`ds2_menu_row::set_row_caption`], and `ds2-menu-row`'s per-frame tick puts it on screen without
//! the player closing the menu.

use ds2_build_import_core::{
    BUILD_HOST, BUILD_URL_PREFIX, UrlRejection, build_id_from_url, build_path,
};

use crate::steam::{KeyboardClaim, SteamError, SteamUtils};
use crate::{LOG_PREFIX, log_line};

/// What Steam draws above the field, where Steam draws a field at all.
const FIELD_DESCRIPTION: &[u8] = c"Paste a soulsplanner.com build link".to_bytes_with_nul();

/// The longest text the Steam field will accept, including the terminator.
const FIELD_CHAR_MAX: u32 = 256;

/// How long to wait for the player before giving up on a Steam session.
const SESSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// How often the worker looks at the game's keyboard state. One poll per frame at 60Hz, near enough.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(16);

/// What the user agent says. The site does not care; a log on the other end might.
const USER_AGENT: &str = "ds2-mods-rs";

/// The row's caption when nothing is happening. Restored after a result has been read.
pub(crate) const IDLE_CAPTION: &str = ds2_build_import_core::ROW_CAPTION;

/// Say something on the row, now.
///
/// # Safety
///
/// Only sound on the GAME thread -- it pushes straight into the scene. Off-thread callers must use
/// [`say`] and let the per-frame tick pick it up.
unsafe fn say_now(text: &str) {
    ds2_menu_row::set_row_caption(crate::row_id(), text);
    // SAFETY: forwarded to the caller, who is promising the game thread with the menu up.
    unsafe { ds2_menu_row::refresh_row_captions() };
}

/// Say something on the row, from any thread. Appears on the next frame the menu is drawn.
fn say(text: &str) {
    ds2_menu_row::set_row_caption(crate::row_id(), text);
}

/// Where a link came from, for the log.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Source {
    /// Steam drew a field and the player typed into it.
    SteamField,
    /// The player had copied a link.
    Clipboard,
}

impl Source {
    const fn describe(self) -> &'static str {
        match self {
            Source::SteamField => "the Steam field",
            Source::Clipboard => "the clipboard",
        }
    }
}

/// Handle the press. **Called on the GAME thread**, from the row's confirm, menu still up.
///
/// Returns the worker's job when there is one. Everything that can be decided immediately is
/// decided here, so the player gets an answer on the same frame they pressed.
pub(crate) fn begin_session() -> Option<Job> {
    // SAFETY: the confirm path -- game thread, menu up.
    unsafe { say_now("Reading link...") };

    // A GAME MUST BE IN PROGRESS. The row lives on the pause menu, so one nearly always is, but
    // "nearly always" is not a check and the fetch is pointless without a character to aim it at.
    match crate::save::require_live_character() {
        Ok(name) => log_line(format_args!("{LOG_PREFIX} character \"{name}\" is loaded")),
        Err(reason) => {
            log_line(format_args!("{LOG_PREFIX} refused: {reason}"));
            // SAFETY: still the confirm path.
            unsafe { say_now(reason.caption()) };
            return None;
        }
    }

    match steam_field() {
        Ok(session) => return Some(Job::SteamField(session)),
        Err(error) => log_line(format_args!(
            "{LOG_PREFIX} no Steam field ({error}, game-keyboard-built={}) -- reading the \
             clipboard instead",
            crate::steam::game_keyboard_built()
        )),
    }

    let Some(text) = crate::clipboard::text() else {
        log_line(format_args!(
            "{LOG_PREFIX} refused: the clipboard holds no text"
        ));
        // SAFETY: still the confirm path.
        unsafe { say_now("Copy a build link first") };
        return None;
    };
    Some(Job::Link {
        text,
        source: Source::Clipboard,
    })
}

/// What the worker has to finish.
pub(crate) enum Job {
    /// Steam drew a field; wait for the player, then act on what they left.
    SteamField(SteamSession),
    /// A link is already in hand.
    Link { text: String, source: Source },
}

/// A Steam field that is open and waiting.
pub(crate) struct SteamSession {
    utils: SteamUtils,
    claim: KeyboardClaim,
}

/// Try to put a prefilled Steam field on screen.
fn steam_field() -> Result<SteamSession, SteamError> {
    let utils = SteamUtils::with_prefill_support()?;
    if !utils.overlay_enabled() {
        return Err(SteamError::OverlayDisabled);
    }
    let claim = KeyboardClaim::acquire()?;
    let prefill = {
        let mut bytes = BUILD_URL_PREFIX.as_bytes().to_vec();
        bytes.push(0);
        bytes
    };
    // `claim` drops on the error path and restores the game's keyboard state, which is the whole
    // reason it is a guard rather than a pair of writes.
    utils.show(FIELD_DESCRIPTION, &prefill, FIELD_CHAR_MAX)?;
    log_line(format_args!(
        "{LOG_PREFIX} field open, prefilled \"{BUILD_URL_PREFIX}\""
    ));
    Ok(SteamSession { utils, claim })
}

/// Finish the press. Called on a worker thread, because both halves can block for a long time.
pub(crate) fn finish_session(job: Job) {
    let (text, source) = match job {
        Job::Link { text, source } => (text, source),
        Job::SteamField(session) => {
            say("Waiting for the field...");
            let Some(state) = wait_for_dismissal(&session.claim) else {
                say(IDLE_CAPTION);
                return log_line(format_args!(
                    "{LOG_PREFIX} gave up after {}s -- the field was never dismissed",
                    SESSION_TIMEOUT.as_secs()
                ));
            };
            if state == ds2_rva::SOFTWARE_KEYBOARD_STATE_CANCELLED {
                say(IDLE_CAPTION);
                return log_line(format_args!("{LOG_PREFIX} cancelled"));
            }
            let text = session.utils.entered_text();
            // The claim has done its job; give the game its keyboard back BEFORE the network work,
            // so a slow fetch cannot keep character naming broken.
            drop(session.claim);
            let Some(text) = text else {
                say("The field could not be read");
                return log_line(format_args!(
                    "{LOG_PREFIX} submitted, but the text could not be read back"
                ));
            };
            (text, Source::SteamField)
        }
    };
    load(&text, source);
}

/// Poll the game's own state field until its listener says the player is done.
fn wait_for_dismissal(claim: &KeyboardClaim) -> Option<i32> {
    let deadline = std::time::Instant::now() + SESSION_TIMEOUT;
    loop {
        if let Some(state) = claim.finished_state() {
            return Some(state);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Validate the link, fetch the page, and say what it holds.
fn load(text: &str, source: Source) {
    let build_id = match build_id_from_url(text) {
        Ok(id) => id,
        Err(rejection) => {
            log_line(format_args!(
                "{LOG_PREFIX} rejected \"{text}\" from {}: {rejection}",
                source.describe()
            ));
            // THE REJECTION'S OWN WORDS, not a generic failure. Each variant of `UrlRejection` is a
            // different thing the player can do about it, and "that did not work" is the one
            // message that helps with none of them.
            say(short_rejection(rejection));
            return;
        }
    };
    log_line(format_args!(
        "{LOG_PREFIX} fetching build {build_id} from {}",
        source.describe()
    ));
    say(&format!("Fetching build {build_id}..."));

    let page = match ds2_game_base::http::get(BUILD_HOST, &build_path(build_id), USER_AGENT) {
        Ok(page) => page,
        Err(error) => {
            log_line(format_args!("{LOG_PREFIX} fetch failed: {error:?}"));
            say("Could not reach soulsplanner");
            return;
        }
    };
    match ds2_build_import_core::saved_build::parse(&page, build_id) {
        Ok(build) => {
            log_line(format_args!(
                "{LOG_PREFIX} build {} loaded: {} / {} / {} armour, {} rings, {} spells",
                build.id,
                build.class,
                build.covenant,
                build.armor.len(),
                build.rings.len(),
                build.spells.len()
            ));
            let stats = build
                .stats
                .each()
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join(" ");
            log_line(format_args!(
                "{LOG_PREFIX} build {} stats: {stats}",
                build.id
            ));
            match crate::save::record(&build) {
                Ok(path) => {
                    log_line(format_args!("{LOG_PREFIX} wrote {}", path.display()));
                    say(&format!("{}: {}", build.id, build.class));
                }
                Err(error) => {
                    log_line(format_args!(
                        "{LOG_PREFIX} could not record the build: {error}"
                    ));
                    say(&format!("Read {} but could not save it", build.id));
                }
            }
        }
        Err(error) => {
            log_line(format_args!(
                "{LOG_PREFIX} build {build_id} could not be read: {error}"
            ));
            say(&format!("Build {build_id} is not readable"));
        }
    }
}

/// A rejection, short enough for a caption box `274` units wide.
///
/// [`UrlRejection::indicator`] is written for a log line and a wider field; the row is narrow, and
/// a caption that runs off the end says less than a shorter one that fits.
const fn short_rejection(rejection: UrlRejection) -> &'static str {
    match rejection {
        UrlRejection::Empty => "No build id in that link",
        UrlRejection::NotSoulsplanner => "Not a soulsplanner link",
        UrlRejection::FragmentForm => "Drop the # from the link",
        UrlRejection::IdNotNumeric => "Build id must be digits",
        UrlRejection::IdTooLarge => "That build id is too big",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything handed to Steam as a `char*` ends in a NUL, including the prefill built at
    /// runtime.
    #[test]
    fn every_string_handed_to_steam_is_terminated() {
        assert_eq!(FIELD_DESCRIPTION.last(), Some(&0));
        let mut prefill = BUILD_URL_PREFIX.as_bytes().to_vec();
        prefill.push(0);
        assert_eq!(prefill.last(), Some(&0));
        assert!(!BUILD_URL_PREFIX.as_bytes().contains(&0));
    }

    /// The field is long enough for the link it exists to collect, with room to type.
    #[test]
    fn the_field_holds_more_than_the_prefill() {
        assert!(FIELD_CHAR_MAX as usize > BUILD_URL_PREFIX.len() + 16);
        assert!(FIELD_CHAR_MAX as usize <= ds2_build_import_core::MAX_UNITS);
    }

    /// The wait is fast enough to feel immediate and long enough not to abandon someone typing.
    #[test]
    fn the_wait_is_bounded_at_both_ends() {
        assert!(POLL_INTERVAL.as_millis() <= 33, "slower than two frames");
        assert!(SESSION_TIMEOUT.as_secs() >= 300, "a person types slowly");
    }

    /// Every caption fits the row, and every rejection has its own words.
    ///
    /// The width is the shipped caption box, `ds2_rva::FLO_CAPTION_BOX` -- 274 units at font size
    /// 22. That is not a character count, so this bounds the CHARACTERS well inside it rather than
    /// pretending to measure the font.
    #[test]
    fn every_caption_is_short_enough_to_read() {
        let all = [
            UrlRejection::Empty,
            UrlRejection::NotSoulsplanner,
            UrlRejection::FragmentForm,
            UrlRejection::IdNotNumeric,
            UrlRejection::IdTooLarge,
        ];
        for (index, one) in all.iter().enumerate() {
            assert!(short_rejection(*one).len() <= 30, "{one:?}");
            for other in &all[index + 1..] {
                assert_ne!(short_rejection(*one), short_rejection(*other));
            }
        }
        for caption in [
            IDLE_CAPTION,
            "Reading link...",
            "Copy a build link first",
            "Waiting for the field...",
            "Could not reach soulsplanner",
        ] {
            assert!(caption.len() <= 30, "{caption}");
        }
    }
}
