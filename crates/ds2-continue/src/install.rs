//! The one detour, and the pointer walk that turns it into a record of which slot was loaded.

use std::ffi::c_void;
use std::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;

/// A log sink, installed by the loader so this crate writes into the same file as everything else.
/// Stored as a `usize` because a `fn` pointer is not an `Atomic` type.
static LOGGER: AtomicUsize = AtomicUsize::new(0);

/// Signature of the sink. Matches the loader's own logging entry point.
pub type LogFn = fn(std::fmt::Arguments<'_>);

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

/// `FeSubStateTitleLoadDataList::v3(this)`.
///
/// **One argument, established from the body rather than assumed.** The prologue is
/// `rex push rsi; sub rsp,0x20; mov edx,[rcx+0x10]`, and every other value the function uses it
/// loads from a global or from `this`. There is no second incoming argument for a detour to drop.
type UpdateFn = unsafe extern "system" fn(*mut u8);

/// `FeSubStateTitleLoadDataList::v1(this)`. One argument, on the same evidence as the update: the
/// body reads `this` and two globals and touches no other incoming register.
type EnterFn = unsafe extern "system" fn(*mut u8);

/// `FeSubStateTitleTopMenu::v3(this)`. One argument, on the same evidence as the other two.
type TopMenuUpdateFn = unsafe extern "system" fn(*mut u8);

/// `void open(scene)` -- `FeSceneTitle` in RCX, the single argument its own body establishes and
/// the one `ds2-dialog-skip` already calls it with.
type TitleOpenFn = unsafe extern "system" fn(*mut u8);

static UPDATE_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static ENTER_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static TOP_MENU_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static TITLE_OPEN_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// Set once the top menu has been sent to the character list, ever. Once per process, for the
/// same reason [`FIRED`] is: if the list bounces straight back -- no save, a refused character --
/// a re-arming shortcut would bounce between the two screens forever.
static TOP_MENU_FIRED: AtomicU32 = AtomicU32::new(0);

/// The slot to select and load, or negative for "leave the game alone".
static PRESELECT_SLOT: AtomicI32 = AtomicI32::new(-1);

/// Set once the slot has been written and the list is expected to be up; cleared when the load
/// action has been injected.
static ARMED: AtomicU32 = AtomicU32::new(0);

/// Set once the action has been injected, ever. **Once per process, deliberately.** If the load
/// bounces back to the list -- a refused character, a broken save -- an autoload that re-armed
/// would take the same edge again forever, and a boot loop is a far worse failure than a menu.
static FIRED: AtomicU32 = AtomicU32::new(0);

/// Ask for a slot to be selected when the character list opens. Call before [`install`].
///
/// A negative value, or one the game's own bound would reject, disables the write entirely -- this
/// never points the cursor somewhere the game would not.
pub fn set_preselect_slot(slot: i32) {
    PRESELECT_SLOT.store(slot, Ordering::Release);
}

/// The live module base, resolved once in [`install`] so the detour never has to.
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);

/// The last sample logged, packed, so a per-frame hook prints on change instead of every frame.
static LAST: AtomicU64 = AtomicU64::new(u64::MAX);

/// Whether the one-shot line proving the pointer walk resolves has been written.
static DESCRIBED: AtomicU32 = AtomicU32::new(0);

/// Everything the update reads or decides, as of one call.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Sample {
    /// The substate's phase at `+0x10`. The update is inert unless this is 1 on entry.
    phase: i32,
    /// `[FE_TITLE_CONTEXT] + 0x564`, the selected slot. Negative or `>= 10` means none.
    slot: i32,
    /// The group's confirmed action at `group + 0x28`.
    action: i32,
    /// The slot record's flags byte, and whether the record resolved at all.
    flags: Option<u8>,
    /// `record[0x1e8] & 0x3f`, the word the ownership gate is asked about.
    ownership: Option<u32>,
}

impl Sample {
    /// Pack into a key for change detection. Lossless for every field that fits, and collisions
    /// only cost a skipped duplicate line, never a wrong one.
    fn key(&self) -> u64 {
        let flags = u64::from(self.flags.unwrap_or(0xff));
        let ownership = u64::from(self.ownership.unwrap_or(0xffff_ffff));
        (u64::from(self.phase as u8) << 56)
            | (u64::from(self.slot as u8) << 48)
            | (u64::from(self.action as u8) << 40)
            | (flags << 32)
            | ownership
    }
}

