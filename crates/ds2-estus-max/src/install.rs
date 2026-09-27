//! The tick registration and the calls into the game.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};

use ds2_game_base::mem::{game_rva, read_bytes, safe_read_usize};

use crate::{CHECK_EVERY_TICKS, LOG_PREFIX, Levels, Seen, Step, Tracker};

/// `fn(u32 key) -> u8`. [`ds2_rva::ESTUS_PROPERTY_INDEX`].
type PropertyIndexFn = unsafe extern "system" fn(u32) -> u8;
/// `fn(ItemInventory2*, const u8* property) -> u8`. The level getter and the at-max predicate.
///
/// Typed `u8` rather than `bool` on purpose: a Rust `bool` holding anything but 0 or 1 is undefined
/// behaviour, and comparing a byte cannot be.
type PropertyFn = unsafe extern "system" fn(usize, *const u8) -> u8;
/// `fn(ItemInventory2*) -> u8`. The charge count.
type ChargesFn = unsafe extern "system" fn(usize) -> u8;
/// `fn(ItemInventory2*, const u8* property, i32 level) -> u8`. The setter.
type SetFn = unsafe extern "system" fn(usize, *const u8, i32) -> u8;

/// Resolved, prologue-checked addresses. All zero until [`install`] has checked every one.
static GET_LEVEL: AtomicUsize = AtomicUsize::new(0);
static GET_CHARGES: AtomicUsize = AtomicUsize::new(0);
static IS_MAX: AtomicUsize = AtomicUsize::new(0);
static SET_PROPERTY: AtomicUsize = AtomicUsize::new(0);
static GAME_MANAGER: AtomicUsize = AtomicUsize::new(0);

/// The game's own table indices for the two properties, asked of it at install.
static USES_INDEX: AtomicU8 = AtomicU8::new(ds2_rva::ESTUS_PROPERTY_NOT_FOUND);
static EFFECT_INDEX: AtomicU8 = AtomicU8::new(ds2_rva::ESTUS_PROPERTY_NOT_FOUND);

static TICKS: AtomicU64 = AtomicU64::new(0);
static TRACKER: Mutex<Tracker> = Mutex::new(Tracker::new());

/// `fn()` -- [`ds2_rva::FE_RETURN_TITLE_CHECK_CONFIRM`], the quit confirm's "yes".
type ReturnTitleFn = unsafe extern "system" fn();

/// The reload test's callback, run just before the return to title is requested.
///
/// The loader uses it to re-arm `ds2-continue`'s autoload once. Zero when the test is off.
static RELOAD_BEFORE: AtomicUsize = AtomicUsize::new(0);
/// The checked address of the quit confirm, set by [`install`] only when the test is on.
static RETURN_TITLE: AtomicUsize = AtomicUsize::new(0);
/// The tick at which to request the return to title; zero until the first load reached max.
static RELOAD_AT: AtomicU64 = AtomicU64::new(0);
/// Set once the return to title has been requested. Once per process: the test is one reload.
static RELOAD_DONE: AtomicBool = AtomicBool::new(false);

/// Turn on the one-shot reload test. Call before [`install`].
///
/// Once the first load in this process is at max, it waits [`crate::RELOAD_TEST_DELAY_TICKS`],
/// calls `before`, then asks the game to return to the title through the quit confirm's own "yes".
pub fn set_reload_test(before: fn()) {
    RELOAD_BEFORE.store(before as usize, Ordering::Release);
}

/// A log sink, installed by the loader so this crate writes into the same file as everything else.
static LOGGER: AtomicUsize = AtomicUsize::new(0);

/// Signature of the sink. Matches the loader's own logging entry point.
pub type LogFn = fn(std::fmt::Arguments<'_>);

/// Point this crate's logging at the loader's log file. Call before [`install`].
pub fn set_logger(logger: LogFn) {
    LOGGER.store(logger as usize, Ordering::Release);
}

fn log(args: std::fmt::Arguments<'_>) {
    let raw = LOGGER.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: `raw` is only ever a `LogFn` stored by `set_logger` above.
        let logger: LogFn = unsafe { std::mem::transmute::<usize, LogFn>(raw) };
        logger(args);
    }
}

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Whether the tick is registered. When this is `false` nothing is ever called.
    pub installed: bool,
}

