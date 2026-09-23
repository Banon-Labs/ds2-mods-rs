//! Installing the detours, the append itself, and the storage that carries the rows past the two
//! fixed vectors the game keeps its own in.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;

/// A log sink, installed by the loader so this crate writes into the same file as everything else
/// rather than opening one of its own. Stored as a `usize` because a `fn` pointer is not an
/// `Atomic` type; only ever set from [`set_logger`].
static LOGGER: AtomicUsize = AtomicUsize::new(0);

/// Signature of the sink. Matches the loader's own logging entry point.
pub type LogFn = fn(std::fmt::Arguments<'_>);

/// Run `callback` on the GAME THREAD, once per frame, while the pause menu is up.
///
/// **This is how a row does anything the game itself has to do.** A row's `on_confirm` is on the
/// game thread but happens once; work that answers later -- a fetch, a parse, anything on a worker
/// -- has nowhere to land, and calling into the game from a worker races the renderer. A tick is
/// both recurring and correctly-threaded, so the pattern is: the worker leaves a result somewhere,
/// and the tick picks it up and acts on it.
///
/// Ticks run BEFORE captions are pushed, so a tick that calls
/// [`set_row_caption`] is on screen the same frame.
///
/// It only fires while the pause menu's group is updating, which is exactly when calling into the
/// menu is safe and exactly when nobody is watching otherwise. A `fn()` rather than a closure,
/// because this crate stores it forever and will not own a caller's captured state.
///
/// Returns whether it was registered.
pub fn add_tick(callback: fn()) -> bool {
    crate::caption::add_tick(callback)
}

/// Change what a registered row says on screen.
///
/// **Safe from any thread, and nothing appears until the game thread pushes it.** The text is
/// copied into a buffer this crate leaked at first bind; the game only sees it when the pause menu
/// is next opened, or when [`refresh_row_captions`] is called while it is already up. That split is
/// deliberate: the caller most likely to want this is reporting the result of something slow, which
/// means it is holding that result on a worker, and writing into the scene from there would race
/// the renderer.
///
/// Text longer than the caption buffer is TRUNCATED rather than refused -- a short label is a
/// better failure than one still saying "Fetching...".
///
/// Returns whether the row exists. `false` also comes back before any pause menu has been opened,
/// because the buffers are built from the registry on the first bind.
pub fn set_row_caption(row: crate::RowId, text: &str) -> bool {
    crate::caption::set_caption(row.0, text)
}

/// Put a row's caption back to the text it registered.
///
/// Call it when a flow that borrowed the label is over. It reaches the screen the same way
/// [`set_row_caption`] does -- the pause menu's own per-frame push if the menu is up, the next
/// caption bind if it is not.
///
/// Calling it is not what keeps a stale caption off the screen the next time the menu is opened:
/// every caption goes back to its registered text at bind, whether or not the row that changed it
/// remembered to say so. What this buys is the label going back **while the player is still looking
/// at it** -- a swap whose confirm was declined leaves the menu up, and a row that goes on
/// announcing a departure nobody took is the thing to avoid.
///
/// Returns whether the row exists.
pub fn reset_row_caption(row: crate::RowId) -> bool {
    crate::caption::reset_caption(row.0)
}

/// Push every registered row's caption onto its element now.
///
/// # Safety
///
/// Calls into the game's scene machinery. **Game thread only**, and only while the pause menu whose
/// captions these are is actually up -- from a [`RowSpec::on_confirm`](crate::RowSpec::on_confirm),
/// which is exactly that. Returns how many were written; `0` means no menu has been bound yet and
/// there was nothing to write to.
pub unsafe fn refresh_row_captions() -> usize {
    // SAFETY: forwarded to the caller, who is promising the game thread and a live menu.
    unsafe { crate::caption::push_captions() }
}

/// Point this crate's logging at the loader's log file. Call before [`install`].
pub fn set_logger(logger: LogFn) {
    LOGGER.store(logger as usize, Ordering::Release);
}

pub(crate) fn log(args: std::fmt::Arguments<'_>) {
    let raw = LOGGER.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: `raw` is only ever a `LogFn` stored by `set_logger` above.
        let logger: LogFn = unsafe { std::mem::transmute::<usize, LogFn>(raw) };
        logger(args);
    }
}

/// Trampoline back to the original builder, published before the site is patched so a detour that
/// fires immediately cannot read a zero.
static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// How many times the detour has fired, and how many of those appended.
///
/// Both, not one. "No fourth row appeared" has two very different causes -- the append was refused,
/// or the tab was never built because the pause menu was never opened -- and a single counter
/// cannot tell them apart. The whole point of this crate is that its negative result is readable.
static FIRED: AtomicUsize = AtomicUsize::new(0);
static APPENDED: AtomicUsize = AtomicUsize::new(0);

/// The builder: `descriptor* build(descriptor*)`, argument in RCX, returned in RAX.
///
/// Read off the disassembly rather than assumed: the entry is `rex push rbx` / `sub rsp,0x50` /
/// `mov rbx,rcx`, it touches no other argument register, and it ends `mov rax,rbx` / `ret`.
type BuildItemsFn = unsafe extern "system" fn(*mut u8) -> *mut u8;

/// The gate every appended entry carries.
///
/// Gate `0` deliberately, for every registered row: a gated row is greyed by the availability pass
/// and a run in which a row is greyed cannot be distinguished from a run in which it never
/// appeared. It is also what keeps a cloned disabled-state overlay off the icon -- see
/// [`ds2_rva::FLO_QUIT_ROW_ICON_GROUP`].
///
/// The actions are OURS, not borrowed from the game -- see
/// [`ds2_rva::FE_INGAME_MENU_ACTION_BASE`]. The shipped dispatch has no case for any of them, so
/// with the dispatch detour absent a registered row plays the ordinary confirm sound and does
/// nothing. An inert row is the right failure mode; a row that quietly opened Key Bindings would
/// not be.
const GATE: u32 = ds2_rva::FE_INGAME_MENU_GATE_ALWAYS;

/// Address of entry `index` inside a tab's item vector.
///
/// The odd-looking padding term is the game's own, transcribed rather than reasoned about: every
/// builder addresses its elements as `(-(int)descriptor & 3) + descriptor + n * 8`. On an aligned
/// descriptor it is zero, but "it is probably zero" is not a reason to compute a different address
/// from the code that will read it back.
///
/// # Safety
///
/// `descriptor` must point at a tab item vector, and `index` must be below
/// [`ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY`].
unsafe fn entry_at(descriptor: *mut u8, index: usize) -> *mut u8 {
    let padding = (0u32.wrapping_sub(descriptor as usize as u32) & 3) as usize;
    // SAFETY: the caller guarantees the descriptor and a within-capacity index, and the offset is
    // the one the game itself computes for the same element.
    unsafe { descriptor.add(padding + index * ds2_rva::FE_INGAME_MENU_ITEM_STRIDE) }
}

/// Read `count` entries as `(action, gate)` pairs.
///
/// # Safety
///
/// `descriptor` must point at a tab item vector holding at least `count` entries.
unsafe fn read_entries(descriptor: *mut u8, count: usize) -> Vec<(u32, u32)> {
    (0..count)
        .map(|index| {
            // SAFETY: `index < count`, and the caller guarantees that many entries are live.
            let entry = unsafe { entry_at(descriptor, index) };
            // SAFETY: an entry is two `u32`s, which is how both of the game's own readers split it.
            unsafe {
                (
                    entry.cast::<u32>().read(),
                    entry.add(4).cast::<u32>().read(),
                )
            }
        })
        .collect()
}

/// `(7,0) (8,0) (9,4)`, for a log line.
fn describe(entries: &[(u32, u32)]) -> String {
    let mut out = String::new();
    for (action, gate) in entries {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("({action:#x},{gate})"));
    }
    out
}

/// Run the original builder, then append one entry to what it produced.
///
/// # Safety
///
/// `descriptor` is the stack descriptor `FeGroupInGameTopSelect`'s constructor passed in; the
/// original has to run against it first, because everything below reads what the original wrote.
unsafe fn append(descriptor: *mut u8) -> *mut u8 {
    let trampoline = TRAMPOLINE.load(Ordering::Acquire);
    let returned = if trampoline == 0 {
        // Cannot happen -- the trampoline is published before the site is patched -- but returning
        // the argument keeps the ABI honest if it ever does, instead of returning uninitialised RAX.
        descriptor
    } else {
        // SAFETY: MinHook published this trampoline for exactly this site, and the signature is the
        // one the disassembled entry and exit implement.
        let original: BuildItemsFn =
            unsafe { std::mem::transmute::<usize, BuildItemsFn>(trampoline) };
        unsafe { original(descriptor) }
    };

    let fired = FIRED.fetch_add(1, Ordering::Relaxed) + 1;
    if descriptor.is_null() {
        log(format_args!(
            "{LOG_PREFIX} REFUSED reason=null-descriptor fire={fired}"
        ));
        return returned;
    }

    // SAFETY: the original has just run against this pointer and wrote this very field, so the
    // descriptor is live and at least as large as the count the game itself writes at this offset.
    let count = unsafe {
        descriptor
            .add(ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_COUNT_OFFSET)
            .cast::<u64>()
            .read()
    } as usize;

    // WHAT THE ORIGINAL LEFT BEHIND, checked before anything is written. This is the integrity
    // check the whole experiment rests on: appending to the wrong tab produces a screenshot that
    // looks exactly like a result and is about nothing.
    if count > ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY {
        log(format_args!(
            "{LOG_PREFIX} REFUSED reason=count-over-capacity count={count} \
             capacity={} fire={fired}",
            ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY
        ));
        return returned;
    }
    // SAFETY: `count` is at or below capacity, so that many entries are within the vector.
    let entries = unsafe { read_entries(descriptor, count) };
    if entries != ds2_rva::FE_INGAME_MENU_SYSTEM_TAB_ITEMS {
        log(format_args!(
            "{LOG_PREFIX} REFUSED reason=unexpected-entries count={count} saw=[{}] \
             expected=[{}] fire={fired}",
            describe(&entries),
            describe(&ds2_rva::FE_INGAME_MENU_SYSTEM_TAB_ITEMS)
        ));
        return returned;
    }
    // AS MANY ROWS AS THE GAME'S OWN VECTOR HOLDS, AND NO MORE. The rest are served from
    // [`ITEM_ENTRIES`] by the lookup detour, which is why running out here is not a refusal any
    // more -- it is the split, and the numbers are logged so a reader can see where it fell.
    //
    // **The game's slots are filled FIRST on purpose.** The lookup detour declines for anything the
    // vector holds, so the rows that fit here go through the game's own path end to end, exactly as
    // they did before that detour existed -- there is one fewer thing that has to be right for the
    // first rows than for the last ones.
    // THE SEVENTH TAB TAKES THE ROWS, if there is one. This builder runs for the System tab, and
    // once `crate::tab` is armed the rows belong somewhere else -- appending here as well is how the
    // first run of that module put every row on both tabs at once. Asked of `armed()` rather than of
    // a group pointer because this call happens inside the constructor whose detour builds the
    // group, so the group does not exist yet.
    let rows = if crate::tab::armed() {
        Vec::new()
    } else {
        crate::api::rows_for(crate::api::Tab::Quit)
    };
    if rows.is_empty() {
        log(format_args!(
            "{LOG_PREFIX} appended nothing -- the System tab keeps the {count} rows the game \
             shipped, and the added rows are on the seventh tab fire={fired}"
        ));
        return returned;
    }
    let fits = ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY.saturating_sub(count);
    let mut written = 0usize;
    for row in rows.iter().take(fits) {
        let at = count + written;
        // SAFETY: `at < capacity` because `written < fits`, so this slot is inside the vector, and
        // it is the same address the builder's own next push would have written.
        unsafe {
            let slot = entry_at(descriptor, at);
            slot.cast::<u32>().write(row.action);
            slot.add(4).cast::<u32>().write(GATE);
        }
        written += 1;
    }
    if written == 0 {
        return returned;
    }
    // SAFETY: `written` slots were just filled, contiguously, from `count`.
    unsafe {
        descriptor
            .add(ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_COUNT_OFFSET)
            .cast::<u64>()
            .write((count + written) as u64);
    }

    let appended = APPENDED.fetch_add(1, Ordering::Relaxed) + 1;
    // Logged only for the first couple of opens. The pause menu is opened over and over in a
    // session and this sink calls `sync_all` per line, so a line that repeats forever is a stall
    // that repeats forever. The first two are the evidence; the rest are noise with a cost.
    if appended <= 2 {
        let added = rows
            .iter()
            .take(written)
            .map(|row| format!("({:#x},{GATE})", row.action))
            .collect::<Vec<_>>()
            .join(" ");
        log(format_args!(
            "{LOG_PREFIX} appended [{added}] was=[{}] count={}->{} of-capacity={} \
             ours={} fire={fired} appends={appended}",
            describe(&entries),
            count,
            count + written,
            ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY,
            rows.len() - written
        ));
    }
    returned
}