/// Follow a global pointer at `base + rva`, returning `None` for a null.
///
/// # Safety
///
/// `rva` must name a pointer-sized global in the loaded image, and the module must be mapped.
unsafe fn deref_global(base: usize, rva: u32) -> Option<*mut u8> {
    let slot = (base + rva as usize) as *const *mut u8;
    // SAFETY: the caller guarantees the RVA names a pointer-sized global in a mapped image; this
    // is the same load the game makes at 0x1400fba24 and 0x1400fba53.
    let value = unsafe { slot.read() };
    (!value.is_null()).then_some(value)
}

/// Read the field the game reads, from the object it reads it on.
///
/// # Safety
///
/// `object` must be non-null and live, and `offset` must name a field of it.
unsafe fn field<T: Copy>(object: *mut u8, offset: usize) -> T {
    // SAFETY: the caller guarantees a live object and a real field offset.
    unsafe { object.add(offset).cast::<T>().read() }
}

/// Take a sample by walking exactly the pointers the update walks.
///
/// Returns [`Default`] fields wherever a pointer is null, which is the honest answer before the
/// title context exists rather than a reason to skip logging.
///
/// # Safety
///
/// `this` must be the live `FeSubStateTitleLoadDataList` the game is calling.
unsafe fn sample(this: *mut u8) -> Sample {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let mut out = Sample {
        // SAFETY: the game's own first instruction reads this field on this pointer.
        phase: unsafe { field::<i32>(this, ds2_rva::FE_SUBSTATE_PHASE_OFFSET) },
        ..Sample::default()
    };
    if base == 0 {
        return out;
    }
    // SAFETY: `FE_TITLE_CONTEXT` is the pointer-sized global the update loads at 0x1400fba24.
    let Some(context) = (unsafe { deref_global(base, ds2_rva::FE_TITLE_CONTEXT) }) else {
        return out;
    };
    // SAFETY: `context` is the live title context; both offsets are ones the update reads on it.
    out.slot = unsafe { field::<i32>(context, ds2_rva::FE_TITLE_CONTEXT_SLOT_NUM_OFFSET) };
    let group =
        unsafe { field::<*mut u8>(context, ds2_rva::FE_TITLE_CONTEXT_DATA_LIST_GROUP_OFFSET) };
    if !group.is_null() {
        // SAFETY: `group` is the list group the update just called a virtual on.
        out.action = unsafe { field::<i32>(group, ds2_rva::FE_GROUP_DATA_LIST_ACTION_OFFSET) };
    }

    // The record, under exactly the bound the game applies before its own `imul`.
    if out.slot < 0 || out.slot >= ds2_rva::SAVE_SLOT_COUNT {
        return out;
    }
    // SAFETY: `GAME_MANAGER_IMP` is the pointer-sized global the update loads at 0x1400fba53.
    let Some(manager) = (unsafe { deref_global(base, ds2_rva::GAME_MANAGER_IMP) }) else {
        return out;
    };
    // SAFETY: the update walks these two offsets in sequence off that same global.
    let data_manager = unsafe { field::<*mut u8>(manager, ds2_rva::GAME_DATA_MANAGER_OFFSET) };
    if data_manager.is_null() {
        return out;
    }
    let array = unsafe { field::<*mut u8>(data_manager, ds2_rva::SAVE_SLOT_ARRAY_OFFSET) };
    if array.is_null() {
        return out;
    }
    // SAFETY: slot is bounds-checked above against the same limit the game uses, and the stride is
    // the one its own `imul` applies.
    let record = unsafe { array.add(out.slot as usize * ds2_rva::SAVE_SLOT_STRIDE) };
    // SAFETY: `record` is one element of the array the game indexes identically.
    out.flags = Some(unsafe { field::<u8>(record, ds2_rva::SAVE_SLOT_FLAGS_OFFSET) });
    out.ownership = Some(
        unsafe { field::<u32>(record, ds2_rva::SAVE_SLOT_OWNERSHIP_OFFSET) }
            & ds2_rva::SAVE_SLOT_OWNERSHIP_MASK,
    );
    out
}