/// Resolve `rva` and check it begins with `expected`, logging and refusing on a mismatch.
fn checked_site(rva: u32, expected: &[u8], name: &str) -> Option<usize> {
    let site = match game_rva(rva) {
        Ok(site) => site,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} not installed reason=no-module-base -- {error}"
            ));
            return None;
        }
    };
    let mut seen = vec![0u8; expected.len()];
    // SAFETY: a resolved RVA inside the loaded game image; `read_bytes` reports an unmapped page
    // rather than faulting on it.
    let read = unsafe { read_bytes(site, &mut seen) };
    if !read || seen != expected {
        log(format_args!(
            "{LOG_PREFIX} not installed reason=prologue fn={name} va=0x{site:016x} read={read} \
             saw={seen:02x?} want={expected:02x?} -- nothing will be called"
        ));
        return None;
    }
    Some(site)
}

/// Ask the game for one property's table index, refusing the not-found answer.
///
/// # Safety
///
/// `site` must be [`ds2_rva::ESTUS_PROPERTY_INDEX`] with its prologue checked.
unsafe fn property_index(site: usize, key: u32) -> Option<u8> {
    // SAFETY: the caller's contract. The function is six instructions over a static table: a key in
    // ECX, a byte in AL, no game state.
    let index = unsafe { std::mem::transmute::<usize, PropertyIndexFn>(site)(key) };
    (index != ds2_rva::ESTUS_PROPERTY_NOT_FOUND).then_some(index)
}

/// Check every function this crate calls and register the tick.
///
/// # Safety
///
/// The tick registration patches executable memory (through `ds2-net-tick`). Call once, from the
/// loader's post-Arxan install position, in the game process.
pub unsafe fn install() -> Outcome {
    let refused = Outcome { installed: false };
    let thunk = &ds2_rva::ESTUS_THUNK_PROLOGUE;
    let (Some(get_level), Some(get_charges), Some(is_max), Some(set), Some(index)) = (
        checked_site(ds2_rva::ESTUS_GET_LEVEL, thunk, "ESTUS_GET_LEVEL"),
        checked_site(ds2_rva::ESTUS_GET_CHARGES, thunk, "ESTUS_GET_CHARGES"),
        checked_site(ds2_rva::ESTUS_IS_MAX, thunk, "ESTUS_IS_MAX"),
        checked_site(ds2_rva::ESTUS_SET_PROPERTY, thunk, "ESTUS_SET_PROPERTY"),
        checked_site(
            ds2_rva::ESTUS_PROPERTY_INDEX,
            &ds2_rva::ESTUS_PROPERTY_INDEX_PROLOGUE,
            "ESTUS_PROPERTY_INDEX",
        ),
    ) else {
        return refused;
    };
    let Ok(manager) = game_rva(ds2_rva::GAME_MANAGER_IMP) else {
        log(format_args!(
            "{LOG_PREFIX} not installed reason=no-module-base for GAME_MANAGER_IMP"
        ));
        return refused;
    };
    // SAFETY: `index` is the checked property-index function.
    let indices = unsafe {
        (
            property_index(index, ds2_rva::ESTUS_PROPERTY_USES),
            property_index(index, ds2_rva::ESTUS_PROPERTY_EFFECT),
        )
    };
    let (Some(uses), Some(effect)) = indices else {
        log(format_args!(
            "{LOG_PREFIX} not installed reason=property-index -- the game's table does not know \
             the uses or effect key, and passing 0xFF on would write the charge count"
        ));
        return refused;
    };
    GET_LEVEL.store(get_level, Ordering::Release);
    GET_CHARGES.store(get_charges, Ordering::Release);
    IS_MAX.store(is_max, Ordering::Release);
    SET_PROPERTY.store(set, Ordering::Release);
    GAME_MANAGER.store(manager, Ordering::Release);
    USES_INDEX.store(uses, Ordering::Release);
    EFFECT_INDEX.store(effect, Ordering::Release);

    if RELOAD_BEFORE.load(Ordering::Acquire) != 0 {
        match checked_site(
            ds2_rva::FE_RETURN_TITLE_CHECK_CONFIRM,
            &ds2_rva::FE_RETURN_TITLE_CHECK_CONFIRM_PROLOGUE,
            "FE_RETURN_TITLE_CHECK_CONFIRM",
        ) {
            Some(quit) => {
                RETURN_TITLE.store(quit, Ordering::Release);
                log(format_args!(
                    "{LOG_PREFIX} reload-test armed: one return to title after the first load \
                     at max, then one more autoload"
                ));
            }
            None => log(format_args!(
                "{LOG_PREFIX} reload-test NOT armed -- the feature itself still installs"
            )),
        }
    }

    // SAFETY: the loader's post-Arxan install position, which is this function's own contract.
    match unsafe { ds2_net_tick::register(ds2_net_tick::When::After, tick) } {
        Ok(site) => {
            log(format_args!(
                "{LOG_PREFIX} installed on the net session tick at 0x{site:016x} property \
                 uses={uses} effect={effect} ask={} every {CHECK_EVERY_TICKS} ticks -- each load \
                 logs the flask's levels as the game reads them back",
                ds2_rva::ESTUS_LEVEL_ASK
            ));
            Outcome { installed: true }
        }
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} not installed: no tick -- {error}"
            ));
            refused
        }
    }
}

