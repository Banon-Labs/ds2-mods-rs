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
    field::{Field, Reaction},
};

use std::sync::Mutex;

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

/// The build id the player is typing, or `None` when nobody is typing.
///
/// **This is the third way in, and the only one that needs nothing from outside the game.** Steam's
/// field is absent on a desktop Steam and the clipboard only answers "what did you already copy" --
/// a player who knows the number and has an unrelated link in their clipboard could not get in at
/// all. See [`crate::typed`] for why this reads ten keys and not a keyboard.
static TYPING: Mutex<Option<Field>> = Mutex::new(None);

/// What the row says while a build id is being typed, with the digits so far after it.
const TYPING_PREFIX: &str = "Build ID: ";

/// The caret drawn at the end of the digits, so an empty field still looks like a field.
const CARET: char = '_';

/// Render the field onto the row. Caller must be on the game thread.
unsafe fn show_typing(field: &Field) {
    // SAFETY: forwarded to the caller.
    unsafe { say_now(&format!("{TYPING_PREFIX}{}{CARET}", field.text())) };
}

/// Start a typing session. **Game thread**, from the row's confirm.
unsafe fn open_typing() {
    let field = Field::new("");
    // The keyboard as it is RIGHT NOW is the baseline, so a key held at the moment of the press --
    // very possibly the key that caused the press -- does not become the first digit.
    // SAFETY: game thread, per this function's contract.
    unsafe { crate::typed::sync_to_current() };
    // SAFETY: game thread.
    unsafe { show_typing(&field) };
    if let Ok(mut typing) = TYPING.lock() {
        *typing = Some(field);
    }
    log_line(format_args!(
        "{LOG_PREFIX} nothing usable to read -- type the build id and press the row again"
    ));
}

/// Feed the field whatever was typed since the last frame. **Runs on the game thread**, per frame.
///
/// Returns without touching anything on the frames -- almost all of them -- when nobody is typing.
fn typing_tick() {
    let Ok(mut guard) = TYPING.lock() else {
        return;
    };
    let Some(field) = guard.as_mut() else {
        return;
    };
    // SAFETY: the tick runs on the game thread.
    let units = unsafe { crate::typed::poll() };
    if units.is_empty() {
        return;
    }
    for unit in units {
        // Only digits and backspace are read, and `Field` handles both. Every other reaction is
        // unreachable here rather than ignored -- if one ever arrives, the field is being fed
        // something `crate::typed` promised not to send.
        match field.on_char(unit) {
            Reaction::Handled | Reaction::Ignored => {}
            other => log_line(format_args!(
                "{LOG_PREFIX} the field asked for {other:?}, which nothing here sends"
            )),
        }
    }
    // SAFETY: the tick runs on the game thread, with the menu up.
    unsafe { show_typing(field) };
}

/// End a typing session and return what was typed, or `None` if nobody was typing.
fn close_typing() -> Option<String> {
    TYPING.lock().ok()?.take().map(|field| field.text())
}

/// A build the worker has fetched, waiting for the game thread to act on it.
///
/// **The handoff exists because the two halves cannot be on the same thread.** The fetch blocks on
/// a TLS handshake and must not be near the game thread; the grant calls INTO the game and must not
/// be anywhere else. `ds2-menu-row`'s per-frame tick is the only place that is both recurring and
/// correctly threaded, so the worker leaves the build here and the tick collects it.
static PENDING: Mutex<Option<ds2_build_import_core::Build>> = Mutex::new(None);

/// Hand a fetched build to the game thread.
fn hand_over(build: ds2_build_import_core::Build) {
    if let Ok(mut pending) = PENDING.lock() {
        *pending = Some(build);
    }
}

/// Apply whatever the worker left. **Runs on the game thread**, once per frame, from the tick.
///
/// Registered by [`crate::install::register`]; it does nothing on the overwhelming majority of
/// frames, because the mailbox is almost always empty.
pub(crate) fn apply_tick() {
    // The typing field lives on this same tick. It is first because it is the interactive half:
    // a frame that also has a build to apply should still show the digit that was just typed.
    typing_tick();
    let Some(build) = PENDING.lock().ok().and_then(|mut pending| pending.take()) else {
        return;
    };
    apply(&build);
}