unsafe extern "system" fn detour(descriptor: *mut u8) -> *mut u8 {
    unsafe { append(descriptor) }
}

/// Trampoline back to the item dispatch, and how many times we have asked the game to quit.
static DISPATCH_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static QUITS_REQUESTED: AtomicUsize = AtomicUsize::new(0);

/// The `FeGroupInGameTopSelect` the dispatch was last called on.
///
/// Stashed because a row's [`crate::RowSpec::on_confirm`] takes no arguments, and firing one of the
/// game's own actions needs the receiver the game would have passed. Recorded on every dispatch,
/// including the shipped rows', so it is the live object rather than one remembered from a menu
/// that has since closed. Only [`return_to_title`] reads it, and only from inside a confirm.
static LAST_TOP_SELECT: AtomicUsize = AtomicUsize::new(0);

/// The dispatch: `void dispatch(topSelect, action)`, `this` in RCX and the action in EDX.
///
/// Read off the disassembly: the entry is `48 89 5c 24 18` / `57` / `sub rsp,...`, and the body
/// switches on the 32-bit second argument.
type DispatchFn = unsafe extern "system" fn(*mut u8, u32);

/// Ask the game to shut down, by the only mechanism it has.
///
/// This is `FeSubStateTitleShutdown::v1` transcribed -- one pointer load and one byte -- and that
/// substate is what the TITLE screen's own exit row enters. The byte is polled every frame by
/// `GameManagerImp`'s master update, so the shutdown that follows is the game's own, on the game's
/// own schedule, and this function is not on the stack when it happens.
///
/// **It does not save, and it does not ask.** The quit-to-title flow offers to save because that
/// flow asks; this one is "without a confirmation" and the absence of a save is the same coin.
///
/// # Safety
///
/// Reads the singleton pointer and writes one byte inside it. Must run on the game thread with the
/// title/game systems constructed, which the menu dispatch guarantees -- there is no pause menu
/// before there is a game.
unsafe fn request_shutdown(base: usize) -> bool {
    // SAFETY: the RVA is a data global recorded in `ds2-rva`, resolved against the live base.
    let singleton = (base + ds2_rva::FE_SYSTEM_SINGLETON as usize) as *const usize;
    // SAFETY: the RVA is a data global recorded in `ds2-rva`, resolved against the live base.
    let system = unsafe { singleton.read() };
    if system == 0 {
        log(format_args!(
            "{LOG_PREFIX} quit REFUSED reason=singleton-null -- nothing was written"
        ));
        return false;
    }
    // SAFETY: non-null, and the game's own shutdown substate writes this exact byte at this exact
    // offset inside this exact object.
    unsafe {
        (system as *mut u8)
            .add(ds2_rva::FE_SYSTEM_SHUTDOWN_REQUEST_OFFSET)
            .write(1);
    }
    let n = QUITS_REQUESTED.fetch_add(1, Ordering::Relaxed) + 1;
    log(format_args!(
        "{LOG_PREFIX} quit-to-desktop requested system=0x{system:016x} \
         offset={:#x} value=1 requests={n} -- the game exits on its next frame",
        ds2_rva::FE_SYSTEM_SHUTDOWN_REQUEST_OFFSET
    ));
    true
}

/// Quit to desktop, without saving and without asking.
///
/// The [`crate::RowSpec::on_confirm`] this crate ships, and the first consumer of its own API --
/// `ds2-loader` registers a row pointing here rather than this crate hardcoding one. Safe to call
/// from anywhere on the game thread: it writes one byte the master update polls, and does nothing
/// but log if the singleton is not up yet.
pub fn quit_to_desktop() {
    let base = ds2_game_base::mem::game_module_base().unwrap_or(0);
    if base == 0 {
        log(format_args!(
            "{LOG_PREFIX} quit REFUSED reason=no-module-base -- nothing was written"
        ));
        return;
    }
    // SAFETY: the base is the live game module and the pause menu cannot be open before the
    // systems this reads are constructed.
    unsafe { request_shutdown(base) };
}

/// `bool refused(const u32 *gate)` -- see [`ds2_rva::FE_INGAME_MENU_GATE_EVALUATE`].
///
/// `u8` rather than `bool` on the way back: the game returns its answer in `al` and a `bool` whose
/// byte is neither 0 nor 1 is undefined behaviour in Rust, which is a poor way to learn that an
/// address was wrong.
type GateFn = unsafe extern "system" fn(*const u32) -> u8;

/// Leave the game the way the shipped Quit Game row does, saving on the way out.
///
/// This fires the game's own action [`ds2_rva::FE_INGAME_MENU_ACTION_RETURN_TITLE`] on the receiver
/// the dispatch was last called with, which opens `FeGroupInGameReturnTitleCheck` -- the confirm
/// that offers to save and then takes the player to the title screen. Nothing is forged: the object,
/// the action and the code that runs are the ones behind the row three lines above this crate's own.
///
/// # It applies the gate, because the row does
///
/// The tab's confirm handler runs [`ds2_rva::FE_INGAME_MENU_GATE_EVALUATE`] on the entry's gate
/// index before it dispatches, and passes `-1` instead of the action when the answer is "refused".
/// A caller that skipped that would be able to leave a game the shipped row would not let go of, so
/// this evaluates the same predicate on the same gate and refuses the same cases. What gate `4`
/// actually forbids is still not recorded anywhere in this project -- which is the reason to ask it
/// rather than to reason about it.
///
/// Returns whether the dispatch was made. A `false` means the game is exactly where it was.
///
/// Safe to call from a row's `on_confirm`: that is the game thread, inside the menu's own confirm
/// path, which is the same stack the shipped quit row dispatches from.
pub fn return_to_title() -> bool {
    let top_select = LAST_TOP_SELECT.load(Ordering::Acquire);
    let trampoline = DISPATCH_TRAMPOLINE.load(Ordering::Acquire);
    if top_select == 0 || trampoline == 0 {
        log(format_args!(
            "{LOG_PREFIX} return-to-title REFUSED reason=not-in-a-menu top_select=0x{top_select:016x} \
             dispatch=0x{trampoline:016x} -- nothing was dispatched"
        ));
        return false;
    }
    let Ok(gate_address) = ds2_game_base::mem::game_rva(ds2_rva::FE_INGAME_MENU_GATE_EVALUATE)
    else {
        log(format_args!(
            "{LOG_PREFIX} return-to-title REFUSED reason=no-module-base -- nothing was dispatched"
        ));
        return false;
    };
    let expected = ds2_rva::FE_INGAME_MENU_GATE_EVALUATE_PROLOGUE;
    let mut prologue = [0u8; 5];
    // SAFETY: a resolved RVA inside the loaded game image; `read_bytes` faults safely.
    let read = unsafe { ds2_game_base::mem::read_bytes(gate_address, &mut prologue) };
    if !read || prologue != expected {
        log(format_args!(
            "{LOG_PREFIX} return-to-title REFUSED reason=gate-prologue va=0x{gate_address:016x} \
             read={read} saw={prologue:02x?} want={expected:02x?} -- that address is not the gate \
             predicate on this build"
        ));
        return false;
    }
    let gate = ds2_rva::FE_INGAME_MENU_GATE_RETURN_TITLE;
    // SAFETY: the prologue matches the function `ds2-rva` transcribed, the signature is the one its
    // disassembly implements (one pointer argument, a byte back), and `gate` is a live local for the
    // duration of the call. The function only reads.
    let refused = unsafe {
        let evaluate: GateFn = std::mem::transmute::<usize, GateFn>(gate_address);
        evaluate(&raw const gate) != 0
    };
    if refused {
        log(format_args!(
            "{LOG_PREFIX} return-to-title REFUSED reason=gate gate={gate} -- the shipped Quit Game \
             row is refused right now too, so this one is as well"
        ));
        return false;
    }
    // SAFETY: MinHook published this trampoline for the dispatch, the signature is the one the
    // disassembled entry implements, and `top_select` is the receiver the game itself passed on the
    // call that is still on this stack.
    unsafe {
        let dispatch: DispatchFn = std::mem::transmute::<usize, DispatchFn>(trampoline);
        dispatch(
            top_select as *mut u8,
            ds2_rva::FE_INGAME_MENU_ACTION_RETURN_TITLE,
        );
    }
    log(format_args!(
        "{LOG_PREFIX} return-to-title dispatched action={} top_select=0x{top_select:016x} -- the \
         game's own confirm is now up",
        ds2_rva::FE_INGAME_MENU_ACTION_RETURN_TITLE
    ));
    true
}

unsafe extern "system" fn dispatch_detour(top_select: *mut u8, action: u32) {
    LAST_TOP_SELECT.store(top_select as usize, Ordering::Release);
    if let Some(row) = crate::api::row_for_action(action) {
        // Deliberately NOT calling the original for a registered action. The original would play a
        // sound and fall through its `default`, which is harmless, but the id is outside the range
        // its `switch` handles, so there is nothing there to run. Fewer moving parts.
        //
        // The callback belongs to whichever crate registered the row. It runs on the game thread,
        // inside the menu's own confirm path, which is the same place the shipped rows' handlers
        // run -- so it may do what they do.
        (row.on_confirm)();
        return;
    }
    let trampoline = DISPATCH_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for exactly this site, and the signature is the
        // one the disassembled entry implements.
        let original: DispatchFn = unsafe { std::mem::transmute::<usize, DispatchFn>(trampoline) };
        unsafe { original(top_select, action) };
    }
}

// =============================================================================================
// STORAGE OF OUR OWN, FOR THE ROWS THE GAME'S TWO FIXED VECTORS CANNOT HOLD
//
// The System tab's item vector holds five and its cell namer's list holds six, and both are inline
// arrays whose element 6 would land on their own count field -- there is nothing to repoint. What
// there is, for each of them, is exactly ONE function that reads an element, and this section is
// those two detours plus the storage they answer out of.
//
// Everything here is written so the hot path takes no lock and allocates nothing: the registry is
// read ONCE at install and flattened into [`ITEM_ENTRIES`] and [`ADDED_ROWS`], because
// [`FE_INGAME_MENU_TAB_ITEM_LOOKUP`](ds2_rva::FE_INGAME_MENU_TAB_ITEM_LOOKUP)'s second caller runs
// off the cursor and the cell lookup runs once per probed cell. An instrument that locks a mutex on
// every frame is how this crate froze a pause menu once already.
// =============================================================================================

/// How many rows this crate is serving on the System tab, read once from the registry at install.
///
/// An atomic rather than a `rows_for` call: the two detours below run on the game thread in the
/// menu's own paths, and the registry is sealed before either is installed, so there is nothing a
/// lock could protect against and a lock is the one thing that has cost a frame here.
static ADDED_ROWS: AtomicUsize = AtomicUsize::new(0);

/// The live module base, resolved once by [`install`].
///
/// `game_module_base()` is `GetModuleHandleA(NULL)`, which is cheap but is still a call, and the
/// item lookup is reached from `FUN_1400a4cc0` as well as from the confirm handler -- a path this
/// crate has not established the frequency of. The base cannot change for the life of the process,
/// so asking once is both correct and the answer to the question.
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);

/// One `(action, gate)` pair per added row, in slot order, at an address stable for the process.
///
/// This is the STORAGE the item vector cannot provide. The game reads an entry as two `u32`s at
/// `+0x00` and `+0x04` ([`ds2_rva::FE_INGAME_MENU_ITEM_STRIDE`]), and a flat array of `AtomicU32`
/// is exactly that layout with no `static mut` and no leak: slot `n` is elements `2n` and `2n + 1`.
///
/// Filled at install, before any hook is enabled, and never written again.
static ITEM_ENTRIES: [AtomicU32; crate::api::MAX_ADDED_ROWS * 2] =
    [const { AtomicU32::new(0) }; crate::api::MAX_ADDED_ROWS * 2];

/// The entry the game itself hands back for an index its vector does not hold, transcribed.
///
/// `FUN_1400a6750`'s miss path initialises a pair of globals to `0xffffffff` and `0` and returns
/// them. An action outside every `case` and a gate of zero is an INERT row -- the correct answer for
/// a lookup that cannot reach the original, and the only one that does not leave the caller
/// dereferencing a null.
static REFUSED_ENTRY: [AtomicU32; 2] = [
    AtomicU32::new(u32::MAX),
    AtomicU32::new(ds2_rva::FE_INGAME_MENU_GATE_ALWAYS),
];

/// A stand-in cell namer: the three fields [`ds2_rva::FE_SCENE_NAMER_CELL_LOOKUP`] reads, and
/// nothing else.
///
/// `align(16)` matters and is not decoration. The lookup computes its entry address as
/// `list + (-(int)list & 7) + row * 0x30` with `list = namer + 0x18`; on a 16-aligned buffer that
/// padding term is zero and the entry is exactly where this crate wrote it. On an odd address it
/// would not be, and the game would read a shifted copy of a path.
#[repr(C, align(16))]
struct ShadowNamer([u8; ds2_rva::FE_SCENE_NAMER_SHADOW_SIZE]);

