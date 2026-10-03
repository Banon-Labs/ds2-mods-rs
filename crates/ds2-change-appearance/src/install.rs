//! The nine detours, the one-instruction patch and the tick.
//!
//! Every detour goes through the hook union, so a second feature on the same function chains
//! rather than replacing ours. The union's dispatcher adds a frame, which is why the row is found
//! by its label and an enclosing flag rather than by a return address.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use ds2_game_base::mem::{game_module_base, read_bytes, safe_read_u8, safe_read_usize};
use ds2_hook::{UnionFn, patch_3byte_stub, register_union_hook};
use ds2_rva::{APPEARANCE_BLOCK_LEN, VISIBLE_EQUIP_RECORD_LEN, VISIBLE_EQUIP_SLOTS};

use crate::tabs::{Layout, layout_of, writes};
use crate::{LOG_PREFIX, REPORT_AFTER_TICKS, ROW_LABEL_UTF16, open_now, records_differing};

/// A log sink, installed by the loader. Stored as a `usize` because a `fn` pointer is not an
/// `Atomic` type.
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

type Record = [u8; VISIBLE_EQUIP_RECORD_LEN];
type Block = [u8; APPEARANCE_BLOCK_LEN];

/// What the creator's enter is about to strip.
struct Snapshot {
    equip: [Record; VISIBLE_EQUIP_SLOTS],
}

static BASE: AtomicUsize = AtomicUsize::new(0);

/// A change is in progress: from the open until the class/gift commit is skipped.
static ARMED: AtomicBool = AtomicBool::new(false);
/// The row was chosen and the creator has not been opened yet.
static OPEN_REQUESTED: AtomicBool = AtomicBool::new(false);
/// The bonfire menu builder is on the stack.
static IN_BONFIRE_BUILD: AtomicBool = AtomicBool::new(false);
/// The label pointer of the Item box row, looked up when the builder starts.
static ITEM_BOX_LABEL: AtomicUsize = AtomicUsize::new(0);
/// The face commit is on the stack and armed.
static IN_FACE_COMMIT: AtomicBool = AtomicBool::new(false);
/// The creator's warehouse, from its enter.
static WAREHOUSE: AtomicUsize = AtomicUsize::new(0);
/// Ticks left until the result is logged; 0 when nothing is pending.
static REPORT_IN: AtomicU32 = AtomicU32::new(0);
static ROW_ADDED_SAID: AtomicBool = AtomicBool::new(false);

static SNAPSHOT: Mutex<Option<Snapshot>> = Mutex::new(None);
static COMMITTED: Mutex<Option<Block>> = Mutex::new(None);

static ORIG_BUILD: AtomicUsize = AtomicUsize::new(0);
static ORIG_ADD_ROW: AtomicUsize = AtomicUsize::new(0);
static ORIG_ENTER: AtomicUsize = AtomicUsize::new(0);
static ORIG_FACE_COMMIT: AtomicUsize = AtomicUsize::new(0);
static ORIG_CLASS_GIFT: AtomicUsize = AtomicUsize::new(0);
static ORIG_SET_FACE: AtomicUsize = AtomicUsize::new(0);
static ORIG_TOP_TABS: AtomicUsize = AtomicUsize::new(0);
static ORIG_TAB_APPEND: AtomicUsize = AtomicUsize::new(0);
static ORIG_FLO_ADOPT: AtomicUsize = AtomicUsize::new(0);

/// The creator's tab bar is ours: from the open until the report, which is after the creator has
/// closed. Longer than [`ARMED`], which the commit clears while the creator is still drawn.
static TABS_HIDDEN: AtomicBool = AtomicBool::new(false);
/// The creator's tab list builder is on the stack with [`TABS_HIDDEN`] set.
static IN_TOP_TABS: AtomicBool = AtomicBool::new(false);
/// This build already left out its first spec.
static CLASS_TAB_SKIPPED: AtomicBool = AtomicBool::new(false);
static TABS_SAID: AtomicBool = AtomicBool::new(false);
/// The creator's parsed layout, from its adopt. The bundle stays cached, so this is set once.
static LAYOUT: AtomicUsize = AtomicUsize::new(0);