/// Put a build on the live character, as far as this crate honestly can.
fn apply(build: &ds2_build_import_core::Build) {
    // WHAT THE CHARACTER IS NOW, read before anything changes, so the log can say what happened
    // rather than what was asked for.
    let param = match crate::game::player_param() {
        Ok(param) => param,
        Err(error) => {
            log_line(format_args!(
                "{LOG_PREFIX} cannot read the character: {error}"
            ));
            say("Could not read the character");
            return;
        }
    };
    let (Some(stats), Some(level), Some(memory)) = (
        crate::game::read_stats(param),
        crate::game::read_soul_level(param),
        crate::game::read_soul_memory(param),
    ) else {
        log_line(format_args!(
            "{LOG_PREFIX} the character's stats could not be read"
        ));
        say("Could not read the character");
        return;
    };
    let named: Vec<String> = ds2_rva::PLAYER_PARAM_STAT_NAMES
        .iter()
        .zip(stats)
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    log_line(format_args!(
        "{LOG_PREFIX} character now: level={level} soul-memory={memory:?} {}",
        named.join(" ")
    ));

    // WHAT THE STATS AMOUNT TO, which is what the player's own attribute menu shows and is NOT the
    // base block above. Logged so a run that ends in "the menu says 40 and you wrote 38" has the
    // answer in it. This used to be a consistency check that announced tampering on any difference,
    // which was wrong: `+0x1E` holds derived values, so a difference is the normal case.
    if let Some(effective) = crate::game::read_effective_stats(param)
        && effective != stats
    {
        log_line(format_args!(
            "{LOG_PREFIX} effective stats differ from base: {effective:?} against {stats:?} -- \
             something is modifying them, and the menu shows the first"
        ));
    }

    // THE LEVEL THE BUILD IS. Not computed from this character at all: the game derives a soul
    // level from the stats themselves (`max(1, sum - 53)`, at `0x14038e310`), so the build's stats
    // already ARE a level and there is nothing to reconcile. The earlier version of this added the
    // point difference to the current level, which gives the same answer only while the character
    // is self-consistent -- and a character whose level disagrees with its stats is exactly the
    // case worth surviving.
    let wanted = build.stats.in_game_order();
    let target = ds2_build_import_core::level::soul_level(&wanted);
    let current_points: u32 = stats.iter().map(|stat| u32::from(*stat)).sum();
    let wanted_points: u32 = wanted.iter().map(|stat| u32::from(*stat)).sum();
    if wanted_points < current_points {
        log_line(format_args!(
            "{LOG_PREFIX} the build is BELOW this character: {wanted_points} points against \
             {current_points}. Levels cannot be given back, so nothing is changed."
        ));
        say("That build is lower than you");
        return;
    }
    log_line(format_args!(
        "{LOG_PREFIX} build {} is level {target} (from {level}, +{} points)",
        build.id,
        wanted_points - current_points
    ));
    // The character's own level should already equal what its own stats imply. Saying so when it
    // does not costs one line and names a save that something else has edited.
    let implied = ds2_build_import_core::level::soul_level(&stats);
    if implied != level {
        log_line(format_args!(
            "{LOG_PREFIX} WARNING: this character reads level {level} but its stats imply \
             {implied} -- its level has been written by hand"
        ));
    }

    // SOUL MEMORY FIRST. This is the user's rule and the whole reason `LevelChange` exists: there is
    // no way to hold a level here without having computed the soul memory for it.
    match crate::game::soul_costs() {
        Ok(costs) => match ds2_build_import_core::LevelChange::to_level(target, &costs) {
            Ok(change) => raise_soul_memory(param, change),
            Err(error) => log_line(format_args!(
                "{LOG_PREFIX} cannot compute soul memory for level {target}: {error}"
            )),
        },
        Err(error) => log_line(format_args!(
            "{LOG_PREFIX} no level costs, so no soul memory: {error}"
        )),
    }

    // THE STATS, AND THEREFORE THE LEVEL -- SECOND, after the soul memory that supports them.
    // That order is the user's rule and it is now load-bearing rather than ceremonial: the call
    // below moves the character to level 150 in one frame, and a level whose soul memory has not
    // been raised first is a character DS2 will match against the wrong opponents.
    set_stats(param, &wanted, target);

    // THE ITEMS, THROUGH THE GAME'S OWN FUNCTION.
    let spawns = crate::build_items(build);
    if spawns.is_empty() {
        log_line(format_args!(
            "{LOG_PREFIX} the build named no grantable items"
        ));
        say("Nothing to grant");
        return;
    }
    // SAFETY: the game thread, from the pause menu's own per-frame update, with a character loaded
    // -- `player_param` above returned non-null. The call site's prologue is re-checked inside.
    match unsafe { crate::game::give_items(&spawns, ITEM_BATCH) } {
        Ok(granted) => {
            log_line(format_args!(
                "{LOG_PREFIX} granted {granted}/{} items for build {}",
                spawns.len(),
                build.id
            ));
            say(&format!("Gave {granted} items"));
        }
        Err(error) => {
            log_line(format_args!("{LOG_PREFIX} grant failed: {error}"));
            say("Could not grant the items");
        }
    }

    equip_everything(build);
    join_covenant(build);
}