/// One fault-safe pointer hop that refuses a null.
fn hop(at: usize, offset: usize) -> Option<usize> {
    // SAFETY: `safe_read_usize` reports an unmapped page rather than faulting.
    unsafe { safe_read_usize(at + offset) }.filter(|pointer| *pointer != 0)
}

/// `(inventory, player)` for the local character, or `None` at the title or mid-load.
fn character() -> Option<(usize, usize)> {
    let manager = hop(GAME_MANAGER.load(Ordering::Acquire), 0)?;
    let player = hop(manager, ds2_rva::PLAYER_CTRL_OFFSET)?;
    let data = hop(manager, ds2_rva::GAME_DATA_MANAGER_OFFSET)?;
    let inventory = hop(data, ds2_rva::ITEM_INVENTORY_OFFSET)?;
    Some((inventory, player))
}

/// Read both levels, both at-max answers and the charges. `None` when there is no flask.
///
/// # Safety
///
/// Game thread, after [`install`] succeeded, with `inventory` the live `ItemInventory2`.
unsafe fn read(inventory: usize) -> Option<Levels> {
    let uses = USES_INDEX.load(Ordering::Acquire);
    let effect = EFFECT_INDEX.load(Ordering::Acquire);
    // SAFETY: each address was prologue-checked by `install` and each signature is the one its
    // disassembled thunk implements: the inventory in RCX, a pointer to the property byte in RDX.
    // `uses` and `effect` are locals that outlive the calls.
    unsafe {
        let level = std::mem::transmute::<usize, PropertyFn>(GET_LEVEL.load(Ordering::Acquire));
        let at_max = std::mem::transmute::<usize, PropertyFn>(IS_MAX.load(Ordering::Acquire));
        let charges = std::mem::transmute::<usize, ChargesFn>(GET_CHARGES.load(Ordering::Acquire));
        let uses_level = level(inventory, &uses);
        // Zero is not a level: the getter answers 0 when the flask slot is unbound.
        if uses_level == 0 {
            return None;
        }
        Some(Levels {
            uses: uses_level,
            effect: level(inventory, &effect),
            uses_at_max: at_max(inventory, &uses) != 0,
            effect_at_max: at_max(inventory, &effect) != 0,
            charges: charges(inventory),
        })
    }
}