/// The row's job creator: a `DLReferenceCountObject` the menu holds by pointer. Its vtable is the
/// game's own first two slots and [`invoke`]; the reference count is set far from zero so neither
/// destructor is ever reached. Both live in `static`s because the game keeps raw pointers to them.
static VTABLE: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];
static CREATOR: [AtomicUsize; 5] = [const { AtomicUsize::new(0) }; 5];
const CREATOR_REFCOUNT: usize = 0x4000_0000;

fn rva(rva: u32) -> usize {
    BASE.load(Ordering::Acquire) + rva as usize
}

/// Call the next handler in the union chain, or the game.
///
/// # Safety
///
/// `slot` is one of this crate's `ORIG_*` cells, filled by `register_union_hook`.
unsafe fn call_orig(slot: &AtomicUsize, a: usize, b: usize, c: usize, d: usize) -> usize {
    // SAFETY: the union stores a `UnionFn` (the next handler or the game's trampoline) in the slot
    // before the target is enabled.
    let f: UnionFn = unsafe { std::mem::transmute::<usize, UnionFn>(slot.load(Ordering::Acquire)) };
    // SAFETY: the caller's own four arguments, passed straight through.
    unsafe { f(a, b, c, d) }
}

/// `obj->vtbl[offset](obj)`, fault-tolerant on the two reads.
///
/// # Safety
///
/// `obj` is a live game object whose vtable slot at `offset` takes only `this` and returns a
/// pointer.
unsafe fn vcall0(obj: usize, offset: usize) -> Option<usize> {
    // SAFETY: fault-tolerant reads.
    let vtbl = unsafe { safe_read_usize(obj) }?;
    // SAFETY: as above.
    let f = unsafe { safe_read_usize(vtbl + offset) }?;
    if f == 0 {
        return None;
    }
    // SAFETY: the slot's signature is the caller's contract.
    let f: extern "system" fn(usize) -> usize = unsafe { std::mem::transmute::<usize, _>(f) };
    Some(f(obj)).filter(|&p| p != 0)
}

fn game_manager() -> Option<usize> {
    // SAFETY: fault-tolerant read of the `GameManagerImp` global.
    unsafe { safe_read_usize(rva(ds2_rva::GAME_MANAGER_IMP)) }.filter(|&p| p != 0)
}

fn player() -> Option<usize> {
    // SAFETY: fault-tolerant read.
    unsafe { safe_read_usize(game_manager()? + ds2_rva::PLAYER_CTRL_OFFSET) }.filter(|&p| p != 0)
}

/// The player's `ChrAsm` and its model and visible equipment.
fn chr_parts(player: usize) -> Option<(usize, usize)> {
    // SAFETY: the player is a live character; these slots take `this` and return a pointer.
    unsafe {
        let asm = vcall0(player, ds2_rva::CHR_ASM_VTABLE_OFFSET)?;
        let model = vcall0(asm, ds2_rva::CHR_ASM_MODEL_VTABLE_OFFSET)?;
        let equip = vcall0(asm, ds2_rva::CHR_ASM_VISIBLE_EQUIP_VTABLE_OFFSET)?;
        Some((model, equip))
    }
}

fn read_block(model: usize) -> Option<Block> {
    // SAFETY: the model is live; the slot returns its appearance block.
    let at = unsafe { vcall0(model, ds2_rva::CHR_MODEL_APPEARANCE_VTABLE_OFFSET) }?;
    let mut block = [0u8; APPEARANCE_BLOCK_LEN];
    // SAFETY: fault-tolerant copy.
    unsafe { read_bytes(at, &mut block) }.then_some(block)
}

fn read_equip(equip: usize) -> [Record; VISIBLE_EQUIP_SLOTS] {
    type ReadFn = extern "system" fn(usize, *mut u8, i32);
    // SAFETY: the RVA's signature is recorded in `ds2-rva`.
    let read: ReadFn =
        unsafe { std::mem::transmute::<usize, ReadFn>(rva(ds2_rva::VISIBLE_EQUIP_READ)) };
    let mut out = [[0u8; VISIBLE_EQUIP_RECORD_LEN]; VISIBLE_EQUIP_SLOTS];
    for (slot, record) in out.iter_mut().enumerate() {
        read(equip, record.as_mut_ptr(), slot as i32);
    }
    out
}