/// The FLAT slot a planned position occupies, using the bases `ds2-rva` records.
///
/// The flat index is then mapped to the internal one the equip function takes. Doing it in two
/// steps rather than one is deliberate: the flat space is what the planner's form matches
/// position-for-position, and the internal space is the one where the weapon hands are swapped.
const fn flat_slot(kind: ds2_build_import_core::equip::SlotKind, position: usize) -> usize {
    use ds2_build_import_core::equip::SlotKind;
    let base = match kind {
        SlotKind::Weapon => ds2_rva::ITEM_SLOT_WEAPON_FLAT_BASE,
        SlotKind::Armour => ds2_rva::ITEM_SLOT_ARMOUR_FLAT_BASE,
        SlotKind::Ring => ds2_rva::ITEM_SLOT_RING_FLAT_BASE,
        SlotKind::Spell => ds2_rva::ITEM_SLOT_SPELL_FLAT_BASE,
        SlotKind::Hotbar => ds2_rva::ITEM_SLOT_HOTBAR_FLAT_BASE,
    };
    base + position
}

/// **Wear, hold, attune and quick-slot everything the build named.**
///
/// Runs after the grant, because the equip function names an item by a pointer to its inventory
/// entry and that entry does not exist until the item has been given.
fn equip_everything(build: &ds2_build_import_core::Build) {
    use ds2_build_import_core::equip::SlotKind;

    let planned = ds2_build_import_core::equip::plan(build);
    if planned.is_empty() {
        return;
    }

    // THE ATTUNEMENT BUDGET. Reading it after the stats were written is NOT enough: the count is
    // cached on the bag and the stat write does not touch it, so a character written to attunement
    // 30 still reports whatever it had before -- zero, for a blank one, which dropped every spell
    // in the build. So the game's own recalculation is run first, and the number is read back from
    // it. Attuning past the budget does not fail; it UNEQUIPS the slot, which is why the surplus is
    // dropped here and said out loud.
    // SAFETY: the game thread, from the pause menu's own per-frame update, with a character loaded.
    let capacity = match unsafe { crate::game::recalc_attunement_slots() } {
        Ok(capacity) => capacity,
        Err(error) => {
            log_line(format_args!(
                "{LOG_PREFIX} could not recalculate the attunement slots ({error}) -- no spell \
                 will be attuned"
            ));
            0
        }
    };

    let mut done = 0usize;
    let mut refused: Vec<String> = Vec::new();
    let mut over_budget = 0usize;

    for slot in &planned {
        if slot.kind == SlotKind::Spell && slot.position >= usize::from(capacity) {
            over_budget += 1;
            continue;
        }
        // EVERY ID THE NAME COULD MEAN, lowest first -- the lowest is what the grant used. The
        // rest are there because the game may store an item under a different id from the one it
        // was granted as; `Estus Flask` did exactly that. See `game::EquipRequest::item_ids`.
        let mut candidates = match ds2_build_import_core::id_for(&slot.name) {
            Ok(id) => vec![id],
            Err(ds2_build_import_core::ItemError::Ambiguous { ref ids, .. }) => ids.clone(),
            Err(_) => continue,
        };
        candidates.sort_unstable();
        if candidates.is_empty() {
            continue;
        }
        let flat = flat_slot(slot.kind, slot.position);
        let Some(internal) = ds2_rva::ITEM_SLOT_FLAT_TO_INTERNAL
            .get(flat)
            .copied()
            .filter(|internal| *internal >= 0)
        else {
            refused.push(format!(
                "{} {} (no such slot)",
                slot.kind.describe(),
                slot.position
            ));
            continue;
        };
        // SAFETY: the game thread, from the pause menu's own per-frame update, with a character
        // loaded. The call site's prologue is re-checked inside.
        match unsafe {
            crate::game::equip(crate::game::EquipRequest {
                internal_slot: internal as u32,
                item_ids: &candidates,
            })
        } {
            // THE READ-BACK IS THE ONLY EVIDENCE. This function returns nothing and fails silently
            // when an item does not fit the slot it was aimed at, so "it was called" is not "it
            // worked" and only the slot's contents afterwards can say which.
            Ok(outcome) if outcome.took() => done += 1,
            Ok(outcome) => refused.push(format!(
                "{} into {} {} (slot holds {:?})",
                slot.name,
                slot.kind.describe(),
                slot.position,
                outcome.landed
            )),
            Err(error) => refused.push(format!("{} ({error})", slot.name)),
        }
    }

    log_line(format_args!(
        "{LOG_PREFIX} equipped {done}/{} for build {}",
        planned.len(),
        build.id
    ));
    if over_budget > 0 {
        log_line(format_args!(
            "{LOG_PREFIX} {over_budget} spell(s) left unattuned -- attunement gives {capacity} \
             slot(s) and the build names more"
        ));
    }
    if !refused.is_empty() {
        log_line(format_args!(
            "{LOG_PREFIX} did not equip: {}",
            refused.join("; ")
        ));
    }
    say(&format!("Equipped {done}/{}", planned.len()));
}