/// One stand-in per slot, allocated a single time and refreshed on every menu open.
///
/// Per slot rather than one shared list, because the lookup's own stride would put entry 6 on top
/// of the count field at `+0x140` -- the same collision that caps the game's list at six. A
/// stand-in carrying ONE entry at row 0, asked for cell `(0, 0)`, never reaches that offset no
/// matter how many rows there are.
///
/// Allocated once rather than per open, because the namer is rebuilt every time the pause menu is
/// opened and leaking twelve buffers per open is a slow leak nobody would attribute to this.
static SHADOW_NAMERS: [AtomicUsize; crate::api::MAX_ADDED_ROWS] =
    [const { AtomicUsize::new(0) }; crate::api::MAX_ADDED_ROWS];

/// The tab strip's own cell namer, and the one stand-in that answers for the seventh tab's cell.
///
/// The strip is a grid like a tab is, and its cells are its tab icons. Its namer holds six ids in a
/// list of six, so the seventh tab's cell cannot be pushed -- and without an entry the grid never
/// asks the layout for the record [`crate::strip`] added, which is exactly what a first run
/// measured: the tab was reachable by cursor and nothing was drawn for it.
///
/// One stand-in, not twelve, because there is only ever one seventh tab.
static STRIP_NAMER: AtomicUsize = AtomicUsize::new(0);
static STRIP_SHADOW: AtomicUsize = AtomicUsize::new(0);
static STRIP_CELLS_SERVED: AtomicUsize = AtomicUsize::new(0);
static STRIP_NAMERS_ADOPTED: AtomicUsize = AtomicUsize::new(0);

/// Take the tab strip's namer and fill the stand-in that answers for the seventh tab's cell.
///
/// Verified before anything is written, on all four fields the clone depends on: the list holds
/// exactly the six the game pushes, and its last entry is a two-component path whose base is the
/// strip and whose cell is the sixth tab's. A namer that is not that one is left alone and the
/// strip keeps its six tabs.
///
/// # Safety
///
/// `namer` must be the namer [`ds2_rva::FE_INGAME_TOP_SELECT_NAMER`] has just constructed, and
/// `base` the live module base.
pub(crate) unsafe fn adopt_strip_namer(base: usize, namer: usize) -> bool {
    let refuse = |why: core::fmt::Arguments<'_>| {
        STRIP_NAMER.store(0, Ordering::Release);
        log(format_args!(
            "{LOG_PREFIX} strip namer REFUSED {why} -- the strip keeps its six tabs"
        ));
        false
    };
    let shadow = STRIP_SHADOW.load(Ordering::Acquire);
    if !sane(namer) || shadow == 0 {
        return refuse(format_args!(
            "namer=0x{namer:016x} stand-in=0x{shadow:016x}"
        ));
    }
    let list = namer + ds2_rva::FE_SCENE_NAMER_LIST_OFFSET;
    // SAFETY: the constructor has just filled this vector, and this is the field its own push reads.
    let count =
        unsafe { ((list + ds2_rva::FE_SCENE_NAMER_COUNT_OFFSET) as *const u64).read() } as usize;
    if count != ds2_rva::FE_INGAME_TOP_SELECT_TABS {
        return refuse(format_args!(
            "count={count}, expected {}",
            ds2_rva::FE_INGAME_TOP_SELECT_TABS
        ));
    }
    let padding = (0u32.wrapping_sub(list as u32) & 7) as usize;
    let last = (list + padding + (count - 1) * ds2_rva::FE_SCENE_NAMER_ENTRY_STRIDE) as *mut u8;
    let dword = |offset: usize| -> u32 {
        // SAFETY: `offset` is inside an entry the count above established is live.
        unsafe { last.add(offset).cast::<u32>().read() }
    };
    let length = dword(ds2_rva::FE_SCENE_NAMER_ENTRY_LEN_OFFSET);
    if length != ds2_rva::FE_INGAME_TOP_SELECT_NAMER_ENTRY_LEN {
        return refuse(format_args!(
            "entry length is {length}, expected {}",
            ds2_rva::FE_INGAME_TOP_SELECT_NAMER_ENTRY_LEN
        ));
    }
    // THE CELL ID IS THE LAST COMPONENT, not a fixed offset. A tab's row paths are five long and
    // the strip's are two, and the one thing both have in common is that the id is at the end.
    let id_at = (length as usize - 1) * 4;
    let (root, cell) = (dword(0), dword(id_at));
    if root != ds2_rva::FE_INGAME_TOP_SELECT_NAMER_BASE
        || cell != ds2_rva::FE_INGAME_TOP_SELECT_NAMER_CELL_IDS[count - 1]
    {
        return refuse(format_args!(
            "entry[{}] is [{root:#x} {cell:#x}], expected [{:#x} {:#x}]",
            count - 1,
            ds2_rva::FE_INGAME_TOP_SELECT_NAMER_BASE,
            ds2_rva::FE_INGAME_TOP_SELECT_NAMER_CELL_IDS[count - 1]
        ));
    }
    // SAFETY: the namer is live and this is the field its own lookup reads the scene proxy from.
    let proxy = unsafe {
        (namer as *const u8)
            .add(ds2_rva::FE_SCENE_NAMER_PROXY_OFFSET)
            .cast::<usize>()
            .read()
    };
    if !sane(proxy) {
        return refuse(format_args!("proxy=0x{proxy:016x}"));
    }
    // SAFETY: the RVA is a `.pdata` function start recorded in `ds2-rva`; `last` is a live entry
    // and the destination is a `FE_SCENE_NAMER_SHADOW_SIZE` buffer this crate owns.
    let copy: NamerEntryCopyFn =
        unsafe { std::mem::transmute(base + ds2_rva::FE_SCENE_NAMER_ENTRY_COPY as usize) };
    // SAFETY: every write below is inside that buffer, at the offsets the lookup reads.
    unsafe {
        let shadow = shadow as *mut u8;
        shadow.write_bytes(0, ds2_rva::FE_SCENE_NAMER_SHADOW_SIZE);
        shadow
            .add(ds2_rva::FE_SCENE_NAMER_PROXY_OFFSET)
            .cast::<usize>()
            .write(proxy);
        let entry = shadow.add(ds2_rva::FE_SCENE_NAMER_LIST_OFFSET);
        copy(entry, last);
        entry
            .add(id_at)
            .cast::<u32>()
            .write(ds2_rva::FLO_ADDED_TAB_ID);
        shadow
            .add(ds2_rva::FE_SCENE_NAMER_COUNT_FROM_NAMER)
            .cast::<u64>()
            .write(1);
    }
    // Published last, so the cell lookup cannot see a namer whose stand-in is half filled.
    STRIP_NAMER.store(namer, Ordering::Release);
    let n = STRIP_NAMERS_ADOPTED.fetch_add(1, Ordering::Relaxed) + 1;
    log(format_args!(
        "{LOG_PREFIX} strip namer adopted namer=0x{namer:016x} cells={count} cell {}={:#x} \
         stand-in=0x{shadow:016x} adoptions={n} -- the seventh tab's icon is now asked for",
        count,
        ds2_rva::FLO_ADDED_TAB_ID
    ));
    true
}

/// The System tab's live cell namer, captured when its constructor's detour accepted it.
///
/// The cell lookup is ONE function shared by every tab's namer and by the tab strip's, so the
/// detour has to know which instance is ours. An identity compare against the pointer the namer
/// detour verified is exact, and it is one comparison for the five namers that are not ours.
///
/// Zero means "no namer has been accepted", which is the state the cell detour has to treat as
/// pass-through -- including after a refusal, which is why it is cleared there.
static SYSTEM_TAB_NAMER: AtomicUsize = AtomicUsize::new(0);

/// How many rows the last accepted namer actually has a cell for, pushed and stood in together.
///
/// The count raise reads this rather than [`ADDED_ROWS`], and that coupling is the point. The
/// cursor's bound and the drawn cells are two numbers from two places -- the whole lesson of the
/// invisible fourth row -- and raising the first past the second gives a row that is reachable and
/// not there. Cleared with [`SYSTEM_TAB_NAMER`] on every namer construction, so a refusal leaves the
/// tab's cursor exactly where the game put it.
static NAMED_ROWS: AtomicUsize = AtomicUsize::new(0);

/// `FUN_1400a6750(tab) -> *entry`: the item entry under the cursor.
type TabItemLookupFn = unsafe extern "system" fn(*mut u8) -> *mut u8;
/// `FUN_140022140(grid) -> int`: the index under the cursor.
type GridCurrentIndexFn = unsafe extern "system" fn(*mut u8) -> i32;
/// `FUN_140021b30(grid, count)`: set the logical item count.
type GridSetItemCountFn = unsafe extern "system" fn(*mut u8, u64);
/// `FUN_1400a4b20(namer, out, cell) -> out`: a cell's element accessor.
type NamerCellLookupFn = unsafe extern "system" fn(*mut u8, *mut u8, *const i32) -> *mut u8;
/// `FUN_1400189f0(dst, src) -> dst`: the copy a namer list entry is made with.
type NamerEntryCopyFn = unsafe extern "system" fn(*mut u8, *const u8) -> *mut u8;
/// `FUN_140027980(out) -> out`: construct the empty element accessor.
type MakeEmptyAccessorFn = unsafe extern "system" fn(*mut u8) -> *mut u8;

static ITEM_LOOKUP_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static CELL_LOOKUP_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// How many times each detour answered out of our own storage, and how many stand-ins were built.
static ITEMS_SERVED: AtomicUsize = AtomicUsize::new(0);
static CELLS_SERVED: AtomicUsize = AtomicUsize::new(0);
static COUNTS_RAISED: AtomicUsize = AtomicUsize::new(0);

/// Whether a pointer is plausibly a live object rather than a small integer or a misalignment.
fn sane(pointer: usize) -> bool {
    pointer >= 0x1_0000 && pointer.is_multiple_of(8)
}

/// Every registered row as the `(action, gate)` pair the game's item vector stores.
///
/// For [`crate::tab`], which builds a group whose vector carries these and nothing else -- as
/// against the System tab's, which carries the shipped three first.
pub(crate) fn registered_entries() -> Vec<(u32, u32)> {
    crate::api::rows_for(crate::api::Tab::Quit)
        .iter()
        .map(|row| (row.action, GATE))
        .collect()
}

/// The System tab's item builder as the game shipped it, bypassing this crate's detour.
///
/// The trampoline when the builder is hooked, and the live entry when it is not -- which is the
/// order that matters rather than a fallback, because a caller wanting the SHIPPED three items must
/// not be handed the detour that appends our rows to them.
///
/// # Safety
///
/// `base` must be the live module base.
pub(crate) unsafe fn item_builder_original(base: usize) -> BuildItemsFn {
    let trampoline = TRAMPOLINE.load(Ordering::Acquire);
    let entry = if trampoline != 0 {
        trampoline
    } else {
        base + ds2_rva::FE_INGAME_TOP_SELECT_SYSTEM_TAB_ITEMS as usize
    };
    // SAFETY: either MinHook's trampoline for this exact site, or the `.pdata` function start
    // recorded in `ds2-rva`; both implement the signature the disassembled entry and exit do.
    unsafe { std::mem::transmute::<usize, BuildItemsFn>(entry) }
}

/// The System tab's item-vector count, or `None` if this group is some other tab.
///
/// **This is the identity check, and it is the same one the item builder runs**, moved to the other
/// side of the copy: the vector has to OPEN with the three `(action, gate)` pairs the System tab
/// ships. The six builders produce six distinct action sets, so those three identify the tab and
/// nothing else in the menu can match them.
///
/// Deliberately allocation-free -- no `read_entries`, no `Vec` -- because this runs on the cursor's
/// own path.
///
/// # Safety
///
/// `tab` must be a constructed `FeGroupInGameGroupSelect` whose item vector has been filled, which
/// is every caller below: they are the game's own readers of that vector.
/// A group's item-vector count, with no claim about which group it is.
///
/// Split out of [`system_tab_count`] for the seventh tab, whose identity is established by a pointer
/// compare rather than by the entries -- its vector carries our actions, so the shipped three are
/// not there to recognise it by.
///
/// # Safety
///
/// `tab` must be a constructed `FeGroupInGameGroupSelect` whose item vector has been filled.
unsafe fn vector_count(tab: *const u8) -> Option<usize> {
    if !sane(tab as usize) {
        return None;
    }
    // SAFETY: the caller guarantees a constructed group, and this is the field the game's own
    // readers address as `[tab + 0x128]`.
    let count = unsafe {
        tab.add(ds2_rva::FE_INGAME_MENU_TAB_ITEM_VECTOR_OFFSET)
            .add(ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_COUNT_OFFSET)
            .cast::<u64>()
            .read()
    } as usize;
    (count <= ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY).then_some(count)
}