/// The one-shot line that proves the pointer walk resolves at runtime.
///
/// The offsets in [`ds2_rva`] were read out of one function's disassembly. This prints the
/// addresses they produce on a live process, so a run that recorded nothing can be told apart from
/// a run whose walk went somewhere wrong -- which a log of plausible-looking zeroes could not.
fn describe_once(sample: &Sample, base: usize) {
    if DESCRIBED.swap(1, Ordering::Relaxed) != 0 {
        return;
    }
    log(format_args!(
        "{LOG_PREFIX} walk base=0x{base:016x} phase={} slot={} action={} record={}",
        sample.phase,
        sample.slot,
        sample.action,
        match sample.flags {
            Some(flags) => format!("resolved flags=0x{flags:02x}"),
            None => "unresolved".to_string(),
        }
    ));
}

/// Sample around the original, and report when anything the player can influence has moved.
unsafe extern "system" fn detour_update(this: *mut u8) {
    if this.is_null() {
        return;
    }
    // SAFETY: the flow is calling a virtual on this object, so it is live.
    let before = unsafe { sample(this) };
    describe_once(&before, MODULE_BASE.load(Ordering::Acquire));

    let trampoline = UPDATE_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for this site, and the signature is the single
        // argument the function's own body establishes.
        let original: UpdateFn = unsafe { std::mem::transmute::<usize, UpdateFn>(trampoline) };
        unsafe { original(this) };
    }

    // SAFETY: the original returned, so the object is still live.
    let after = unsafe { sample(this) };

    // TAKE THE GAME'S OWN LOAD BRANCH, after the original has declined to.
    //
    // Injecting the action BEFORE the original was tried and measured: the group's poll rewrites
    // that field every frame, so the write was gone before the switch read it (`action=0->0`).
    // The slot field goes the same way -- the poll republishes it from the cursor on any input
    // (`slot=0->1`). Both are outputs of the group, not inputs to it, so there is nothing to set.
    //
    // What is left is to do exactly what the branch does, in its order: the `0x1400f10e0(group)`
    // call all four branches make, then the ownership gate, then the phase. The slot is written
    // last, immediately before the transition, because the update has already read it and the
    // substate this hands to reads it fresh.
    if after.phase == 1
        && ARMED.swap(0, Ordering::AcqRel) == 1
        && FIRED.swap(1, Ordering::AcqRel) == 0
    {
        // SAFETY: the original just returned on this object, so it and the globals are live.
        unsafe { take_load_branch(this) };
    }

    // THE TWO WAYS THE LIST ENDS WITHOUT LOADING. Suppression is released at `StartIngame`, which
    // a backed-out or refused list never reaches -- so without this the game would keep playing
    // silently long after the shortcut it was covering had stopped.
    if after.phase == ds2_rva::FE_DATA_LIST_PHASE_REFUSED {
        crate::silence::release("data-list-refused");
    } else if after.phase == ds2_rva::FE_DATA_LIST_PHASE_BACK {
        crate::silence::release("data-list-backed-out");
    }

    // NOTHING THE POLL TOUCHES IS VALID BEFORE, and that is wider than the first fix assumed. The original opens with
    // `group->vtable[4](group)`, which is what SETS the action -- so a sample taken before it
    // carries the previous frame's value. The first recorded load printed `action=0` beside a
    // phase of 1->2, a transition only action 2 can produce. Both samples are printed now rather
    // than picking one, because which of them the game leaves behind after the branch is a
    // question about code this crate has not read, and a second guess would be no better than the
    // first.
    //
    // THE SLOT HAD THE SAME DEFECT and the first fix missed it, which is worse than the defect.
    // The update reads the slot AFTER the poll too, so a `before` slot is the previous frame's
    // cursor. Reporting only that made one run look like the game had loaded a character the
    // screen was not highlighting -- an alarming conclusion produced entirely by this file's own
    // stale field. Slot, flags and ownership all come from the sample the game decided on now.
    let moved = after.phase != before.phase || after.key() != LAST.load(Ordering::Relaxed);
    if !moved {
        return;
    }
    LAST.store(after.key(), Ordering::Relaxed);
    log(format_args!(
        "{LOG_PREFIX} data-list phase={}->{} slot={}->{} action={}->{} occupied={} excluded={} \
         own={} dest={}",
        before.phase,
        after.phase,
        before.slot,
        after.slot,
        before.action,
        after.action,
        after.flags.map_or("?".to_string(), |f| ((f
            & ds2_rva::SAVE_SLOT_FLAG_OCCUPIED)
            != 0)
            .to_string()),
        after.flags.map_or("?".to_string(), |f| ((f
            & ds2_rva::SAVE_SLOT_FLAG_EXCLUDED)
            != 0)
            .to_string()),
        after
            .ownership
            .map_or("?".to_string(), |o| format!("0x{o:02x}")),
        destination(after.phase),
    ));
}