fn hud_suspended() -> Option<u8> {
    let root = {
        // SAFETY: fault-tolerant read.
        unsafe { safe_read_usize(game_manager()? + ds2_rva::GAME_MANAGER_FRONTEND_ROOT_OFFSET) }?
    };
    // SAFETY: fault-tolerant read.
    let hud = unsafe { safe_read_usize(root + ds2_rva::FRONTEND_HUD_OPERATOR_OFFSET) }?;
    if root == 0 || hud == 0 {
        return None;
    }
    // SAFETY: fault-tolerant read.
    unsafe { safe_read_u8(hud + ds2_rva::FRONTEND_HUD_SUSPENDED_OFFSET) }
}

// --- the row ---------------------------------------------------------------------------------

unsafe extern "system" fn build_handler(a: usize, b: usize, c: usize, d: usize) -> usize {
    type LookupFn = extern "system" fn(u32, u32) -> usize;
    // SAFETY: the RVA's signature is recorded in `ds2-rva`.
    let lookup: LookupFn =
        unsafe { std::mem::transmute::<usize, LookupFn>(rva(ds2_rva::FMG_TEXT_LOOKUP)) };
    let label = lookup(
        ds2_rva::BONFIRE_TEXT_CATEGORY,
        ds2_rva::BONFIRE_TEXT_ITEM_BOX,
    );
    ITEM_BOX_LABEL.store(label, Ordering::Release);
    IN_BONFIRE_BUILD.store(true, Ordering::Release);
    // SAFETY: the game's own arguments.
    let result = unsafe { call_orig(&ORIG_BUILD, a, b, c, d) };
    IN_BONFIRE_BUILD.store(false, Ordering::Release);
    result
}

unsafe extern "system" fn add_row_handler(
    builder: usize,
    label: usize,
    slot: usize,
    d: usize,
) -> usize {
    // SAFETY: the game's own arguments.
    let result = unsafe { call_orig(&ORIG_ADD_ROW, builder, label, slot, d) };
    if !IN_BONFIRE_BUILD.load(Ordering::Acquire)
        || label == 0
        || label != ITEM_BOX_LABEL.load(Ordering::Acquire)
        || result == 0
    {
        return result;
    }
    // The add consumes a reference from `*slot` and zeroes it, so it gets a slot of its own.
    let mut ours = CREATOR.as_ptr() as usize;
    // SAFETY: the builder the game just returned, our static label and creator, and a live slot.
    let result = unsafe {
        call_orig(
            &ORIG_ADD_ROW,
            result,
            ROW_LABEL_UTF16.as_ptr() as usize,
            &raw mut ours as usize,
            0,
        )
    };
    if !ROW_ADDED_SAID.swap(true, Ordering::AcqRel) {
        log(format_args!("{LOG_PREFIX} row added below Item box"));
    }
    result
}

/// The job creator's `vtbl+0x10(this, DLReferencePointer<FeJob>* out)`.
///
/// No job closes the bonfire menu; the creator is opened from the tick once the menu has let go.
extern "system" fn invoke(_this: usize, out: *mut usize) -> *mut usize {
    if !out.is_null() {
        // SAFETY: the dialog passes a live out-slot (`0x14001c2b3`) and assigns from it after.
        unsafe { out.write(0) };
    }
    OPEN_REQUESTED.store(true, Ordering::Release);
    log(format_args!("{LOG_PREFIX} row chosen"));
    out
}

// --- the open and the report -----------------------------------------------------------------

fn tick(_session: usize) {
    if open_now(OPEN_REQUESTED.load(Ordering::Acquire), hud_suspended()) {
        OPEN_REQUESTED.store(false, Ordering::Release);
        open();
    }
    let left = REPORT_IN.load(Ordering::Acquire);
    if left > 0 {
        REPORT_IN.store(left - 1, Ordering::Release);
        if left == 1 {
            report();
        }
    }
}

