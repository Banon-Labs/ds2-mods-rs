//! One session: open the field, wait for the player, fetch what they asked for.
//!
//! # Why this runs on a worker thread
//!
//! [`ds2_menu_row::RowSpec::on_confirm`] is called on the GAME thread, inside the menu's confirm
//! path, with the pause menu still up. Two things in this flow must not happen there. The field is
//! open for as long as the player is typing, which is unbounded; and the fetch is a blocking TLS
//! handshake, which `ds2_game_base::http` documents as "must not be called from a frame callback or
//! any other hook that runs on the game thread, where it would stall the game for the duration".
//!
//! So the confirm hands off immediately and the whole session lives on a worker.
//!
//! **The one unverified assumption in this crate is here.** `ShowGamepadTextInput` and
//! `GetEnteredGamepadTextInput` are called from that worker rather than from the game thread the
//! game itself uses. The Steamworks API is documented as thread-safe and the interlock this crate
//! holds is an aligned dword either way, but "documented" is not "measured on this build under this
//! Proton", and no amount of reading `DarkSoulsII.exe` settles it. If it turns out to matter, the
//! fix is a per-frame hook at `FeGroupInGameTopSelect::v2` (RVA `0x000a5dd0`, a clean five-byte
//! prologue) to pump the poll from the game thread instead. That is a change to WHERE these four
//! calls happen, not to what they do.

use ds2_build_import_core::{BUILD_HOST, BUILD_URL_PREFIX, build_id_from_url, build_path};

use crate::steam::{KeyboardClaim, SteamError, SteamUtils};
use crate::{LOG_PREFIX, log_line};

/// What Steam draws above the field.
const FIELD_DESCRIPTION: &[u8] = c"Paste a soulsplanner.com build link".to_bytes_with_nul();

/// The longest text the field will accept, including the terminator.
const FIELD_CHAR_MAX: u32 = 256;

/// How long to wait for the player before giving up on a session.
///
/// Generous on purpose: the cost of being wrong in the short direction is abandoning a field the
/// player is still typing into, and then restoring `m_state` underneath a live overlay. The cost of
/// being wrong in the long direction is one sleeping thread.
const SESSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// How often the worker looks at the state field. One poll per frame at 60Hz, near enough.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(16);

/// What the user agent says. The site does not care; a log on the other end might.
const USER_AGENT: &str = "ds2-mods-rs";

/// Run one session to completion. Called on a worker thread.
pub(crate) fn run_session() {
    let utils = match SteamUtils::with_prefill_support() {
        Ok(utils) => utils,
        Err(error) => return log_line(format_args!("{LOG_PREFIX} no field: {error}")),
    };

    // ASKED BEFORE CLAIMING ANYTHING. If the overlay is off there is nothing to put back, and this
    // is the one failure the executable could never have predicted -- it is a property of the
    // running Steam client.
    if !utils.overlay_enabled() {
        return log_line(format_args!(
            "{LOG_PREFIX} no field: {}",
            SteamError::OverlayDisabled
        ));
    }

    let claim = match KeyboardClaim::acquire() {
        Ok(claim) => claim,
        Err(error) => return log_line(format_args!("{LOG_PREFIX} no field: {error}")),
    };

    let prefill = {
        let mut bytes = BUILD_URL_PREFIX.as_bytes().to_vec();
        bytes.push(0);
        bytes
    };
    if let Err(error) = utils.show(FIELD_DESCRIPTION, &prefill, FIELD_CHAR_MAX) {
        // `claim` drops here and restores the state -- which is the whole reason it is a guard.
        return log_line(format_args!("{LOG_PREFIX} no field: {error}"));
    }
    log_line(format_args!(
        "{LOG_PREFIX} field open, prefilled \"{BUILD_URL_PREFIX}\""
    ));

    let Some(state) = wait_for_dismissal(&claim) else {
        return log_line(format_args!(
            "{LOG_PREFIX} gave up after {}s -- the field was never dismissed",
            SESSION_TIMEOUT.as_secs()
        ));
    };
    if state == ds2_rva::SOFTWARE_KEYBOARD_STATE_CANCELLED {
        return log_line(format_args!("{LOG_PREFIX} cancelled"));
    }

    let text = utils.entered_text();
    // The claim has done its job; put the game's keyboard back before the network work starts, so a
    // slow fetch cannot keep character naming broken.
    drop(claim);

    let Some(text) = text else {
        return log_line(format_args!(
            "{LOG_PREFIX} submitted, but the text could not be read back"
        ));
    };
    load(&text);
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
fn load(text: &str) {
    let build_id = match build_id_from_url(text) {
        Ok(id) => id,
        Err(rejection) => {
            return log_line(format_args!(
                "{LOG_PREFIX} rejected \"{text}\": {rejection}"
            ));
        }
    };
    log_line(format_args!("{LOG_PREFIX} fetching build {build_id}"));

    let page = match ds2_game_base::http::get(BUILD_HOST, &build_path(build_id), USER_AGENT) {
        Ok(page) => page,
        Err(error) => {
            return log_line(format_args!("{LOG_PREFIX} fetch failed: {error:?}"));
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
            // NOT APPLIED, AND SAYING SO RATHER THAN LEAVING IT AMBIGUOUS. Putting this on a
            // character means writing equipment, inventory and stats, none of which is mapped for
            // DARK SOULS II yet -- see `ds2-mods-rs-a1g` for the Ghidra work that comes first.
            log_line(format_args!(
                "{LOG_PREFIX} the build was read, NOT applied -- applying it needs the equip and \
                 stat writers, which do not exist yet"
            ));
        }
        Err(error) => log_line(format_args!(
            "{LOG_PREFIX} build {build_id} could not be read: {error}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything handed to Steam as a `char*` ends in a NUL, including the prefill built at
    /// runtime.
    ///
    /// Steam reads until the terminator. A prefill assembled from a Rust `&str` has none unless one
    /// is pushed, and the failure would not be a crash -- it would be a field prefilled with this
    /// DLL's `.rodata` until the first zero byte.
    #[test]
    fn every_string_handed_to_steam_is_terminated() {
        assert_eq!(FIELD_DESCRIPTION.last(), Some(&0));
        let mut prefill = BUILD_URL_PREFIX.as_bytes().to_vec();
        prefill.push(0);
        assert_eq!(prefill.last(), Some(&0));
        assert_eq!(&prefill[..prefill.len() - 1], BUILD_URL_PREFIX.as_bytes());
        // And no interior NUL, or the field would open with a truncated link.
        assert!(!BUILD_URL_PREFIX.as_bytes().contains(&0));
    }

    /// The field is long enough for the link it exists to collect, with room to type.
    #[test]
    fn the_field_holds_more_than_the_prefill() {
        assert!(FIELD_CHAR_MAX as usize > BUILD_URL_PREFIX.len() + 16);
        // And it does not promise more than the core's own editor would keep.
        assert!(FIELD_CHAR_MAX as usize <= ds2_build_import_core::MAX_UNITS);
    }

    /// The poll is fast enough to feel immediate and the timeout is long enough not to abandon
    /// someone mid-sentence.
    #[test]
    fn the_wait_is_bounded_at_both_ends() {
        assert!(POLL_INTERVAL.as_millis() <= 33, "slower than two frames");
        assert!(SESSION_TIMEOUT.as_secs() >= 300, "a person types slowly");
    }
}