/// Pose the title screen hidden the moment anything opens it.
///
/// # Why the open and not the top menu's update
///
/// MEASURED, from the log's own ordering. `title_settle` opens the screen during substate `0x17`
/// -- `ds2-dialog-skip` logs `settled screen=title-main` there, and that call is the screen's whole
/// open, not a sequence play. A pose driven from `FeSubStateTitleTopMenu`'s update lands eleven log
/// lines later, after the entire network and dialog chain. The process windows cover the screen for
/// most of that gap, which is why it read on screen as a brief flash rather than seconds of menu:
/// what was visible was the window between the last dialog clearing and substate `0x47` arriving.
///
/// Hooking the open removes the gap by construction. Whatever raises the screen is posed hidden on
/// its way out, so there is no interval in which it is up and un-posed.
///
/// The pose uses this detour's own `this` rather than re-reading the title context, so it lands on
/// the object that was actually opened.
unsafe extern "system" fn detour_title_open(this: *mut u8) {
    let trampoline = TITLE_OPEN_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for this site; one argument, established from
        // the function's own body and matching the call `ds2-dialog-skip` already makes.
        let original: TitleOpenFn =
            unsafe { std::mem::transmute::<usize, TitleOpenFn>(trampoline) };
        unsafe { original(this) };
    }
    // AFTER the original: it is the open, so the screen has to exist before it can be posed.
    // SAFETY: on the game thread, with the receiver the open was itself called on.
    unsafe { crate::hide_menus::pose_scene_hidden(this, "open") };
}

/// Take the top menu's LOAD GAME edge as soon as the menu is idle, so no button is needed.
///
/// **After the original, never before.** The original polls the group and then copies whatever the
/// group parked into the phase, so a value written before it is overwritten by that copy. Written
/// after, it survives the one frame that matters: `FeStateFlow` evaluates transitions as soon as
/// the update returns.
///
/// This is the row-1 transition the menu already registers, unconditionally, taken with the value
/// the row itself would have produced. It invents no destination -- the character list still opens
/// and still runs its own `enter`, which is what keeps `LoadProfile` supplied.
unsafe extern "system" fn detour_top_menu(this: *mut u8) {
    let trampoline = TOP_MENU_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for this site; one argument, established from
        // the function's own body.
        let original: TopMenuUpdateFn =
            unsafe { std::mem::transmute::<usize, TopMenuUpdateFn>(trampoline) };
        unsafe { original(this) };
    }
    if this.is_null() || PRESELECT_SLOT.load(Ordering::Acquire) < 0 {
        return;
    }
    // AFTER the original, every frame the title screen is resident. The original is what opens it,
    // so there is nothing to hide before it runs, and re-posing an already-posed scene writes the
    // same playback position again -- which is why this is safe to repeat rather than latch.
    //
    // A pose and not a close. The close animates, and this substate does not live long enough for
    // the fade to finish; see `ds2_rva::FE_SCENE_TITLE_POSE_HIDDEN`.
    // SAFETY: on the game thread, with the scene read the way the original reads it.
    unsafe { crate::hide_menus::pose_title_hidden() };
    // SAFETY: the flow just called a virtual on this object, so it is live.
    let phase = unsafe { field::<i32>(this, ds2_rva::FE_SUBSTATE_PHASE_OFFSET) };
    // Only from rest. A non-resting phase means the player activated a row this frame, and their
    // choice outranks the config every time.
    if phase != ds2_rva::FE_TOP_MENU_PHASE_RESTING || TOP_MENU_FIRED.swap(1, Ordering::AcqRel) != 0
    {
        return;
    }
    // SAFETY: the phase field the flow is about to read for its transition search.
    unsafe {
        this.add(ds2_rva::FE_SUBSTATE_PHASE_OFFSET)
            .cast::<i32>()
            .write(ds2_rva::FE_TOP_MENU_ACTION_LOAD_GAME);
    }
    log(format_args!(
        "{LOG_PREFIX} top-menu took action={} dest=0x55-LoadDataList",
        ds2_rva::FE_TOP_MENU_ACTION_LOAD_GAME
    ));
}