/// **Join the covenant the build names, if it names one.**
///
/// # The only thing here besides soul memory that cannot be undone
///
/// The game's setter marks the covenant permanently discovered on this character and nothing
/// clears that flag. Leaving the covenant later changes which one is current and leaves the mark.
/// So a build import that names a covenant makes a change to the save that outlives the import,
/// and the log says which press did it rather than leaving the player to find out.
///
/// Nothing is announced to the session -- see [`crate::game::set_covenant`].
fn join_covenant(build: &ds2_build_import_core::Build) {
    let Some(id) = crate::game::covenant_id(&build.covenant) else {
        return;
    };
    let name = ds2_rva::COVENANT_NAMES
        .get(usize::from(id))
        .copied()
        .unwrap_or("?");
    // SAFETY: the game thread, from the pause menu's own per-frame update, with a character loaded.
    // The call site's prologue is re-checked inside.
    match unsafe { crate::game::set_covenant(id) } {
        Ok(set) if set.after == id => {
            log_line(format_args!(
                "{LOG_PREFIX} covenant {} -> {id} ({name})",
                set.before
            ));
            if !set.already_discovered {
                log_line(format_args!(
                    "{LOG_PREFIX} PERMANENT: {name} is now marked discovered on this character, \
                     and leaving the covenant does not clear it"
                ));
            }
        }
        Ok(set) => log_line(format_args!(
            "{LOG_PREFIX} the covenant did not take: asked for {id} ({name}), reads {}",
            set.after
        )),
        Err(error) => log_line(format_args!(
            "{LOG_PREFIX} could not set the covenant: {error}"
        )),
    }
}

/// Items per call. **One**, deliberately, even though the engine accepts up to 32.
///
/// # A batch is all-or-nothing, so a wide batch is a worse answer
///
/// [`ds2_rva::ITEM_GIVE`] returns a single bool for the whole call. At eight per call, one item the
/// engine refuses costs the seven beside it AND hides which one it was: a real run reported
/// `granted 8/18` with three batches, and the count alone cannot distinguish a bad ninth item from
/// a bad eighteenth. Eight was never chosen for a reason of ours -- it is the width the community
/// scripts use, bounded by their own buffer.
///
/// At one per call a refused item costs exactly itself and the log names it. Eighteen calls instead
/// of three, on a frame where the player has just pressed a menu row, is not a cost worth the
/// ambiguity.
const ITEM_BATCH: usize = 1;