unsafe fn system_tab_count(tab: *const u8) -> Option<usize> {
    if !sane(tab as usize) {
        return None;
    }
    // SAFETY: the caller guarantees a constructed group, and this is the field the game's own
    // readers address as `[tab + 0x128]`.
    let vector = unsafe { tab.add(ds2_rva::FE_INGAME_MENU_TAB_ITEM_VECTOR_OFFSET) };
    // SAFETY: as above.
    let count = unsafe {
        vector
            .add(ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_COUNT_OFFSET)
            .cast::<u64>()
            .read()
    } as usize;
    let shipped = ds2_rva::FE_INGAME_MENU_SYSTEM_TAB_ITEMS;
    if count < shipped.len() || count > ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY {
        return None;
    }
    for (index, (action, gate)) in shipped.iter().enumerate() {
        // SAFETY: `index < shipped.len() <= count`, so this entry is live, at the offset the game
        // itself computes for it.
        let entry = unsafe { entry_at(vector.cast_mut(), index) };
        // SAFETY: an entry is two `u32`s, which is how both of the game's own readers split it.
        let (got_action, got_gate) = unsafe {
            (
                entry.cast::<u32>().read(),
                entry.add(4).cast::<u32>().read(),
            )
        };
        if (got_action, got_gate) != (*action, *gate) {
            return None;
        }
    }
    Some(count)
}

/// The entry this crate would serve for the cursor's current position on `tab`, if any.
///
/// `None` for every case the game can answer itself, which is most of them: another tab, an index
/// the vector holds, an index past every registered row. The last one matters -- the game's own
/// answer there is a static `(0xffffffff, 0)`, an action no `case` matches, so declining leaves an
/// inert row rather than a wrong one.
///
/// # Safety
///
/// `tab` is the group the game just passed its own lookup, and `base` the live module base.
unsafe fn our_item_for(base: usize, tab: *mut u8) -> Option<*mut u8> {
    let added = ADDED_ROWS.load(Ordering::Acquire);
    if added == 0 {
        return None;
    }
    // WHICH TAB, AND THEREFORE HOW MANY ROWS ARE THE GAME'S. On the seventh tab none of them are:
    // its vector was built by `crate::tab` and carries our actions from index 0, so the slot is the
    // index. The identity test there is a pointer compare against the group this crate constructed,
    // which is exact -- it does not need the entry-pattern check the shipped tabs are told apart by.
    let seventh = crate::tab::group();
    let shipped = if seventh != 0 && tab as usize == seventh {
        0
    } else {
        ds2_rva::FE_INGAME_MENU_SYSTEM_TAB_ITEMS.len()
    };
    let count = if shipped == 0 {
        // SAFETY: our own group, constructed by the game's own constructor, whose item vector's
        // count field is at the offset every reader of it uses.
        unsafe { vector_count(tab) }?
    } else {
        // SAFETY: the caller's argument is the game's own, on the game's own path.
        unsafe { system_tab_count(tab) }?
    };
    // SAFETY: the RVA is a `.pdata` function start recorded in `ds2-rva`, and the argument is the
    // object the game's own lookup passes it one instruction later.
    let current_index: GridCurrentIndexFn =
        unsafe { std::mem::transmute(base + ds2_rva::FEX_GRID_CURRENT_INDEX as usize) };
    // SAFETY: as above.
    let index = unsafe { current_index(tab) };
    if index < 0 {
        return None;
    }
    let index = index as usize;
    // THE GAME'S OWN ENTRIES WIN. Anything the vector holds is answered by the original, which is
    // what keeps the shipped three rows and the rows that fit behaving exactly as they did.
    if index < count {
        return None;
    }
    let slot = index.checked_sub(shipped)?;
    if slot >= added {
        return None;
    }
    Some(&ITEM_ENTRIES[slot * 2] as *const AtomicU32 as *mut u8)
}

unsafe extern "system" fn item_lookup_detour(tab: *mut u8) -> *mut u8 {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base != 0 {
        // SAFETY: `tab` is the game's own argument and `base` the live module.
        if let Some(entry) = unsafe { our_item_for(base, tab) } {
            let n = ITEMS_SERVED.fetch_add(1, Ordering::Relaxed) + 1;
            // First two only: this fires off the cursor, and the log sink fsyncs per line.
            if n <= 2 {
                // SAFETY: an entry of ours, two `u32`s wide.
                let action = unsafe { entry.cast::<u32>().read() };
                log(format_args!(
                    "{LOG_PREFIX} item served action={action:#x} gate={GATE} served={n} \
                     -- past the game's vector, out of our own entries"
                ));
            }
            return entry;
        }
    }
    let trampoline = ITEM_LOOKUP_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        // A trampoline is published before its site is patched, so this cannot happen -- and null
        // is not the answer if it ever does, because both callers dereference what comes back. The
        // safe answer is the same one the game gives for an index it does not have: an action no
        // `case` matches and a gate of zero, i.e. an inert row.
        return &REFUSED_ENTRY[0] as *const AtomicU32 as *mut u8;
    }
    // SAFETY: MinHook published this trampoline for exactly this site, and the signature is the one
    // the disassembled entry and exit implement.
    let original: TabItemLookupFn =
        unsafe { std::mem::transmute::<usize, TabItemLookupFn>(trampoline) };
    unsafe { original(tab) }
}

/// The stand-in namer that answers for `cell`, if this is our namer and the cell is one of ours.
///
/// # Safety
///
/// `namer` and `cell` are the game's own arguments to its own lookup.
unsafe fn shadow_for(namer: *mut u8, cell: *const i32) -> Option<*mut u8> {
    if namer.is_null() {
        return None;
    }
    // A cell is two `i32`s, so four-aligned rather than eight -- `sane` is the wrong test for it.
    if cell.is_null() || (cell as usize) < 0x1_0000 || !(cell as usize).is_multiple_of(4) {
        return None;
    }
    // SAFETY: the lookup itself reads both of these as `[r8]` and `[r8+4]`.
    let (col, row) = unsafe { (cell.read(), cell.add(1).read()) };
    if col != 0 || row < 0 {
        return None;
    }
    if namer as usize != SYSTEM_TAB_NAMER.load(Ordering::Acquire) {
        return None;
    }
    // SAFETY: our namer, whose count the lookup reads at this same offset.
    let count = unsafe {
        namer
            .add(ds2_rva::FE_SCENE_NAMER_COUNT_FROM_NAMER)
            .cast::<u64>()
            .read()
    } as usize;
    // THE GAME'S OWN LIST WINS, exactly as the item vector does above.
    if (row as usize) < count {
        return None;
    }
    let slot = (row as usize).checked_sub(ds2_rva::FE_QUIT_TAB_CELL_IDS.len())?;
    if slot >= ADDED_ROWS.load(Ordering::Acquire) {
        return None;
    }
    let shadow = SHADOW_NAMERS[slot].load(Ordering::Acquire);
    if shadow == 0 {
        return None;
    }
    Some(shadow as *mut u8)
}

/// Trampolines back to the tab strip's two per-cell lookups, which are two different functions.
///
/// A grid asks its adapter for two elements per cell. On a tab the second is a stub, which is why
/// the row work needed one trampoline; on the strip both are real and both resolve the same entry,
/// so the seventh tab needs both answered. Serving only the first drew its highlight and not its
/// glyph.
static STRIP_CELL_LOOKUP_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static STRIP_CELL_SECOND_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// The tab strip's cell lookup: answer for the one column the strip's namer cannot.
///
/// A separate detour because it is a separate function --
/// [`ds2_rva::FE_SCENE_NAMER_STRIP_CELL_LOOKUP`] is slot 2 of `HLayoutAdapter` where the one
/// [`cell_lookup_detour`] sits on is slot 2 of `VLayoutAdapter`. Detouring only the second is why a
/// seventh tab could be selected and never drawn: the strip asked its own function, which this
/// crate was not on, got an empty accessor for column six, and left the highlight on the sixth tab.
///
/// The axes are swapped with respect to the tab's: here the COLUMN is the index and the row must be
/// zero. Everything else about the stand-in is the same object, so it is handed over with a cell of
/// `(0, 0)` exactly as a row's is.
unsafe extern "system" fn strip_cell_lookup_detour(
    namer: *mut u8,
    out: *mut u8,
    cell: *const i32,
) -> *mut u8 {
    // SAFETY: every argument is the game's own, and the trampoline is this site's.
    unsafe { serve_strip_cell(&STRIP_CELL_LOOKUP_TRAMPOLINE, "cell", namer, out, cell) }
}

/// The strip's second element per cell. Same body, second site -- see
/// [`ds2_rva::FE_SCENE_NAMER_STRIP_CELL_SECOND`].
unsafe extern "system" fn strip_cell_second_detour(
    namer: *mut u8,
    out: *mut u8,
    cell: *const i32,
) -> *mut u8 {
    // SAFETY: as above, with this site's own trampoline.
    unsafe { serve_strip_cell(&STRIP_CELL_SECOND_TRAMPOLINE, "glyph", namer, out, cell) }
}

/// Answer one of the strip's two per-cell lookups out of the seventh tab's stand-in.
///
/// # Safety
///
/// `trampoline` must hold the published trampoline for the site being answered, and the three
/// remaining arguments must be that site's own.
unsafe fn serve_strip_cell(
    trampoline: &AtomicUsize,
    what: &str,
    namer: *mut u8,
    out: *mut u8,
    cell: *const i32,
) -> *mut u8 {
    let trampoline = trampoline.load(Ordering::Acquire);
    if trampoline == 0 {
        // Published before the site is patched, so unreachable. As the tab's: hand back the game's
        // own empty accessor rather than the caller's uninitialised buffer, whose vtable slot 0 is
        // the very next thing it calls.
        let base = MODULE_BASE.load(Ordering::Acquire);
        if base == 0 {
            return out;
        }
        // SAFETY: the RVA is a `.pdata` function start recorded in `ds2-rva`, and `out` is the
        // caller's own accessor buffer.
        let make_empty: MakeEmptyAccessorFn =
            unsafe { std::mem::transmute(base + ds2_rva::FE_SCENE_ACCESSOR_MAKE_EMPTY as usize) };
        // SAFETY: as above.
        return unsafe { make_empty(out) };
    }
    // SAFETY: MinHook published this trampoline for exactly this site, and the signature is the one
    // the disassembled entry and exit implement.
    let original: NamerCellLookupFn =
        unsafe { std::mem::transmute::<usize, NamerCellLookupFn>(trampoline) };
    let strip = STRIP_NAMER.load(Ordering::Acquire);
    let shadow = STRIP_SHADOW.load(Ordering::Acquire);
    let wanted = !namer.is_null()
        && strip != 0
        && namer as usize == strip
        && shadow != 0
        && !cell.is_null()
        && (cell as usize) >= 0x1_0000
        && (cell as usize).is_multiple_of(4)
        // SAFETY: the lookup itself reads both of these, as `[r8]` and `[r8+4]`.
        && unsafe { cell.add(1).read() } == 0
        && unsafe { cell.read() } as usize == ds2_rva::FE_INGAME_TOP_SELECT_TABS;
    if wanted {
        let own_cell = [0i32, 0i32];
        let n = STRIP_CELLS_SERVED.fetch_add(1, Ordering::Relaxed) + 1;
        if n <= 4 {
            log(format_args!(
                "{LOG_PREFIX} strip {what} served column={} stand-in=0x{shadow:016x} served={n} \
                 -- the seventh tab's own, out of our own stand-in",
                ds2_rva::FE_INGAME_TOP_SELECT_TABS
            ));
        }
        // SAFETY: the stand-in carries the three fields this function reads -- proxy at `+0x10`,
        // one entry at `+0x18`, count `1` at `+0x140` -- and `own_cell` is the pair it reads a cell
        // as.
        return unsafe { original(shadow as *mut u8, out, own_cell.as_ptr()) };
    }
    // SAFETY: every argument is the game's own, passed through unchanged.
    unsafe { original(namer, out, cell) }
}

unsafe extern "system" fn cell_lookup_detour(
    namer: *mut u8,
    out: *mut u8,
    cell: *const i32,
) -> *mut u8 {
    let trampoline = CELL_LOOKUP_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        // Published before the patch, so unreachable -- and handing back the UNTOUCHED buffer is
        // not the answer if it ever is reached, because the caller's next act is to call slot 0 of
        // whatever vtable is in it, and that buffer is its own uninitialised stack. The game's own
        // "nothing here" accessor is a function, so construct one: its slot 0 resolves to null and
        // the grid's bind reads that as the end of the row.
        let base = MODULE_BASE.load(Ordering::Acquire);
        if base == 0 {
            return out;
        }
        // SAFETY: the RVA is a `.pdata` function start recorded in `ds2-rva`, and `out` is the
        // caller's own accessor buffer -- the same one the original would have constructed into.
        let make_empty: MakeEmptyAccessorFn =
            unsafe { std::mem::transmute(base + ds2_rva::FE_SCENE_ACCESSOR_MAKE_EMPTY as usize) };
        // SAFETY: as above.
        return unsafe { make_empty(out) };
    }
    // SAFETY: MinHook published this trampoline for exactly this site, and the signature is the one
    // the disassembled entry and exit implement -- `out` in RDX, returned in RAX.
    let original: NamerCellLookupFn =
        unsafe { std::mem::transmute::<usize, NamerCellLookupFn>(trampoline) };
    // SAFETY: both are the game's own arguments.
    if let Some(shadow) = unsafe { shadow_for(namer, cell) } {
        // OUR STAND-IN, AND THE GAME'S OWN CODE. The cell is rewritten to `(0, 0)` because the
        // stand-in carries one entry at row 0 -- which is what keeps the entry away from the count
        // field the lookup's own stride would otherwise reach.
        let own_cell = [0i32, 0i32];
        let n = CELLS_SERVED.fetch_add(1, Ordering::Relaxed) + 1;
        if n <= 2 {
            log(format_args!(
                "{LOG_PREFIX} cell served namer=0x{:016x} stand-in=0x{:016x} served={n} \
                 -- past the game's namer list, out of our own stand-in",
                namer as usize, shadow as usize
            ));
        }
        // SAFETY: the stand-in carries the three fields this function reads -- proxy, one entry,
        // count `1` -- and `own_cell` is the two `i32`s it reads a cell as.
        return unsafe { original(shadow, out, own_cell.as_ptr()) };
    }
    unsafe { original(namer, out, cell) }
}