/// Do what the update's load branch does, on the configured slot.
///
/// # Safety
///
/// Must run on the game thread immediately after the update returned, with `this` live.
unsafe fn take_load_branch(this: *mut u8) {
    let wanted = PRESELECT_SLOT.load(Ordering::Acquire);
    let base = MODULE_BASE.load(Ordering::Acquire);
    // SAFETY: the two globals the update dereferences on every call.
    let (Some(context), Some(manager)) = (unsafe {
        (
            deref_global(base, ds2_rva::FE_TITLE_CONTEXT),
            deref_global(base, ds2_rva::GAME_MANAGER_IMP),
        )
    }) else {
        log(format_args!(
            "{LOG_PREFIX} autoload abandoned reason=no-globals"
        ));
        crate::silence::release("abandon-no-globals");
        return;
    };
    // SAFETY: `context` is live; the group is the one the update just polled.
    let group =
        unsafe { field::<*mut u8>(context, ds2_rva::FE_TITLE_CONTEXT_DATA_LIST_GROUP_OFFSET) };
    // SAFETY: the update's own walk to the slot record.
    let record = unsafe {
        let data = field::<*mut u8>(manager, ds2_rva::GAME_DATA_MANAGER_OFFSET);
        if data.is_null() || !(0..ds2_rva::SAVE_SLOT_COUNT).contains(&wanted) {
            None
        } else {
            let array = field::<*mut u8>(data, ds2_rva::SAVE_SLOT_ARRAY_OFFSET);
            (!array.is_null()).then(|| array.add(wanted as usize * ds2_rva::SAVE_SLOT_STRIDE))
        }
    };
    let (Some(record), false) = (record, group.is_null()) else {
        log(format_args!(
            "{LOG_PREFIX} autoload abandoned slot={wanted} reason=unresolved"
        ));
        crate::silence::release("abandon-unresolved");
        return;
    };
    // SAFETY: `record` is one element of the array the game indexes identically.
    let flags = unsafe { field::<u8>(record, ds2_rva::SAVE_SLOT_FLAGS_OFFSET) };
    if flags & ds2_rva::SAVE_SLOT_FLAG_OCCUPIED == 0
        || flags & ds2_rva::SAVE_SLOT_FLAG_EXCLUDED != 0
    {
        log(format_args!(
            "{LOG_PREFIX} autoload abandoned slot={wanted} reason=slot-unusable flags=0x{flags:02x}"
        ));
        crate::silence::release("abandon-slot-unusable");
        return;
    }

    // The gate, replicated as reads rather than called. `0x140af6610` is four loads, an OR, an AND
    // and a compare, with no side effect, so reproducing it costs nothing and calls into nothing.
    // SAFETY: every dereference is null-checked, exactly as the game's own function checks.
    let refused = unsafe {
        let required = u64::from(
            field::<u32>(record, ds2_rva::SAVE_SLOT_OWNERSHIP_OFFSET)
                & ds2_rva::SAVE_SLOT_OWNERSHIP_MASK,
        );
        let ctx = field::<*mut u8>(manager, ds2_rva::GAME_MANAGER_CONTENT_CTX_OFFSET);
        let owned = if ctx.is_null() {
            None
        } else {
            let obj = field::<*mut u8>(ctx, ds2_rva::CONTENT_CTX_OWNED_OFFSET);
            (!obj.is_null()).then(|| {
                field::<u64>(obj, ds2_rva::CONTENT_OWNED_MASK_A)
                    | field::<u64>(obj, ds2_rva::CONTENT_OWNED_MASK_B)
            })
        };
        owned.is_some_and(|owned| owned & required != required)
    };

    // SAFETY: one argument, the group, taken from `mov rcx,rdi` at every branch's call site.
    unsafe {
        let close: unsafe extern "system" fn(*mut u8) =
            std::mem::transmute((base + ds2_rva::FE_DATA_LIST_CLOSE as usize) as *const ());
        close(group);
    }
    let phase = if refused {
        ds2_rva::FE_DATA_LIST_PHASE_REFUSED
    } else {
        ds2_rva::FE_DATA_LIST_PHASE_LOAD
    };
    // SAFETY: `context` and `this` are live; these are the two fields the branch writes.
    unsafe {
        context
            .add(ds2_rva::FE_TITLE_CONTEXT_SLOT_NUM_OFFSET)
            .cast::<i32>()
            .write(wanted);
        this.add(ds2_rva::FE_SUBSTATE_PHASE_OFFSET)
            .cast::<i32>()
            .write(phase);
    }
    log(format_args!(
        "{LOG_PREFIX} autoload slot={wanted} refused={refused} phase={phase} dest={}",
        destination(phase)
    ));
}