/// Raise soul memory to the floor a level implies, through the game's own `AddSouls`.
///
/// # Why this happens even though the LEVEL is not written
///
/// The level itself needs the commit path at [`ds2_rva::PLAYER_PARAM_COMMIT_STATS`], whose request
/// object has an unknown layout -- so this crate cannot finish the job. What it CAN do is leave the
/// character able to finish it: `AddSouls` raises souls held by the same amount it raises soul
/// memory, so the player walks to a bonfire with exactly the souls the build's level costs and
/// levels up through the game's own menu. The result is a character the game built.
///
/// That ordering is the user's rule, not an accident of what is implemented: soul memory is
/// assigned BEFORE the level, and here the level is assigned by the player afterwards.
///
/// **It never lowers.** Soul memory is monotonic in this game -- the engine has no path that
/// reduces it -- so a character already past this floor keeps their own number and nothing is
/// called at all.
fn raise_soul_memory(param: usize, change: ds2_build_import_core::LevelChange) {
    let floor = change.soul_memory();
    // SAFETY: the game thread, from the pause menu's own per-frame update, with a character loaded.
    // The call site's prologue is re-checked inside.
    match unsafe { crate::game::raise_soul_memory_to(param, floor) } {
        Ok(None) => log_line(format_args!(
            "{LOG_PREFIX} soul memory already covers level {} ({floor} needed) -- left alone",
            change.level()
        )),
        Ok(Some(added)) => {
            log_line(format_args!(
                "{LOG_PREFIX} soul memory {:?} -> {:?}, souls held {} -> {} (guards {:?})",
                added.memory.0, added.memory.1, added.held.0, added.held.1, added.guards
            ));
            if added.memory_moved() {
                log_line(format_args!(
                    "{LOG_PREFIX} level {} is now affordable -- level up at a bonfire",
                    change.level()
                ));
            } else {
                // The call can be vetoed by a player status flag, or per counter by a skip byte.
                // Saying "it did nothing" beats reporting the amount we asked for.
                log_line(format_args!(
                    "{LOG_PREFIX} AddSouls changed NOTHING -- a status flag or a counter guard \
                     refused it. Guards read {:?}",
                    added.guards
                ));
            }
        }
        Err(error) => log_line(format_args!("{LOG_PREFIX} could not add souls: {error}")),
    }
}

/// Write the build's nine stats, and report what the game did with them.
///
/// `wanted` is in the GAME's order -- see [`ds2_build_import_core::saved_build::Stats::in_game_order`],
/// which is not the planner's. `expected_level` is what the stats imply, computed the same way the
/// game computes it, and is here only to be COMPARED against what the game actually wrote.
///
/// # Everything is read back, and the level is the reason
///
/// The level is not passed to the game; the game derives it. So reading it back is a check on the
/// whole call: if the nine stats landed and the level did not move to match, the recompute did not
/// run and the character is now inconsistent in exactly the way this crate exists to avoid. That is
/// worth a loud line rather than silence.
fn set_stats(param: usize, wanted: &[u16; 9], expected_level: u32) {
    // SAFETY: the game thread, from the pause menu's own per-frame update, with a character loaded.
    // The call site's prologue is re-checked inside.
    let set = match unsafe { crate::game::set_all_stats(param, wanted) } {
        Ok(set) => set,
        Err(error) => {
            log_line(format_args!(
                "{LOG_PREFIX} could not set the stats: {error}"
            ));
            say("Could not set the stats");
            return;
        }
    };
    log_line(format_args!(
        "{LOG_PREFIX} stats {:?} -> {:?}, level {} -> {}",
        set.stats.0, set.stats.1, set.level.0, set.level.1
    ));
    if !set.stats_took(wanted) {
        return log_line(format_args!(
            "{LOG_PREFIX} THE STATS DID NOT TAKE: asked for {wanted:?}, the character reads {:?}",
            set.stats.1
        ));
    }
    if set.level.1 != expected_level {
        return log_line(format_args!(
            "{LOG_PREFIX} the stats took but the level is {} where these stats imply \
             {expected_level} -- the game did not recompute",
            set.level.1
        ));
    }
    if let Some(effective) = set.effective
        && effective != *wanted
    {
        // NOT a failure. The attribute menu reads this block, so a player comparing it against the
        // planner will see these numbers, and the difference is whatever is modifying them.
        log_line(format_args!(
            "{LOG_PREFIX} the menu will show {effective:?}, which is these stats plus whatever \
             modifies them"
        ));
    }
    say(&format!("Level {expected_level}"));
}