/// Fill the stand-in for `slot` from the namer the game just built.
///
/// The ENTRY is copied through the game's own [`ds2_rva::FE_SCENE_NAMER_ENTRY_COPY`] rather than
/// with a `memcpy`, for the same reason the pushed clones are: an entry is a
/// `DLFixedVector<u32, 8>` and its copy is the function that knows how many of the eight are live.
///
/// # Safety
///
/// `namer` is the constructed namer, `source` one of its live entries, and `base` the live module.
unsafe fn fill_shadow(base: usize, namer: usize, source: *const u8, slot: usize, id: u32) -> bool {
    let shadow = SHADOW_NAMERS[slot].load(Ordering::Acquire);
    if shadow == 0 {
        return false;
    }
    // The alignment the lookup's padding term depends on. Checked rather than assumed, because a
    // misaligned stand-in would hand the game a path shifted by up to seven bytes.
    if !(shadow + ds2_rva::FE_SCENE_NAMER_LIST_OFFSET).is_multiple_of(8) {
        return false;
    }
    // SAFETY: the namer is live and this is the field its own lookup reads the scene proxy from.
    let proxy = unsafe {
        (namer as *const u8)
            .add(ds2_rva::FE_SCENE_NAMER_PROXY_OFFSET)
            .cast::<usize>()
            .read()
    };
    if !sane(proxy) {
        return false;
    }
    // SAFETY: the RVA is a `.pdata` function start recorded in `ds2-rva`; `source` is a live entry
    // and the destination is a `FE_SCENE_NAMER_SHADOW_SIZE` buffer this crate owns.
    let copy: NamerEntryCopyFn =
        unsafe { std::mem::transmute(base + ds2_rva::FE_SCENE_NAMER_ENTRY_COPY as usize) };
    // SAFETY: every write below is inside the buffer, at the offsets the lookup reads.
    unsafe {
        let shadow = shadow as *mut u8;
        shadow.write_bytes(0, ds2_rva::FE_SCENE_NAMER_SHADOW_SIZE);
        shadow
            .add(ds2_rva::FE_SCENE_NAMER_PROXY_OFFSET)
            .cast::<usize>()
            .write(proxy);
        let entry = shadow.add(ds2_rva::FE_SCENE_NAMER_LIST_OFFSET);
        copy(entry, source);
        entry
            .add(ds2_rva::FE_SCENE_NAMER_ENTRY_ID_OFFSET)
            .cast::<u32>()
            .write(id);
        shadow
            .add(ds2_rva::FE_SCENE_NAMER_COUNT_FROM_NAMER)
            .cast::<u64>()
            .write(1);
    }
    true
}

/// Trampoline back to the per-tab init, and how many times the probe has reported.
static TAB_INIT_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static TAB_INIT_REPORTS: AtomicUsize = AtomicUsize::new(0);

/// The per-tab init: `void init(tab)`, `this` in RCX. Prologue `40 53 48 81 ec b0 00 00 00`.
type TabInitFn = unsafe extern "system" fn(*mut u8);

/// Read the three numbers that decide whether a row is drawn, AFTER the game has set them.
///
/// They come from three different places and this is the only way to see them disagree:
///
/// * the item count is what `FUN_140021b30` was handed -- the item vector's length, which the
///   append above moves;
/// * the column extent is a census of the cell elements the layout bind FOUND, which nothing in
///   code moves;
/// * the scroll object's visible count is a third number again, and `FUN_140021b30` compares the
///   total against it to decide whether to show a scrollbar.
///
/// If visible is 3 while the item count is 4, this grid is a VIRTUALISED list and the missing row
/// is a scroll that did not happen. If the extent is 3 and there is no scroll concept, the row has
/// no cell and only the layout can supply one. Those two futures cost very different amounts and
/// nothing short of this tells them apart.
///
/// # Safety
///
/// `tab` must be a constructed `FeGroupInGameGroupSelect` whose init has already run.
unsafe fn report_tab(tab: *mut u8) {
    if tab.is_null() {
        return;
    }
    // SAFETY: the original init has just run against this pointer and wrote every field below.
    let (items, cols, rows, scroll) = unsafe {
        (
            tab.add(ds2_rva::FEX_GRID_ITEM_COUNT_OFFSET)
                .cast::<u32>()
                .read(),
            tab.add(ds2_rva::FEX_GRID_COL_EXTENT_OFFSET)
                .cast::<u32>()
                .read(),
            tab.add(ds2_rva::FEX_GRID_ROW_EXTENT_OFFSET)
                .cast::<u32>()
                .read(),
            tab.add(ds2_rva::FEX_GRID_SCROLL_OFFSET)
                .cast::<usize>()
                .read(),
        )
    };
    // The scroll object is a pointer the grid may legitimately not have; a null one is reported as
    // such rather than dereferenced, because "this grid does not scroll" is itself the answer to
    // half the question.
    let (visible, total) = if scroll == 0 {
        (None, None)
    } else {
        // SAFETY: non-null, and `FUN_140021b30` reads and writes these two fields on every call.
        unsafe {
            (
                Some(
                    (scroll as *const u8)
                        .add(ds2_rva::FEX_GRID_SCROLL_VISIBLE_OFFSET)
                        .cast::<u32>()
                        .read(),
                ),
                Some(
                    (scroll as *const u8)
                        .add(ds2_rva::FEX_GRID_SCROLL_TOTAL_OFFSET)
                        .cast::<u32>()
                        .read(),
                ),
            )
        }
    };
    // THE NAMER'S IDENTITY. `FUN_140022160` asks the object at `grid+0xf0` for a cell's element,
    // and which CLASS that is decides how the id for an absent cell is generated. A vtable address
    // walks back to an RTTI type descriptor offline with `scripts/ds2-rtti.py`, so one logged
    // pointer turns an open question into a class name.
    // SAFETY: the original init has run, so the grid's namer is live.
    let namer = unsafe { tab.add(0xf0).cast::<usize>().read() };
    // SAFETY: a namer is polymorphic, so its first qword is its vtable.
    let namer_vtable = if namer == 0 {
        0
    } else {
        unsafe { (namer as *const usize).read() }
    };
    let n = TAB_INIT_REPORTS.fetch_add(1, Ordering::Relaxed) + 1;
    // THE AXIS MATTERS AND THIS GOT IT WRONG ONCE. These tabs measure one COLUMN by N ROWS -- the
    // five tabs this crate does not touch all report `col-extent=1` and `row-extent == items` --
    // so the number of authored cells is the ROW extent. Comparing against the column extent said
    // "NO CELL" for every tab including the four that are perfectly fine.
    let cells = rows;
    let verdict = if items <= cells {
        "ok: every item has a cell"
    } else if visible.is_some_and(|v| v < items) {
        "VIRTUALISED: visible<items, the row needs a SCROLL"
    } else {
        "NO CELL: items>cells and there is no scroll to cover it"
    };
    log(format_args!(
        "{LOG_PREFIX} tab tab=0x{:016x} items={items} col-extent={cols} row-extent={rows} \
         cells={cells} scroll=0x{scroll:x} visible={visible:?} total={total:?} \
         namer=0x{namer:x} namer-vtable=0x{namer_vtable:x} report={n} -- {verdict}",
        tab as usize
    ));
}

/// `FexGridControl::indexToCell(grid, out_cell, index)` and `cellToElement(grid, out, cell)`.
///
/// Called rather than reimplemented, and that is the point: the id an appended cell WOULD have is
/// whatever the game's own namer produces for it, and asking the namer is the only way to learn it
/// that cannot be wrong. Reimplementing the naming would be inventing the answer we came for.
type IndexToCellFn = unsafe extern "system" fn(*mut u8, *mut u8, i32) -> *mut u8;
type CellToElementFn = unsafe extern "system" fn(*mut u8, *mut u8, *mut u8) -> *mut u8;

/// Hex, because the accessor's shape is not known and a decoded field would be a guess.
fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && i % 8 == 0 {
            out.push(' ');
        }
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Ask the grid for the element accessor of each cell, INCLUDING one past the last.
///
/// The one past the last is the whole reason this exists. Cells 0..n-1 are the records that already
/// live in the `.flo`; cell n is the one that does not, and its accessor still carries the id the
/// namer would have looked for. That id is what a new record has to be keyed by, and it cannot be
/// derived from the file -- only from the namer.
///
/// # Safety
///
/// `tab` must be a bound grid, and `base` the live module base. Both callees are pure accessor
/// builders that write only into the caller's buffers.
unsafe fn report_cell_ids(base: usize, tab: *mut u8, cells: u32) {
    // SAFETY: both RVAs are `.pdata` function starts recorded in `ds2-rva`.
    let index_to_cell: IndexToCellFn =
        unsafe { std::mem::transmute(base + ds2_rva::FEX_GRID_INDEX_TO_CELL as usize) };
    // SAFETY: same.
    let cell_to_element: CellToElementFn =
        unsafe { std::mem::transmute(base + ds2_rva::FEX_GRID_CELL_TO_ELEMENT as usize) };
    // 160 bytes because the game's own callers of these give them 144-byte stack buffers; the
    // extra 16 is slack, not a guess about the type.
    for index in 0..(cells + 1).min(8) {
        let mut coords = [0u8; 160];
        let mut accessor = [0u8; 160];
        // SAFETY: the buffers are at least as large as the ones the game gives these functions.
        unsafe {
            index_to_cell(tab, coords.as_mut_ptr(), index as i32);
            cell_to_element(tab, accessor.as_mut_ptr(), coords.as_mut_ptr());
        }
        let past = if index >= cells {
            " ONE-PAST-THE-END"
        } else {
            ""
        };
        log(format_args!(
            "{LOG_PREFIX} cell tab=0x{:016x} index={index} coords={} accessor={}{past}",
            tab as usize,
            hex(&coords[..8]),
            hex(&accessor[..48])
        ));
    }
}

/// The push a cell namer's list takes: `fn(&namer[0x18], src_entry)`.
type NamerPushFn = unsafe extern "system" fn(*mut u8, *mut u8);

/// The namer constructor: `fn(out_ref, allocator) -> out_ref`, leaving the namer in `*out_ref`.
///
/// **IT RETURNS ITS FIRST ARGUMENT, AND THE DECOMPILER SAYS IT DOES NOT.** Ghidra types it `void`;
/// the disassembly ends `mov rax, r14` with `r14` holding the `rcx` saved in the prologue, and the
/// TopSelect constructor passes that return straight into `FeGroupInGameGroupSelect`'s constructor,
/// whose first instruction dereferences it.
///
/// A detour declared `-> ()` therefore hands the game whatever Rust left in RAX. Measured, that was
/// `1`, and the game died reading address `1` inside `0x1400a40f5` -- a crash with our DLL nowhere
/// in the stack, on the very next call. Read the RET, not the decompiler's signature.
type NamerCtorFn = unsafe extern "system" fn(*mut usize, *mut u8) -> *mut usize;

static NAMER_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static CELLS_ADDED: AtomicUsize = AtomicUsize::new(0);