/// The net-session-update callback. Never panics out: `ds2-net-tick` catches at the detour too.
fn tick(_session: usize) {
    if !TICKS
        .fetch_add(1, Ordering::Relaxed)
        .is_multiple_of(CHECK_EVERY_TICKS)
    {
        return;
    }
    let mut tracker = TRACKER.lock().unwrap_or_else(|poison| poison.into_inner());
    let found = character();
    let seen = match found {
        None => Seen::NoCharacter,
        Some(key) => Seen::Character {
            key,
            // SAFETY: the game thread, after `install`, with the inventory the game just handed us.
            levels: unsafe { read(key.0) },
        },
    };
    let mut at_max_now = matches!(seen, Seen::Character { levels: Some(l), .. } if l.at_max());
    match tracker.observe(seen) {
        Step::Nothing => {}
        Step::Say(note) => log(format_args!("{note}")),
        Step::Raise { uses, effect } => {
            let Some((inventory, _)) = found else { return };
            let Seen::Character {
                levels: Some(before),
                ..
            } = seen
            else {
                return;
            };
            let set = SET_PROPERTY.load(Ordering::Acquire);
            // SAFETY: `set` was prologue-checked by `install`; its signature is the disassembled
            // thunk's: inventory in RCX, a pointer to a property byte the game's own index
            // function returned (never 0xFF) in RDX, a level in R8D. The bool it returns is
            // dropped on purpose -- `false` means "already that level" -- and the read-back below
            // is the answer.
            unsafe {
                let set = std::mem::transmute::<usize, SetFn>(set);
                if uses {
                    let index = USES_INDEX.load(Ordering::Acquire);
                    set(inventory, &index, ds2_rva::ESTUS_LEVEL_ASK);
                }
                if effect {
                    let index = EFFECT_INDEX.load(Ordering::Acquire);
                    set(inventory, &index, ds2_rva::ESTUS_LEVEL_ASK);
                }
            }
            // SAFETY: as for `read` above.
            let after = unsafe { read(inventory) }.unwrap_or(before);
            at_max_now = after.at_max();
            let note = tracker.raised(before, after);
            log(format_args!("{note}"));
        }
    }
    drop(tracker);
    reload_test(at_max_now);
}

/// The one-shot reload test. Does nothing unless [`set_reload_test`] was called and [`install`]
/// checked the quit confirm's prologue.
fn reload_test(at_max_now: bool) {
    let quit = RETURN_TITLE.load(Ordering::Acquire);
    if quit == 0 || RELOAD_DONE.load(Ordering::Acquire) {
        return;
    }
    let now = TICKS.load(Ordering::Relaxed);
    let at = RELOAD_AT.load(Ordering::Acquire);
    if at == 0 {
        if at_max_now {
            RELOAD_AT.store(now + crate::RELOAD_TEST_DELAY_TICKS, Ordering::Release);
            log(format_args!(
                "{LOG_PREFIX} reload-test: first load at max -- returning to the title in {} ticks",
                crate::RELOAD_TEST_DELAY_TICKS
            ));
        }
        return;
    }
    if now < at {
        return;
    }
    RELOAD_DONE.store(true, Ordering::Release);
    let before = RELOAD_BEFORE.load(Ordering::Acquire);
    if before != 0 {
        // SAFETY: only ever a `fn()` stored by `set_reload_test`.
        let before = unsafe { std::mem::transmute::<usize, fn()>(before) };
        before();
    }
    // SAFETY: `quit` is FE_RETURN_TITLE_CHECK_CONFIRM with its prologue checked by `install`. It
    // reads only globals, ignores RCX, and its request refuses by itself outside the world. Called
    // on the game thread, where the quit confirm's own "yes" runs.
    unsafe { std::mem::transmute::<usize, ReturnTitleFn>(quit)() };
    log(format_args!(
        "{LOG_PREFIX} reload-test: asked the game to return to the title (the quit confirm's \
         own yes) -- the next load's line is the re-apply"
    ));
}