/// Where a link came from, for the log.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Source {
    /// Steam drew a field and the player typed into it.
    SteamField,
    /// The player had copied a link.
    Clipboard,
    /// The player typed the build id on the row itself.
    Typed,
}

impl Source {
    const fn describe(self) -> &'static str {
        match self {
            Source::SteamField => "the Steam field",
            Source::Clipboard => "the clipboard",
            Source::Typed => "the row",
        }
    }
}

/// Handle the press. **Called on the GAME thread**, from the row's confirm, menu still up.
///
/// Returns the worker's job when there is one. Everything that can be decided immediately is
/// decided here, so the player gets an answer on the same frame they pressed.
pub(crate) fn begin_session() -> Option<Job> {
    // A GAME MUST BE IN PROGRESS. The row lives on the pause menu, so one nearly always is, but
    // "nearly always" is not a check and the fetch is pointless without a character to aim it at.
    match crate::save::require_live_character() {
        Ok(Some(name)) => log_line(format_args!("{LOG_PREFIX} character \"{name}\" is loaded")),
        // A blank character -- never named, all-ones stats, the kind a mule save is full of. It is
        // as loaded as any other and this used to refuse it.
        Ok(None) => log_line(format_args!("{LOG_PREFIX} an unnamed character is loaded")),
        Err(reason) => {
            log_line(format_args!("{LOG_PREFIX} refused: {reason}"));
            // A typing session belongs to a character. Losing the character ends it, or the digits
            // would still be sitting on the row after the player quit to the title.
            close_typing();
            // SAFETY: the confirm path -- game thread, menu up.
            unsafe { say_now(reason.caption()) };
            return None;
        }
    }

    // A PRESS WHILE TYPING IS THE SUBMIT, and it comes before every other source: the player is
    // looking at digits they typed, and reading the clipboard out from under them would be the row
    // ignoring the thing it just asked them to do.
    if let Some(typed) = close_typing() {
        if typed.is_empty() {
            log_line(format_args!(
                "{LOG_PREFIX} typing cancelled -- the field was empty"
            ));
            // SAFETY: still the confirm path.
            unsafe { say_now(IDLE_CAPTION) };
            return None;
        }
        log_line(format_args!("{LOG_PREFIX} typed build id {typed}"));
        // SAFETY: still the confirm path.
        unsafe { say_now("Reading link...") };
        // The prefix is not the player's to get wrong -- they typed a number, so they get the URL
        // that number belongs to. It still goes through `build_id_from_url` in `load`, which is
        // what catches a number too large to be an id.
        return Some(Job::Link {
            text: format!("{BUILD_URL_PREFIX}{typed}"),
            source: Source::Typed,
        });
    }

    // SAFETY: the confirm path.
    unsafe { say_now("Reading link...") };

    match steam_field() {
        Ok(session) => return Some(Job::SteamField(session)),
        Err(error) => log_line(format_args!(
            "{LOG_PREFIX} no Steam field ({error}, game-keyboard-built={}) -- reading the \
             clipboard instead",
            crate::steam::game_keyboard_built()
        )),
    }

    // THE CLIPBOARD ONLY WINS IF IT HOLDS A BUILD LINK. It used to win whenever it held anything,
    // and the rejection was then the end of the press -- so a player with an unrelated link copied
    // (which is to say, a player who had been browsing) pressed the row, read "not a soulsplanner
    // link", and had no way forward. Now that is the case that opens the field.
    match crate::clipboard::text() {
        Some(text) if build_id_from_url(&text).is_ok() => Some(Job::Link {
            text,
            source: Source::Clipboard,
        }),
        Some(text) => {
            // The rejection's own words, so the log says WHY this was not a link rather than that
            // it was not one. `load` would have said it; this path never reaches `load`.
            let why = build_id_from_url(&text).err().map_or_else(
                || String::from("no reason"),
                |rejection| rejection.to_string(),
            );
            log_line(format_args!(
                "{LOG_PREFIX} the clipboard holds \"{text}\" ({why})"
            ));
            // SAFETY: still the confirm path.
            unsafe { open_typing() };
            None
        }
        None => {
            log_line(format_args!("{LOG_PREFIX} the clipboard holds no text"));
            // SAFETY: still the confirm path.
            unsafe { open_typing() };
            None
        }
    }
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
                    // The game thread takes it from here -- see `apply_tick`.
                    hand_over(build);
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