/// Name the fourth cell, so the grid asks the layout for the record `layout.rs` just added.
///
/// The namer holds the cell path of every row it will draw: four shared components and then one id
/// per row. The grid walks that list, resolves each path against the scene, and its extent is a
/// census of the ones that came back non-null -- which is why the appended item had no row.
///
/// This appends a fourth entry naming [`ds2_rva::FLO_ADDED_ROW_ID`]. **It is one half of a pair.**
/// On its own it names an element that does not exist and the extent stays at three -- which is
/// exactly what an earlier run measured, and which is now the control for this one: if the extent
/// reads four, the container substitution worked, because nothing else changed.
///
/// The clone is a copy of the last shipped entry with one field rewritten, not an entry assembled
/// from scratch. The slack between the ids and the length is uninitialised stack and differs
/// between two entries the game built back to back, so only the named fields mean anything.
///
/// # Safety
///
/// `namer` is the object the original constructor just returned, and `base` the live module base.
/// The push takes the source entry by reference the same way the original loop does.
unsafe fn name_added_cell(base: usize, namer: usize) -> bool {
    if namer == 0 {
        return false;
    }
    let refuse = |why: core::fmt::Arguments<'_>| {
        log(format_args!(
            "{LOG_PREFIX} cell REFUSED {why} -- the tab keeps its three rows"
        ));
        false
    };
    let list = namer + ds2_rva::FE_SCENE_NAMER_LIST_OFFSET;
    let count_at = (list + ds2_rva::FE_SCENE_NAMER_COUNT_OFFSET) as *mut u64;
    // SAFETY: the original constructor has just filled this vector.
    let count = unsafe { count_at.read() } as usize;
    if count != ds2_rva::FE_QUIT_TAB_CELL_IDS.len() {
        return refuse(format_args!(
            "count={count}, expected {}",
            ds2_rva::FE_QUIT_TAB_CELL_IDS.len()
        ));
    }
    let all = crate::api::rows_for(crate::api::Tab::Quit);
    // WHICH ROWS NEED A CELL HERE, which depends on whose namer this is. Three cases, and the
    // middle one is the seventh tab's:
    //
    //   not armed          the System tab carries the added rows, so every one needs a cell of its
    //                      own beyond the shipped three.
    //   armed, building    this is the seventh tab's namer. Its rows ARE the tab, so the game's own
    //                      three cells already draw rows 0..2 and only the rest need one.
    //   armed, not         the System tab, which keeps exactly what the game shipped.
    let rows: Vec<_> = if !crate::tab::armed() {
        all
    } else if crate::tab::building() {
        all.into_iter().skip(count).collect()
    } else {
        Vec::new()
    };
    if rows.is_empty() {
        return false;
    }
    // AS MANY CELLS AS THE GAME'S OWN LIST HOLDS, AND STAND-INS FOR THE REST. Running out of list
    // is the split, not a refusal -- see [`SHADOW_NAMERS`]. The game's slots are filled first for
    // the same reason the item vector's are: the cell-lookup detour declines for anything already
    // in the list, so those cells go through the game's own path with nothing of ours in it.
    let fits = ds2_rva::FE_SCENE_NAMER_LIST_CAPACITY
        .saturating_sub(count)
        .min(rows.len());
    let stride = ds2_rva::FE_SCENE_NAMER_ENTRY_STRIDE;
    let padding = (0u32.wrapping_sub(list as u32) & 7) as usize;
    let entry = |i: usize| (list + padding + i * stride) as *mut u8;
    let dword = |e: *mut u8, off: usize| {
        // SAFETY: `off` is inside an entry the caller has established is live.
        unsafe { e.add(off).cast::<u32>().read() }
    };

    // VERIFY THE ENTRY THE CLONE IS TAKEN FROM, field by field, against what the namer's own
    // constructor put there. Appending to some other tab's namer would produce a run that looks
    // exactly like a result and is about nothing.
    let last = entry(count - 1);
    for (i, want) in ds2_rva::FE_QUIT_TAB_BASE_PATH.iter().enumerate() {
        let got = dword(last, i * 4);
        if got != *want {
            return refuse(format_args!(
                "entry[{}] component {i} is {got:#x}, expected {want:#x}",
                count - 1
            ));
        }
    }
    let want_id = ds2_rva::FE_QUIT_TAB_CELL_IDS[count - 1];
    let got_id = dword(last, ds2_rva::FE_SCENE_NAMER_ENTRY_ID_OFFSET);
    if got_id != want_id {
        return refuse(format_args!(
            "entry[{}] id is {got_id:#x}, expected {want_id:#x}",
            count - 1
        ));
    }
    let got_len = dword(last, ds2_rva::FE_SCENE_NAMER_ENTRY_LEN_OFFSET);
    if got_len != ds2_rva::FE_SCENE_NAMER_ENTRY_LEN {
        return refuse(format_args!(
            "entry[{}] length is {got_len}, expected {}",
            count - 1,
            ds2_rva::FE_SCENE_NAMER_ENTRY_LEN
        ));
    }

    // SAFETY: the RVA is a `.pdata` function start recorded in `ds2-rva`, and the source is a live
    // element of the vector it is pushed back into.
    let push = unsafe {
        std::mem::transmute::<usize, NamerPushFn>(base + ds2_rva::FE_SCENE_NAMER_PUSH as usize)
    };
    // ON A TAB OF OUR OWN, THE GAME'S THREE CELLS BECOME OURS TOO. They are already in the list and
    // they draw the top three row positions, and left alone they draw the game's own rows -- so the
    // seventh tab's first row would have read "Game Options" and fired quit-to-desktop. Only the id
    // is rewritten; the path components around it stay the ones the game's own builder wrote.
    let mut reused = Vec::new();
    if crate::tab::building() {
        let head = crate::api::rows_for(crate::api::Tab::Quit);
        for (index, row) in head.iter().take(count).enumerate() {
            // SAFETY: `index < count`, so this entry is live, and the offset is the id field the
            // verification above just read out of entry `count - 1`.
            unsafe {
                entry(index)
                    .add(ds2_rva::FE_SCENE_NAMER_ENTRY_ID_OFFSET)
                    .cast::<u32>()
                    .write(row.row_id);
            }
            reused.push(format!("{index}={:#x}", row.row_id));
        }
        // AND THE SUBTREE THEY RESOLVE UNDER. Every entry the builder wrote carries
        // `FE_QUIT_TAB_BASE_PATH`, whose second component is the System tab's subtree. The seventh
        // tab draws into a copy of that subtree with a different element id, so each of these paths
        // has to name it or the cell resolves against the System tab's container and this tab draws
        // the System tab's rows. One component; everything either side of it is the game's own.
        //
        // Done before the appends below, so the clones they take inherit it.
        for index in 0..count {
            // SAFETY: `index < count` and the offset is component 1 of an entry whose first four
            // components were verified against `FE_QUIT_TAB_BASE_PATH` above.
            unsafe {
                entry(index)
                    .add(ds2_rva::FE_SCENE_NAMER_ENTRY_SUBTREE_OFFSET)
                    .cast::<u32>()
                    .write(ds2_rva::FLO_ADDED_TAB_SUBTREE_ID);
            }
        }
    }
    // ONE ENTRY PER REGISTERED ROW, each a clone of the last SHIPPED entry with its id rewritten.
    // Cloned rather than assembled: the slack between the named fields is the unused tail of a
    // `DLFixedVector<u32, 8>` and differs between two entries the game built back to back, so only
    // the named fields mean anything and inventing the rest would be inventing the answer.
    let mut named = Vec::with_capacity(rows.len());
    let mut shadowed = 0usize;
    for (slot, row) in rows.iter().enumerate() {
        let at = count + slot;
        if slot < fits {
            // SAFETY: `at < capacity`, which is the same bound the push enforces itself.
            unsafe { push(list as *mut u8, last) };
            // SAFETY: the push just made element `at` live, and the offset is inside it.
            unsafe {
                entry(at)
                    .add(ds2_rva::FE_SCENE_NAMER_ENTRY_ID_OFFSET)
                    .cast::<u32>()
                    .write(row.row_id);
            }
            named.push(format!("{}={:#x}", at, row.row_id));
            continue;
        }
        // PAST THE GAME'S LIST. The stand-in carries the same cloned entry at ROW 0 of a list of
        // its own, and the cell-lookup detour hands it over with a cell of `(0, 0)`.
        // SAFETY: `namer` is the constructed namer and `last` one of its live entries.
        if unsafe { fill_shadow(base, namer, last, slot, row.row_id) } {
            named.push(format!("{}={:#x}*", at, row.row_id));
            shadowed += 1;
        } else {
            return refuse(format_args!(
                "slot {slot} wanted a stand-in namer and there is none -- \
                 install() allocates one per slot before any hook is enabled"
            ));
        }
    }
    // PUBLISHED LAST, so the cell-lookup detour cannot see a namer whose stand-ins are half built,
    // and so the count raise in `tab_init_detour` only fires for rows that now have a cell.
    NAMED_ROWS.store(rows.len(), Ordering::Release);
    SYSTEM_TAB_NAMER.store(namer, Ordering::Release);
    let n = CELLS_ADDED.fetch_add(1, Ordering::Relaxed) + 1;
    log(format_args!(
        "{LOG_PREFIX} cells named{} [{}] container={:#x} list={count}->{} of-capacity={} \
         stand-ins={shadowed} namer=0x{namer:016x} additions={n} \
         -- `*` is ours; row-extent {} on the tab line means the layout answered",
        if reused.is_empty() {
            String::new()
        } else {
            format!(
                " reusing-the-games-cells [{}] under-subtree={:#x}",
                reused.join(" "),
                ds2_rva::FLO_ADDED_TAB_SUBTREE_ID
            )
        },
        named.join(" "),
        ds2_rva::FE_QUIT_TAB_BASE_PATH[3],
        count + fits,
        ds2_rva::FE_SCENE_NAMER_LIST_CAPACITY,
        count + rows.len()
    ));
    true
}

unsafe extern "system" fn namer_detour(out: *mut usize, allocator: *mut u8) -> *mut usize {
    let trampoline = NAMER_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        // The caller dereferences what comes back, so returning the out pointer is the only safe
        // answer even on a path that cannot happen.
        return out;
    }
    // SAFETY: MinHook published this trampoline for exactly this site.
    let original: NamerCtorFn = unsafe { std::mem::transmute::<usize, NamerCtorFn>(trampoline) };
    // RETURNED, not discarded. See the note on `NamerCtorFn`.
    let returned = unsafe { original(out, allocator) };
    if out.is_null() {
        return returned;
    }
    // SAFETY: the original writes the constructed namer here.
    let namer = unsafe { out.read() };
    // CLEARED FIRST. A namer is built on every pause-menu open, and the cell-lookup detour must not
    // answer for the one being replaced -- nor for this one until its stand-ins are filled. The
    // stores back to these are the LAST thing `name_added_cell` does, and only on success.
    SYSTEM_TAB_NAMER.store(0, Ordering::Release);
    NAMED_ROWS.store(0, Ordering::Release);
    let base = ds2_game_base::mem::game_module_base().unwrap_or(0);
    if base != 0 {
        unsafe { name_added_cell(base, namer) };
    }
    returned
}

/// Raise the cursor's bound to the rows this tab actually has, AFTER the game has set it.
///
/// The init does three things in order: bind the grid to the layout, call
/// [`ds2_rva::FEX_GRID_SET_ITEM_COUNT`] with the item vector's count, and run the availability pass.
/// The bind's extent now counts our cells too, but the count it was handed is the vector's -- which
/// stops at five and cannot reach the rows served out of [`ITEM_ENTRIES`].
///
/// **The order is the whole safety argument, and it is why this runs AFTER the original rather than
/// patching the count the original reads.** The availability pass bounds-checks every index against
/// the VECTOR's count and falls back to a static `(0xffffffff, 0)` above it -- gate `0`, which it
/// skips. Having already run at the unraised count, it never sees the raised one at all; and even if
/// it ran again it would read the static rather than uninitialised storage. Raising the vector's own
/// count field instead would have handed that pass an unwritten gate.
///
/// The count is set through the game's own setter, not by storing into the field, because that
/// setter also drives the scrollbar -- null on these tabs today, and not this crate's to assume.
unsafe extern "system" fn tab_init_detour(tab: *mut u8) {
    let trampoline = TAB_INIT_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for exactly this site, and the signature is the
        // one the disassembled entry implements.
        let original: TabInitFn = unsafe { std::mem::transmute::<usize, TabInitFn>(trampoline) };
        unsafe { original(tab) };
    }
    // ROWS THAT HAVE A CELL, not rows that are registered. If the namer detour refused this open,
    // this is zero and the cursor keeps the bound the game gave it -- a row the cursor can reach and
    // the layout never drew is the exact failure this crate was written to stop producing.
    let added = NAMED_ROWS.load(Ordering::Acquire);
    if added == 0 {
        return;
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return;
    }
    // WHICH TAB, and once there is a seventh it is the only one this applies to. The init runs for
    // every tab, and raising a cursor bound on a tab whose cells were never added lets its cursor
    // walk onto rows that are not there -- which is what the System tab got on the first run of
    // `crate::tab`, a bound of seven over six drawn cells.
    let seventh = crate::tab::group();
    // HOW MANY ROWS THIS TAB HAS ALTOGETHER, and the two tabs answer differently. The System tab's
    // is its three shipped rows plus ours; the seventh tab ships nothing, so its total is ours
    // alone. One expression covering both said `shipped + added` for each, which on the seventh tab
    // is three more items than there are cells -- a cursor bound of seven over four drawn rows, and
    // a confirm that resolves an entry the vector does not have.
    let (count, want) = if crate::tab::armed() {
        if seventh == 0 || tab as usize != seventh {
            return;
        }
        // SAFETY: our own group, whose vector the game's own constructor filled.
        match unsafe { vector_count(tab) } {
            Some(count) => (count, added),
            None => return,
        }
    } else {
        // SAFETY: the original init has just run against this pointer, so the group is constructed.
        match unsafe { system_tab_count(tab) } {
            Some(count) => (
                count,
                ds2_rva::FE_INGAME_MENU_SYSTEM_TAB_ITEMS.len() + added,
            ),
            None => return,
        }
    };
    if want <= count {
        // Every row fitted in the game's own vector, so its own count already says so.
        return;
    }
    // SAFETY: the RVA is a `.pdata` function start recorded in `ds2-rva`, and `tab` is the same
    // object the original just passed it.
    let set_item_count: GridSetItemCountFn =
        unsafe { std::mem::transmute(base + ds2_rva::FEX_GRID_SET_ITEM_COUNT as usize) };
    // SAFETY: as above; the second argument is the count, which the setter narrows to 32 bits.
    unsafe { set_item_count(tab, want as u64) };
    let n = COUNTS_RAISED.fetch_add(1, Ordering::Relaxed) + 1;
    if n <= 2 {
        // SAFETY: the setter has just written this field.
        let (items, cells) = unsafe {
            (
                tab.add(ds2_rva::FEX_GRID_ITEM_COUNT_OFFSET)
                    .cast::<u32>()
                    .read(),
                tab.add(ds2_rva::FEX_GRID_ROW_EXTENT_OFFSET)
                    .cast::<u32>()
                    .read(),
            )
        };
        log(format_args!(
            "{LOG_PREFIX} item count raised tab=0x{:016x} vector={count} -> items={items} \
             row-extent={cells} raises={n}{}",
            tab as usize,
            if items as usize == want && cells as usize >= want {
                ""
            } else {
                " -- MISMATCH: the cursor and the drawn cells disagree, so a row is reachable and \
                 invisible or the reverse"
            }
        ));
    }
}