/// The substate the flow will move to for a phase the update just wrote, from the transition table
/// `FeSubStateTitleLoadDataList::v5` (`0x1400fb1f0`) publishes. `-` for a phase that registers no
/// transition, which includes the resting phase 1.
fn destination(phase: i32) -> &'static str {
    match phase {
        2 => "0x57-LoadProfile",
        3 => "0x47-TopMenu",
        4 => "0x56",
        5 => "0x5f",
        6 => "0x5d",
        7 => "0x17-TitleMain",
        _ => "-",
    }
}

/// Write the configured slot before the list is built, then let the original build it.
///
/// **Before, and that is the whole point.** The list group both reads the slot field when it lays
/// the list out and writes it back on every cursor move, so a write made after `enter` is a write
/// the next cursor move erases. This one lands before `0x1400f1cb0` is even called.
///
/// It refuses to point the cursor anywhere the game would not: the slot is bounds-checked against
/// [`ds2_rva::SAVE_SLOT_COUNT`] exactly as the update checks it, and the record must be occupied
/// and not excluded. A configured slot that fails any of those is logged and skipped, leaving the
/// game's own selection untouched -- an empty list is a recoverable screen, and a cursor parked on
/// a slot the update will refuse is not.
unsafe extern "system" fn detour_enter(this: *mut u8) {
    let wanted = PRESELECT_SLOT.load(Ordering::Acquire);
    if wanted >= 0 {
        // SAFETY: the flow is entering this substate, so the title context and save data the walk
        // reads are the same ones the update reads one frame later.
        unsafe { preselect(wanted) };
    }
    let trampoline = ENTER_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for this site, and the signature is the single
        // argument the function's own body establishes.
        let original: EnterFn = unsafe { std::mem::transmute::<usize, EnterFn>(trampoline) };
        unsafe { original(this) };
    }
    // The list's `enter` is what opens the group -- it ends in `0x1400f1cb0(group, 1)`, which plays
    // the open sequence. Closing here rather than from the update means the list is shut on the
    // same frame it was opened, before it has a frame to be drawn in.
    //
    // Deliberately NOT also done from the update: `take_load_branch` calls the list's own close on
    // this group, and doing less to the object the working autoload depends on is worth a frame of
    // risk that the menu shows.
    if wanted >= 0 {
        // SAFETY: on the game thread, with the group read the way the original reads it.
        unsafe { crate::hide_menus::close_data_list() };
    }
}