fn open() {
    // Reopened before the last change was reported: report it now, against its own snapshot.
    if REPORT_IN.swap(0, Ordering::AcqRel) > 0 {
        report();
    }
    TABS_HIDDEN.store(true, Ordering::Release);
    TABS_SAID.store(false, Ordering::Release);
    if LAYOUT.load(Ordering::Acquire) != 0 {
        set_layout(Layout::Thirds);
    }
    *lock(&SNAPSHOT) = None;
    *lock(&COMMITTED) = None;
    WAREHOUSE.store(0, Ordering::Release);
    ARMED.store(true, Ordering::Release);
    type OpenFn = extern "system" fn(usize);
    // SAFETY: the RVA's signature is recorded in `ds2-rva`; it ignores its argument. Called on the
    // game thread, outside any menu handler.
    let open: OpenFn =
        unsafe { std::mem::transmute::<usize, OpenFn>(rva(ds2_rva::OPEN_CHARA_MAKER_WINDOW)) };
    open(0);
    log(format_args!("{LOG_PREFIX} creator opened"));
}

fn report() {
    // The creator is gone by now: give the next one, which may be New Game's, its four tabs back.
    TABS_HIDDEN.store(false, Ordering::Release);
    set_layout(Layout::Original);
    let Some((model, equip)) = player().and_then(chr_parts) else {
        log(format_args!("{LOG_PREFIX} done (no player to read back)"));
        return;
    };
    let face_matches = match (read_block(model), *lock(&COMMITTED)) {
        (Some(live), Some(committed)) => Some(live == committed),
        _ => None,
    };
    let differing = lock(&SNAPSHOT)
        .as_ref()
        .map(|before| records_differing(&before.equip, &read_equip(equip)));
    log(format_args!(
        "{LOG_PREFIX} done: live appearance equals the creator's {face_matches:?}, \
         equipment records differing from before {differing:?}"
    ));
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// --- the creator -----------------------------------------------------------------------------

unsafe extern "system" fn enter_handler(this: usize, b: usize, c: usize, d: usize) -> usize {
    let armed = ARMED.load(Ordering::Acquire);
    if armed {
        match player().and_then(chr_parts) {
            Some((_, equip)) => {
                *lock(&SNAPSHOT) = Some(Snapshot {
                    equip: read_equip(equip),
                });
                WAREHOUSE.store(
                    this - ds2_rva::CHARA_MAKER_WAREHOUSE_FROM_ENTER,
                    Ordering::Release,
                );
            }
            None => log(format_args!(
                "{LOG_PREFIX} enter: no player, equipment not saved"
            )),
        }
    }
    // SAFETY: the game's own arguments.
    let result = unsafe { call_orig(&ORIG_ENTER, this, b, c, d) };
    let warehouse = WAREHOUSE.load(Ordering::Acquire);
    if armed && warehouse != 0 {
        let sel = warehouse + ds2_rva::CHARA_MAKER_SELECTION_OFFSET;
        // SAFETY: the selection the Finish gate reads, inside the creator's live warehouse. Any
        // valid row passes, and the only reader that grants anything is skipped while armed.
        unsafe {
            ((sel + ds2_rva::CHARA_MAKER_SELECTION_CLASS_OFFSET) as *mut u32)
                .write(ds2_rva::CHARA_MAKER_PREFILL_CLASS);
            ((sel + ds2_rva::CHARA_MAKER_SELECTION_GIFT_OFFSET) as *mut u32)
                .write(ds2_rva::CHARA_MAKER_PREFILL_GIFT);
            ((sel + ds2_rva::CHARA_MAKER_SELECTION_LIST_OPEN_OFFSET) as *mut u8).write(0);
        }
        log(format_args!(
            "{LOG_PREFIX} creator entered; class and gift pre-filled"
        ));
    }
    result
}

unsafe extern "system" fn set_face_handler(face: usize, block: usize, c: usize, d: usize) -> usize {
    if IN_FACE_COMMIT.load(Ordering::Acquire) && block != 0 {
        let mut copy = [0u8; APPEARANCE_BLOCK_LEN];
        // SAFETY: fault-tolerant copy of the block the face commit passes.
        if unsafe { read_bytes(block, &mut copy) } {
            lock(&COMMITTED).get_or_insert(copy);
        }
    }
    // SAFETY: the game's own arguments.
    unsafe { call_orig(&ORIG_SET_FACE, face, block, c, d) }
}

unsafe extern "system" fn face_commit_handler(this: usize, b: usize, c: usize, d: usize) -> usize {
    let armed = ARMED.load(Ordering::Acquire);
    IN_FACE_COMMIT.store(armed, Ordering::Release);
    // SAFETY: the game's own arguments.
    let result = unsafe { call_orig(&ORIG_FACE_COMMIT, this, b, c, d) };
    IN_FACE_COMMIT.store(false, Ordering::Release);
    if armed {
        put_back();
    }
    result
}

/// Import the committed block into the player's face part and restore the equipment the enter
/// stripped.
fn put_back() {
    let Some(player) = player() else { return };
    let Some((model, equip)) = chr_parts(player) else {
        return;
    };
    if let Some(block) = *lock(&COMMITTED) {
        type PartFn = extern "system" fn(usize) -> usize;
        // SAFETY: the RVA's signature is recorded in `ds2-rva`.
        let part_of: PartFn =
            unsafe { std::mem::transmute::<usize, PartFn>(rva(ds2_rva::CHR_MODEL_FACE_PART)) };
        let part = part_of(model);
        // SAFETY: fault-tolerant reads of the part's vtable slot.
        let import = (part != 0)
            .then(|| unsafe { safe_read_usize(part) })
            .flatten()
            // SAFETY: as above.
            .and_then(|vtbl| unsafe {
                safe_read_usize(vtbl + ds2_rva::FACE_PART_IMPORT_VTABLE_OFFSET)
            });
        match import {
            Some(f) if f != 0 => {
                type ImportFn = extern "system" fn(usize, *const u8);
                // SAFETY: the face part's import slot, `void(part, const u8* block)`.
                let import: ImportFn = unsafe { std::mem::transmute::<usize, ImportFn>(f) };
                import(part, block.as_ptr());
            }
            _ => log(format_args!(
                "{LOG_PREFIX} player has no face part: import skipped"
            )),
        }
    } else {
        log(format_args!(
            "{LOG_PREFIX} face commit passed no block: import skipped"
        ));
    }
    if let Some(snapshot) = lock(&SNAPSHOT).as_ref() {
        type WriteFn = extern "system" fn(usize, i32, *const u8);
        type RefreshFn = extern "system" fn(usize);
        // SAFETY: the RVAs' signatures are recorded in `ds2-rva`.
        let (write, refresh): (WriteFn, RefreshFn) = unsafe {
            (
                std::mem::transmute::<usize, WriteFn>(rva(ds2_rva::VISIBLE_EQUIP_WRITE)),
                std::mem::transmute::<usize, RefreshFn>(rva(ds2_rva::CHR_REFRESH_VISIBLE_EQUIP)),
            )
        };
        for (slot, record) in snapshot.equip.iter().enumerate() {
            write(equip, slot as i32, record.as_ptr());
        }
        refresh(player);
    }
    log(format_args!(
        "{LOG_PREFIX} appearance imported, equipment put back"
    ));
}

unsafe extern "system" fn class_gift_handler(this: usize, b: usize, c: usize, d: usize) -> usize {
    if ARMED.swap(false, Ordering::AcqRel) {
        log(format_args!("{LOG_PREFIX} class and gift commit skipped"));
        REPORT_IN.store(REPORT_AFTER_TICKS, Ordering::Release);
        return 0;
    }
    // SAFETY: the game's own arguments.
    unsafe { call_orig(&ORIG_CLASS_GIFT, this, b, c, d) }
}

// --- the tab bar -----------------------------------------------------------------------------

unsafe extern "system" fn top_tabs_handler(this: usize, out: usize, c: usize, d: usize) -> usize {
    let hide = TABS_HIDDEN.load(Ordering::Acquire);
    IN_TOP_TABS.store(hide, Ordering::Release);
    CLASS_TAB_SKIPPED.store(false, Ordering::Release);
    // SAFETY: the game's own arguments.
    let result = unsafe { call_orig(&ORIG_TOP_TABS, this, out, c, d) };
    IN_TOP_TABS.store(false, Ordering::Release);
    if hide && !TABS_SAID.swap(true, Ordering::AcqRel) {
        log(format_args!(
            "{LOG_PREFIX} tab bar built without Class & gift (skipped={})",
            CLASS_TAB_SKIPPED.load(Ordering::Acquire)
        ));
    }
    result
}

unsafe extern "system" fn tab_append_handler(
    list: usize,
    spec: usize,
    c: usize,
    d: usize,
) -> usize {
    // The builder's first append is Class & gift. Not copying it is clean: the builder releases
    // its own spec after every append either way.
    if IN_TOP_TABS.load(Ordering::Acquire) && !CLASS_TAB_SKIPPED.swap(true, Ordering::AcqRel) {
        return 0;
    }
    // SAFETY: the game's own arguments.
    unsafe { call_orig(&ORIG_TAB_APPEND, list, spec, c, d) }
}

unsafe extern "system" fn flo_adopt_handler(
    holder: usize,
    bytes: usize,
    len: usize,
    d: usize,
) -> usize {
    // SAFETY: the game's own arguments.
    let result = unsafe { call_orig(&ORIG_FLO_ADOPT, holder, bytes, len, d) };
    if result & 0xff != 0 && len as u32 as usize == ds2_rva::CHARA_MAKE_FLO_LEN && bytes != 0 {
        LAYOUT.store(bytes, Ordering::Release);
        log(format_args!(
            "{LOG_PREFIX} creator layout adopted at 0x{bytes:x}"
        ));
        if TABS_HIDDEN.load(Ordering::Acquire) {
            set_layout(Layout::Thirds);
        }
    }
    result
}

/// Put the creator's cached layout in `to`, if it is the layout [`tabs`] knows in either state.
fn set_layout(to: Layout) {
    let base = LAYOUT.load(Ordering::Acquire);
    if base == 0 {
        return;
    }
    // SAFETY: fault-tolerant reads; a freed block reads as something that is not the layout.
    let now = layout_of(|at, out| unsafe { read_bytes(base + at, out) });
    match now {
        Some(now) if now == to => {}
        Some(_) => {
            for (at, bytes) in writes(to) {
                // SAFETY: `layout_of` just found all eight records and their transforms at these
                // offsets, inside the heap copy the game parsed in place and builds from.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        bytes.as_ptr(),
                        (base + at) as *mut u8,
                        bytes.len(),
                    );
                }
            }
            log(format_args!("{LOG_PREFIX} creator layout set to {to:?}"));
        }
        None => log(format_args!(
            "{LOG_PREFIX} creator layout at 0x{base:x} not recognised; left alone"
        )),
    }
}