/// Check the bytes at a site against the ones recorded, then patch it. Refuses rather than patches.
///
/// **The prologue check is the only defence against this table being read against a different
/// build.** An RVA is a number and will happily point into the middle of some other function, which
/// MinHook would detour just as willingly; the difference between "the mod did nothing" and "the mod
/// corrupted an unrelated function" is this comparison.
///
/// The trampoline is published BEFORE the site is patched, so a detour that fires immediately
/// cannot observe a zero.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. `rva` must be a `.pdata` function start
/// recorded in `ds2-rva` and `base` the live module base.
pub(crate) unsafe fn hook_site(
    base: usize,
    rva: u32,
    prologue: &[u8],
    detour: *mut c_void,
    trampoline: &AtomicUsize,
    what: &str,
) -> bool {
    let site = base + rva as usize;
    // SAFETY: `site` is inside the loaded image's `.text` -- a `.pdata` function start resolved
    // against the live base -- so `prologue.len()` bytes are readable there.
    let found = unsafe { std::slice::from_raw_parts(site as *const u8, prologue.len()) };
    if found != prologue {
        log(format_args!(
            "{LOG_PREFIX} {what} NOT installed stage=prologue rva=0x{rva:08x} va=0x{site:016x} \
             expected={prologue:02x?} found={found:02x?}"
        ));
        return false;
    }
    // SAFETY: the bytes at the site are the recorded ones, so this is the function it claims to be.
    let hook = match unsafe { MhHook::new(site as *mut c_void, detour) } {
        Ok(hook) => hook,
        Err(status) => {
            log(format_args!(
                "{LOG_PREFIX} {what} NOT installed stage=MH_CreateHook rva=0x{rva:08x} \
                 va=0x{site:016x} status={status:?}"
            ));
            return false;
        }
    };
    trampoline.store(hook.trampoline() as usize, Ordering::Release);
    // SAFETY: the hook was just created for this exact address.
    let status = unsafe { MH_EnableHook(site as *mut c_void) };
    if status != MH_STATUS::MH_OK {
        log(format_args!(
            "{LOG_PREFIX} {what} NOT installed stage=MH_EnableHook rva=0x{rva:08x} \
             va=0x{site:016x} status={status:?}"
        ));
        return false;
    }
    // The handle falls out of scope here. `MhHook` has no `Drop`, so that does NOT remove the hook
    // -- the patch stays for the life of the process, which is what is wanted.
    log(format_args!(
        "{LOG_PREFIX} {what} hooked rva=0x{rva:08x} va=0x{site:016x}"
    ));
    true
}

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The builder is now detoured.
    pub installed: bool,
}