/// Point the character list at `wanted`, or say why it did not.
///
/// # Safety
///
/// Must run on the game thread with the title context live, which `enter` guarantees.
unsafe fn preselect(wanted: i32) {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return;
    }
    // SAFETY: the same two globals the update dereferences.
    let Some(context) = (unsafe { deref_global(base, ds2_rva::FE_TITLE_CONTEXT) }) else {
        log(format_args!(
            "{LOG_PREFIX} preselect skipped slot={wanted} reason=no-title-context"
        ));
        return;
    };
    if wanted >= ds2_rva::SAVE_SLOT_COUNT {
        log(format_args!(
            "{LOG_PREFIX} preselect skipped slot={wanted} reason=out-of-range limit={}",
            ds2_rva::SAVE_SLOT_COUNT
        ));
        return;
    }
    // SAFETY: `GAME_MANAGER_IMP` and the two offsets after it are the update's own walk.
    let record = unsafe {
        deref_global(base, ds2_rva::GAME_MANAGER_IMP).and_then(|manager| {
            let data = field::<*mut u8>(manager, ds2_rva::GAME_DATA_MANAGER_OFFSET);
            (!data.is_null()).then(|| field::<*mut u8>(data, ds2_rva::SAVE_SLOT_ARRAY_OFFSET))
        })
    };
    let Some(array) = record.filter(|array| !array.is_null()) else {
        log(format_args!(
            "{LOG_PREFIX} preselect skipped slot={wanted} reason=no-save-data"
        ));
        return;
    };
    // SAFETY: bounds-checked above against the limit the game applies before its own `imul`.
    let flags = unsafe {
        field::<u8>(
            array.add(wanted as usize * ds2_rva::SAVE_SLOT_STRIDE),
            ds2_rva::SAVE_SLOT_FLAGS_OFFSET,
        )
    };
    if flags & ds2_rva::SAVE_SLOT_FLAG_OCCUPIED == 0
        || flags & ds2_rva::SAVE_SLOT_FLAG_EXCLUDED != 0
    {
        log(format_args!(
            "{LOG_PREFIX} preselect skipped slot={wanted} reason=slot-unusable flags=0x{flags:02x}"
        ));
        return;
    }
    // SAFETY: `context` is the live title context and this is the field the list lays out from.
    let previous = unsafe {
        let slot = context
            .add(ds2_rva::FE_TITLE_CONTEXT_SLOT_NUM_OFFSET)
            .cast::<i32>();
        let previous = slot.read();
        slot.write(wanted);
        previous
    };
    ARMED.store(1, Ordering::Release);
    log(format_args!(
        "{LOG_PREFIX} preselect slot={previous}->{wanted} flags=0x{flags:02x} armed=true"
    ));
}

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Sites now detoured.
    pub installed: usize,
    /// Sites attempted.
    pub attempted: usize,
}