// --- install ---------------------------------------------------------------------------------

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The row, the creator's hooks, the heap patch and the tick are all live.
    pub installed: bool,
}

/// Hook the bonfire menu and the creator, patch the bundle's heap argument and register the tick.
///
/// The patch site is read first and nothing is touched if it is not the expected instruction. A
/// hook that fails after others went in leaves them inert: none of them acts unless the row is
/// chosen, and the row is hooked last.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Call from the loader's post-Arxan callback.
pub unsafe fn install() -> Outcome {
    let failed = Outcome { installed: false };
    let base = match game_module_base() {
        Ok(base) => base,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} install-failed stage=module-base error={error}"
            ));
            return failed;
        }
    };
    BASE.store(base, Ordering::Release);

    let site = rva(ds2_rva::CHARA_MAKER_BUNDLE_HEAP_ARG);
    let mut found = [0u8; 3];
    // SAFETY: fault-tolerant read of a byte range in the image.
    if !unsafe { read_bytes(site, &mut found) }
        || found != ds2_rva::CHARA_MAKER_BUNDLE_HEAP_ARG_ORIGINAL
    {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=heap-arg site=0x{site:x} found={found:02x?} expected={:02x?}",
            ds2_rva::CHARA_MAKER_BUNDLE_HEAP_ARG_ORIGINAL
        ));
        return failed;
    }

    // Alone, the patch only moves the creator's bundle to another heap, which is harmless.
    if !patch_3byte_stub(
        base,
        ds2_rva::CHARA_MAKER_BUNDLE_HEAP_ARG as usize,
        ds2_rva::CHARA_MAKER_BUNDLE_HEAP_ARG_ORIGINAL[0],
        ds2_rva::CHARA_MAKER_BUNDLE_HEAP_ARG_PATCH,
        LOG_PREFIX,
    ) {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=heap-patch site=0x{site:x}"
        ));
        return failed;
    }

    VTABLE[0].store(rva(ds2_rva::FE_JOB_CREATOR_SLOT0), Ordering::Release);
    VTABLE[1].store(rva(ds2_rva::FE_JOB_CREATOR_SLOT1), Ordering::Release);
    VTABLE[2].store(invoke as *const () as usize, Ordering::Release);
    CREATOR[0].store(VTABLE.as_ptr() as usize, Ordering::Release);
    CREATOR[ds2_rva::FE_JOB_CREATOR_REFCOUNT_OFFSET / 8].store(CREATOR_REFCOUNT, Ordering::Release);

    if let Err(error) = {
        // SAFETY: the caller's contract.
        unsafe { ds2_net_tick::register(ds2_net_tick::When::After, tick) }
    } {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=tick error={error}"
        ));
        return failed;
    }

    // The row goes last, so a failure before it leaves a game where nothing of ours can arm.
    let hooks: [(&str, u32, UnionFn, &'static AtomicUsize); 9] = [
        (
            "flo-adopt",
            ds2_rva::FLO_ADOPT,
            flo_adopt_handler,
            &ORIG_FLO_ADOPT,
        ),
        (
            "tab-append",
            ds2_rva::FE_TAB_SPEC_APPEND,
            tab_append_handler,
            &ORIG_TAB_APPEND,
        ),
        (
            "top-tabs",
            ds2_rva::CHARA_MAKER_TOP_TABS_BUILD,
            top_tabs_handler,
            &ORIG_TOP_TABS,
        ),
        (
            "class-gift-commit",
            ds2_rva::CHARA_MAKER_CLASS_GIFT_COMMIT,
            class_gift_handler,
            &ORIG_CLASS_GIFT,
        ),
        (
            "set-face-data",
            ds2_rva::SET_FACE_DATA,
            set_face_handler,
            &ORIG_SET_FACE,
        ),
        (
            "face-commit",
            ds2_rva::CHARA_MAKER_FACE_COMMIT,
            face_commit_handler,
            &ORIG_FACE_COMMIT,
        ),
        (
            "creator-enter",
            ds2_rva::CHARA_MAKER_ENTER,
            enter_handler,
            &ORIG_ENTER,
        ),
        (
            "add-row",
            ds2_rva::FEX_COMMAND_DIALOG_ADD_ROW,
            add_row_handler,
            &ORIG_ADD_ROW,
        ),
        (
            "bonfire-build",
            ds2_rva::BONFIRE_MENU_BUILD,
            build_handler,
            &ORIG_BUILD,
        ),
    ];
    for (name, target, handler, orig) in hooks {
        // SAFETY: each target takes at most four integer/pointer arguments (read in the
        // disassembly), and `orig` is the static its handler calls through.
        if let Err(status) = unsafe { register_union_hook(rva(target), handler, orig) } {
            log(format_args!(
                "{LOG_PREFIX} install-failed stage=hook site={name} rva=0x{target:08x} status={status:?}"
            ));
            return failed;
        }
    }

    log(format_args!(
        "{LOG_PREFIX} installed: bundle heap patch, tick, creator hooks, tab bar, row"
    ));
    Outcome { installed: true }
}