/// Detour the quit tab's item builder. Call from the post-Arxan callback, never `DllMain`.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Must run after `neuter_arxan` (or after
/// `schedule_after_arxan`), which in practice means the loader's Arxan callback. It does not have
/// to be early: the site is only reached when `FeGroupInGameTopSelect` is constructed, which is
/// long after the entry point.
pub unsafe fn install() -> Outcome {
    // SEALED FIRST, so a row registered from another thread while the hooks are going in cannot be
    // half-applied -- named by the namer but absent from the item vector, or the reverse.
    crate::api::seal();
    // TWO REASONS TO INSTALL, AND THEY ARE INDEPENDENT. A row needs the item-vector, dispatch,
    // namer and caption detours; a TICK needs only the per-frame update. Gating both on `any()` --
    // which counts rows -- made `add_tick` silently inert for any crate that registered a callback
    // and no row, which is a public entry point that looks broken rather than absent.
    let rows = crate::api::any();
    let ticks = crate::caption::has_ticks();
    if !rows && !ticks {
        log(format_args!(
            "{LOG_PREFIX} nothing registered -- no rows, no ticks, no hooks, the shipped menu is \
             untouched"
        ));
        return Outcome { installed: false };
    }
    let base = match ds2_game_base::mem::game_module_base() {
        Ok(base) => base,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} install-failed stage=module-base error={error}"
            ));
            return Outcome { installed: false };
        }
    };

    // MinHook is statically linked into this DLL, so nothing else shares this instance and
    // ALREADY_INITIALIZED can only mean this ran twice. Treat it as success.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=MH_Initialize status={status:?}"
        ));
        return Outcome { installed: false };
    }

    if !rows {
        // TICK ONLY: not one byte of the shipped menu's own row machinery is patched. The pause
        // menu draws exactly what it shipped with; the only detour is the per-frame update, which
        // runs the original first.
        unsafe { crate::caption::install_tick(base) };
        log(format_args!(
            "{LOG_PREFIX} tick-only install -- no rows registered, the shipped menu is untouched"
        ));
        return Outcome { installed: true };
    }

    // Published before any detour that reads it, and never changed again.
    MODULE_BASE.store(base, Ordering::Release);

    // OUR OWN STORAGE, AND THE TWO DETOURS THAT READ IT, BEFORE ANYTHING ELSE IS PATCHED.
    //
    // All or nothing, and first: every row past the game's own five item slots and six cell slots
    // depends on both of these, and a half-installed pair is the worst outcome available -- a cursor
    // that reaches a row with no cell, or a drawn row the cursor cannot reach. Refusing here leaves
    // the shipped menu completely untouched, because not one other site has been patched yet.
    let rows = crate::api::rows_for(crate::api::Tab::Quit);
    for (slot, row) in rows.iter().enumerate() {
        ITEM_ENTRIES[slot * 2].store(row.action, Ordering::Relaxed);
        ITEM_ENTRIES[slot * 2 + 1].store(GATE, Ordering::Relaxed);
        // LEAKED ON PURPOSE, one per slot, once for the process. The namer is rebuilt on every
        // pause-menu open and only its CONTENTS are refreshed, so nothing here grows with use.
        let shadow = Box::leak(Box::new(ShadowNamer(
            [0; ds2_rva::FE_SCENE_NAMER_SHADOW_SIZE],
        )));
        SHADOW_NAMERS[slot].store(shadow as *mut ShadowNamer as usize, Ordering::Release);
    }
    // And one for the tab strip itself, whose single spare cell is the seventh tab's icon. Same
    // allocation and the same leak, once for the process; it is filled by `adopt_strip_namer` on
    // every pause-menu open, because the strip's namer is rebuilt every time too.
    let strip_shadow = Box::leak(Box::new(ShadowNamer(
        [0; ds2_rva::FE_SCENE_NAMER_SHADOW_SIZE],
    )));
    STRIP_SHADOW.store(strip_shadow as *mut ShadowNamer as usize, Ordering::Release);
    // Published after the storage it describes, and before the detours that read it.
    ADDED_ROWS.store(rows.len(), Ordering::Release);

    let lookups = unsafe {
        hook_site(
            base,
            ds2_rva::FE_INGAME_MENU_TAB_ITEM_LOOKUP,
            &ds2_rva::FE_INGAME_MENU_TAB_ITEM_LOOKUP_PROLOGUE,
            item_lookup_detour as *mut c_void,
            &ITEM_LOOKUP_TRAMPOLINE,
            "item-lookup",
        )
    } && unsafe {
        hook_site(
            base,
            ds2_rva::FE_SCENE_NAMER_CELL_LOOKUP,
            &ds2_rva::FE_SCENE_NAMER_CELL_LOOKUP_PROLOGUE,
            cell_lookup_detour as *mut c_void,
            &CELL_LOOKUP_TRAMPOLINE,
            "cell-lookup",
        )
    } && unsafe {
        hook_site(
            base,
            ds2_rva::FE_SCENE_NAMER_STRIP_CELL_LOOKUP,
            &ds2_rva::FE_SCENE_NAMER_STRIP_CELL_LOOKUP_PROLOGUE,
            strip_cell_lookup_detour as *mut c_void,
            &STRIP_CELL_LOOKUP_TRAMPOLINE,
            "strip-cell-lookup",
        )
    } && unsafe {
        hook_site(
            base,
            ds2_rva::FE_SCENE_NAMER_STRIP_CELL_SECOND,
            &ds2_rva::FE_SCENE_NAMER_STRIP_CELL_LOOKUP_PROLOGUE,
            strip_cell_second_detour as *mut c_void,
            &STRIP_CELL_SECOND_TRAMPOLINE,
            "strip-cell-glyph",
        )
    } && unsafe {
        hook_site(
            base,
            ds2_rva::FE_INGAME_MENU_TAB_INIT,
            &ds2_rva::FE_INGAME_MENU_TAB_INIT_PROLOGUE,
            tab_init_detour as *mut c_void,
            &TAB_INIT_TRAMPOLINE,
            "tab-init",
        )
    };
    if !lookups {
        // EVERY DETOUR ABOVE IS GATED ON THIS, so clearing it makes any site that did get patched a
        // pass-through: `our_item_for` and `tab_init_detour` return immediately at `added == 0`, and
        // `shadow_for` has no namer to match because none has been accepted. Nothing else in the
        // menu has been touched yet -- the item builder, the dispatch, the namer, the layout and the
        // captions are all below this point -- so the pause menu is the shipped one.
        ADDED_ROWS.store(0, Ordering::Release);
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=own-storage rows={} -- the three sites that carry a \
             row past the game's own five item slots and six cell slots are not all patched, so \
             every detour is inert and the pause menu is the shipped one",
            rows.len()
        ));
        return Outcome { installed: false };
    }

    // THE SEVENTH TAB, which is allowed to fail on its own. Its three sites are independent of
    // everything below: if they do not go in, `crate::tab::group()` stays zero, every detour that
    // asks about it declines, and the rows appear on the System tab exactly as they did before this
    // module existed. That is a worse menu, not a broken one, so it is not grounds for refusing the
    // rows as well.
    // SAFETY: `base` is the live module base and MinHook is initialised by the same call that
    // patched the three sites above.
    let seventh_tab = unsafe { crate::tab::install(base) };
    log(format_args!(
        "{LOG_PREFIX} seventh tab {} -- rows go on {}",
        if seventh_tab { "hooked" } else { "NOT hooked" },
        if seventh_tab {
            "a tab of their own"
        } else {
            "the System tab, as before"
        }
    ));

    // THE SEVENTH TAB'S HEXAGON, which is allowed to fail on its own for the same reason the tab is.
    // The six tab icons are one baked quad and the seventh's is a slice of it, so this is a shape
    // the layout has to be able to look up; `crate::strip` asks whether it went in before it writes
    // the record that names it, and before it moves the `RB` prompt out of the way. A refusal is the
    // iconless seventh tab that shipped before this, not a broken strip.
    // SAFETY: `base` is the live module base and MinHook is initialised by the same call that
    // patched the sites above.
    let icon = seventh_tab && unsafe { crate::icon::install(base) };
    log(format_args!(
        "{LOG_PREFIX} seventh tab icon {} -- the tab {}",
        if icon { "hooked" } else { "NOT hooked" },
        if icon {
            "wears the sixth tab's glyph, sliced out of the strip's own plate"
        } else {
            "keeps its highlight and no hexagon, as before"
        }
    ));

    let rva = ds2_rva::FE_INGAME_TOP_SELECT_SYSTEM_TAB_ITEMS;
    let site = base + rva as usize;

    // THE BYTES BEFORE THE PATCH, because an RVA is just a number. On a build this table was not
    // read from, `site` points into the middle of something else and MinHook would happily detour
    // it. Refusing here costs one comparison and is the difference between "the mod did nothing"
    // and "the mod corrupted an unrelated function".
    let expected = ds2_rva::FE_INGAME_TOP_SELECT_SYSTEM_TAB_ITEMS_PROLOGUE;
    // SAFETY: `site` is inside the loaded image's `.text` -- it is a `.pdata` function start
    // recorded in `ds2-rva`, resolved against the live base -- so `expected.len()` bytes are
    // readable there.
    let found = unsafe { std::slice::from_raw_parts(site as *const u8, expected.len()) };
    if found != expected.as_slice() {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=prologue va=0x{site:016x} expected={expected:02x?} \
             found={found:02x?}"
        ));
        return Outcome { installed: false };
    }

    let hook = match unsafe { MhHook::new(site as *mut c_void, detour as *mut c_void) } {
        Ok(hook) => hook,
        Err(status) => {
            log(format_args!(
                "{LOG_PREFIX} install-failed stage=MH_CreateHook va=0x{site:016x} status={status:?}"
            ));
            return Outcome { installed: false };
        }
    };
    // Published BEFORE the site is patched, so a detour cannot observe a zero and skip the
    // original -- which here would mean handing the game a tab with no items at all.
    TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
    let status = unsafe { MH_EnableHook(site as *mut c_void) };
    if status != MH_STATUS::MH_OK {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=MH_EnableHook va=0x{site:016x} status={status:?}"
        ));
        return Outcome { installed: false };
    }
    // The handle falls out of scope here. `MhHook` has no `Drop`, so that does NOT remove the hook
    // -- the patch stays for the life of the process, which is what is wanted.

    log(format_args!(
        "{LOG_PREFIX} hooked rva=0x{rva:08x} va=0x{site:016x} rows={} gate={GATE} \
         open the pause menu's last tab to read the result",
        crate::api::rows_for(crate::api::Tab::Quit).len()
    ));

    // THE DISPATCH DETOUR, which is what makes the appended row DO something. Installed before the
    // probe because it is the one that matters: without it the row is inert.
    let dispatch_rva = ds2_rva::FE_INGAME_MENU_DISPATCH;
    let dispatch_site = base + dispatch_rva as usize;
    match unsafe { MhHook::new(dispatch_site as *mut c_void, dispatch_detour as *mut c_void) } {
        Ok(hook) => {
            DISPATCH_TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
            let status = unsafe { MH_EnableHook(dispatch_site as *mut c_void) };
            if status == MH_STATUS::MH_OK {
                log(format_args!(
                    "{LOG_PREFIX} dispatch hooked rva=0x{dispatch_rva:08x} \
                     va=0x{dispatch_site:016x} actions={:#x}..{:#x}",
                    ds2_rva::FE_INGAME_MENU_ACTION_BASE,
                    ds2_rva::FE_INGAME_MENU_ACTION_BASE
                        + crate::api::rows_for(crate::api::Tab::Quit).len() as u32
                ));
            } else {
                log(format_args!(
                    "{LOG_PREFIX} dispatch NOT installed stage=MH_EnableHook status={status:?} \
                     -- the appended row will be INERT"
                ));
            }
        }
        Err(status) => log(format_args!(
            "{LOG_PREFIX} dispatch NOT installed stage=MH_CreateHook status={status:?} \
             -- the appended row will be INERT"
        )),
    }

    // THE ROW'S CELL, in two halves that only work together. The container substitution adds the
    // layout record; the namer entry is what makes the grid ask for it. Installed in that order so
    // that if the first refuses, the log says so before the second claims a cell that is not there.
    let cell = unsafe { crate::layout::install(base) };
    let _captions = unsafe { crate::caption::install(base) };

    let namer_rva = ds2_rva::FE_INGAME_MENU_QUIT_TAB_NAMER;
    let namer_site = base + namer_rva as usize;
    match unsafe { MhHook::new(namer_site as *mut c_void, namer_detour as *mut c_void) } {
        Ok(hook) => {
            NAMER_TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
            let status = unsafe { MH_EnableHook(namer_site as *mut c_void) };
            if status == MH_STATUS::MH_OK {
                log(format_args!(
                    "{LOG_PREFIX} namer hooked rva=0x{namer_rva:08x} va=0x{namer_site:016x} \
                     cells=[{}] container-substitution={cell}",
                    crate::api::rows_for(crate::api::Tab::Quit)
                        .iter()
                        .map(|row| format!("{:#x}", row.row_id))
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
            } else {
                log(format_args!(
                    "{LOG_PREFIX} namer NOT installed stage=MH_EnableHook status={status:?} \
                     -- the appended row will have no cell and stay invisible"
                ));
            }
        }
        Err(status) => log(format_args!(
            "{LOG_PREFIX} namer NOT installed stage=MH_CreateHook status={status:?} \
             -- the appended row will have no cell and stay invisible"
        )),
    }

    // THE PROBE IS NOT INSTALLED, AND LEAVING IT INSTALLED IS WHAT FROZE THE PAUSE MENU.
    //
    // It reports six tabs and every cell of each: twenty-six lines per open, each one followed by a
    // `sync_all` in the loader's log sink, plus twenty-odd calls into the grid's own cell accessors
    // just to produce them. That ran on every single pause-menu open, long after the question it
    // was written to answer -- `row-extent=4` -- had been settled.
    //
    // The tree dump in `tree.rs` was disarmed for exactly this reason and the freeze SURVIVED,
    // because this one was still going. Two instruments, one mistake, and the second run was wasted
    // by fixing one without looking for the other. An instrument left in the shipping path is a
    // feature nobody asked for.
    //
    // **The per-tab init IS hooked now** -- the count raise above needs it -- so re-arming means
    // calling these two from `tab_init_detour`, not restoring a `MhHook::new`. The detour logs two
    // numbers per install instead of twenty-six per open, which is the same question answered at a
    // cost that can stay.
    let _ = (
        report_tab as *const (),
        report_cell_ids as *const (),
        ds2_rva::FEX_GRID_COL_EXTENT_OFFSET,
    );

    Outcome { installed: true }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every action a slot can be handed must be outside the game's own id space -- the
    /// dispatch's cases are `0..=9`, `0xb`, `0xc`, `0xd`. An id inside that range would make the
    /// row do whatever the game does for it on any run where the detour failed to install, and it
    /// must never be the quit action: a second Return-to-Title row would be a perfectly visible
    /// row that quietly doubles the one item in this menu that discards progress.
    #[test]
    fn every_action_is_ours_and_outside_the_games_range() {
        for slot in 0..crate::api::MAX_ADDED_ROWS {
            let action = ds2_rva::FE_INGAME_MENU_ACTION_BASE + slot as u32;
            assert!(action > 0xd, "{action:#x} is inside the shipped switch");
            assert_ne!(action, ds2_rva::FE_INGAME_MENU_ACTION_RETURN_TITLE);
        }
        assert_eq!(GATE, ds2_rva::FE_INGAME_MENU_GATE_ALWAYS);
    }

    /// The shipped tab must have room in the game's own vector, or the split below never has a
    /// first half and the append is dead code on every run.
    #[test]
    fn the_tab_has_room_for_one_more() {
        assert!(
            ds2_rva::FE_INGAME_MENU_SYSTEM_TAB_ITEMS.len()
                < ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY
        );
    }

    /// Our item entries are the layout the game reads an entry as: two `u32`s per slot, one stride
    /// apart. A flat array is that layout by construction, and this is what says so.
    #[test]
    fn our_entries_are_the_layout_the_game_reads() {
        assert_eq!(ITEM_ENTRIES.len(), crate::api::MAX_ADDED_ROWS * 2);
        let at = |index: usize| &ITEM_ENTRIES[index] as *const AtomicU32 as usize;
        assert_eq!(at(1) - at(0), 4, "the gate is four bytes past the action");
        assert_eq!(
            at(2) - at(0),
            ds2_rva::FE_INGAME_MENU_ITEM_STRIDE,
            "slot 1 must be one entry past slot 0"
        );
    }

    /// There is one stand-in namer per row the registry can accept, or a row registered at the
    /// ceiling would name a cell nothing can answer for.
    #[test]
    fn there_is_a_stand_in_for_every_slot() {
        assert_eq!(SHADOW_NAMERS.len(), crate::api::MAX_ADDED_ROWS);
    }

    /// **A stand-in's one entry must stop short of the count field**, which is the entire reason
    /// there is one stand-in per slot instead of a single list of twelve: the lookup's own stride
    /// puts entry 6 across `+0x140`, and that collision is what caps the GAME's list at six.
    #[test]
    fn a_stand_in_entry_stops_short_of_its_own_count() {
        let one_entry = ds2_rva::FE_SCENE_NAMER_LIST_OFFSET + ds2_rva::FE_SCENE_NAMER_ENTRY_STRIDE;
        assert!(one_entry <= ds2_rva::FE_SCENE_NAMER_COUNT_FROM_NAMER);
        const {
            assert!(
                ds2_rva::FE_SCENE_NAMER_SHADOW_SIZE > ds2_rva::FE_SCENE_NAMER_COUNT_FROM_NAMER,
                "the buffer has to cover the count the lookup reads"
            );
        }
        // THE COLLISION BEING AVOIDED, spelled out. A full list of `capacity` entries fits below
        // the count; the one PAST it starts below the count and runs across it, which is why the
        // game's list stops at six and why a shared list of twelve stand-in entries could not work.
        let next = ds2_rva::FE_SCENE_NAMER_LIST_OFFSET
            + ds2_rva::FE_SCENE_NAMER_LIST_CAPACITY * ds2_rva::FE_SCENE_NAMER_ENTRY_STRIDE;
        assert!(
            next <= ds2_rva::FE_SCENE_NAMER_COUNT_FROM_NAMER,
            "the shipped capacity does not even fit below its own count"
        );
        assert!(
            next + ds2_rva::FE_SCENE_NAMER_ENTRY_STRIDE > ds2_rva::FE_SCENE_NAMER_COUNT_FROM_NAMER,
            "one more entry would have fitted, so six is not the bound this thinks it is"
        );
    }

    /// The stand-in has to be aligned well enough that the lookup's own padding term vanishes --
    /// `list + (-(int)list & 7)`. On a misaligned buffer the game would read a shifted path.
    #[test]
    fn a_stand_in_is_aligned_for_the_lookups_padding_term() {
        let shadow = ShadowNamer([0; ds2_rva::FE_SCENE_NAMER_SHADOW_SIZE]);
        let list = &shadow as *const ShadowNamer as usize + ds2_rva::FE_SCENE_NAMER_LIST_OFFSET;
        assert_eq!(
            (0usize.wrapping_sub(list)) & 7,
            0,
            "the lookup would shift the entry by that many bytes"
        );
    }

    /// The count a full tab wants must be one the grid's bind loop will actually go looking for,
    /// or the cursor would reach a row that has no cell.
    #[test]
    fn a_full_tabs_item_count_is_within_the_binds_reach() {
        let want = ds2_rva::FE_INGAME_MENU_SYSTEM_TAB_ITEMS.len() + crate::api::MAX_ADDED_ROWS;
        assert!(want <= ds2_rva::FEX_GRID_MAX_ROWS);
    }

    /// The expected contents have to include the quit item, or this is not the tab this crate says
    /// it is.
    #[test]
    fn the_expected_tab_carries_the_quit_item() {
        assert!(
            ds2_rva::FE_INGAME_MENU_SYSTEM_TAB_ITEMS
                .iter()
                .any(|(action, _)| *action == ds2_rva::FE_INGAME_MENU_ACTION_RETURN_TITLE)
        );
    }

    /// The padding term is the game's, so it is worth pinning: on any 4-aligned descriptor it must
    /// vanish, and entries must be [`ds2_rva::FE_INGAME_MENU_ITEM_STRIDE`] apart.
    #[test]
    fn entries_are_stride_apart_on_an_aligned_descriptor() {
        let mut buffer = [0u8; 64];
        let base = buffer.as_mut_ptr();
        assert_eq!(base as usize % 4, 0, "test buffer is not 4-aligned");
        // SAFETY: indices 0 and 1 are within a 64-byte buffer at stride 8.
        let (first, second) = unsafe { (entry_at(base, 0), entry_at(base, 1)) };
        assert_eq!(first, base);
        assert_eq!(
            second as usize - first as usize,
            ds2_rva::FE_INGAME_MENU_ITEM_STRIDE
        );
    }

    /// The log's entry rendering is read by a human comparing it against this repo's docs, so its
    /// shape is part of the instrument.
    #[test]
    fn entries_render_as_hex_action_and_decimal_gate() {
        assert_eq!(
            describe(&ds2_rva::FE_INGAME_MENU_SYSTEM_TAB_ITEMS),
            "(0x7,0) (0x8,0) (0x9,4)"
        );
    }
}