/// Detour `FeSubStateTitleLoadDataList::v3`. Call from the post-Arxan callback, never `DllMain`.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Must run after `neuter_arxan` and before
/// the title flow starts, which in practice is the loader's Arxan callback. The site was checked
/// against `scripts/ds2-arxan-chain.py`: it is an ordinary `rex push rsi` prologue, not one of the
/// five-byte `e9` redirects Arxan installs.
pub unsafe fn install() -> Outcome {
    struct Site {
        name: &'static str,
        rva: u32,
        detour: *mut c_void,
        trampoline: &'static AtomicUsize,
    }
    let mut sites: Vec<Site> = vec![
        Site {
            name: "top-menu-update",
            rva: ds2_rva::FE_SUBSTATE_TOP_MENU_UPDATE,
            detour: detour_top_menu as *mut c_void,
            trampoline: &TOP_MENU_TRAMPOLINE,
        },
        // ENTER FIRST. The recorder alone is a complete instrument; the pre-select alone is a
        // write with nothing watching it. If only one of the two can be installed, the one that
        // observes is worth more than the one that acts.
        Site {
            name: "data-list-enter",
            rva: ds2_rva::FE_SUBSTATE_LOAD_DATA_LIST_ENTER,
            detour: detour_enter as *mut c_void,
            trampoline: &ENTER_TRAMPOLINE,
        },
        Site {
            name: "data-list-update",
            rva: ds2_rva::FE_SUBSTATE_LOAD_DATA_LIST_UPDATE,
            detour: detour_update as *mut c_void,
            trampoline: &UPDATE_TRAMPOLINE,
        },
    ];
    // Only patched when `[continue] silence` asked for it. Two of the three sites above are needed
    // to record anything at all; these two exist purely to keep the shortcut quiet, so a run that
    // does not want that should not carry them.
    // EITHER feature, not just silencing: the `start-ingame-enter` site in that list is what ends
    // both, so a run with the cover on and the mute off still needs it. The two sound detours are
    // inert when silencing is off -- they check the flag before touching anything.
    // Only patched when `[continue] hide_menus` asked for it, for the same reason the sound sites
    // are conditional: a run that does not want the feature should not carry its patched site.
    //
    // FIRST in the list it joins, because it is the one that decides whether anything is ever seen.
    // The others act once the flow has already reached a screen; this one acts as the screen goes
    // up, which is what closes the gap the last run measured.
    if crate::hide_menus::enabled() {
        sites.insert(
            0,
            Site {
                name: "title-screen-open",
                rva: ds2_rva::FE_SCENE_TITLE_OPEN,
                detour: detour_title_open as *mut c_void,
                trampoline: &TITLE_OPEN_TRAMPOLINE,
            },
        );
    }
    if crate::silence::enabled() || crate::hide_menus::enabled() {
        sites.extend(
            crate::silence::sites()
                .into_iter()
                .map(|(name, rva, detour, trampoline)| Site {
                    name,
                    rva,
                    detour,
                    trampoline,
                }),
        );
    }
    let attempted = sites.len();

    let base = match ds2_game_base::mem::game_module_base() {
        Ok(base) => base,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} install-failed stage=module-base error={error}"
            ));
            return Outcome {
                installed: 0,
                attempted,
            };
        }
    };
    MODULE_BASE.store(base, Ordering::Release);
    crate::silence::set_module_base(base);
    crate::hide_menus::set_module_base(base);

    // MinHook is statically linked into this DLL, so ALREADY_INITIALIZED can only mean this ran
    // twice. Treat it as success, exactly as the other feature crates do.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=MH_Initialize status={status:?}"
        ));
        return Outcome {
            installed: 0,
            attempted,
        };
    }

    let mut installed = 0;
    for site in &sites {
        let address = base + site.rva as usize;
        let hook = match unsafe { MhHook::new(address as *mut c_void, site.detour) } {
            Ok(hook) => hook,
            Err(status) => {
                log(format_args!(
                    "{LOG_PREFIX} hook-failed site={} va=0x{address:016x} stage=MH_CreateHook \
                     status={status:?}",
                    site.name
                ));
                continue;
            }
        };
        // Published BEFORE the site is patched. Both run on the game thread while the character
        // list is up, so a detour that read a zero here would skip the original and hang the list.
        site.trampoline
            .store(hook.trampoline() as usize, Ordering::Release);
        let status = unsafe { MH_EnableHook(address as *mut c_void) };
        if status != MH_STATUS::MH_OK {
            log(format_args!(
                "{LOG_PREFIX} hook-failed site={} va=0x{address:016x} stage=MH_EnableHook \
                 status={status:?}",
                site.name
            ));
            continue;
        }
        installed += 1;
        log(format_args!(
            "{LOG_PREFIX} hooked site={} rva=0x{:08x} va=0x{address:016x}",
            site.name, site.rva
        ));
    }
    // MUTING IS NOT ARMED HERE. It arms inside the detour on audio init, because that is the
    // first moment a master channel group exists to mute. What matters at this point is the
    // opposite check: if any site failed, the mute could end up applied with no detour able to
    // give the volume back, so say so loudly rather than let a half-installed run go quiet.
    if (crate::silence::enabled() || crate::hide_menus::enabled()) && installed != attempted {
        log(format_args!(
            "{LOG_PREFIX} silence sites-incomplete installed={installed} of {attempted} --              audio may not be restored and the cover may not be dropped"
        ));
    }
    log(format_args!(
        "{LOG_PREFIX} install installed={installed} of {attempted} preselect={} silence={} \
         hide-menus={}",
        PRESELECT_SLOT.load(Ordering::Acquire),
        crate::silence::enabled(),
        crate::hide_menus::enabled()
    ));
    Outcome {
        installed,
        attempted,
    }
}
