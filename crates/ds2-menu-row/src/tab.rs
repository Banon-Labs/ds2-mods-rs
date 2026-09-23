//! A seventh tab on the pause menu, built out of the game's own constructor into storage we own.
//!
//! # Why a tab and not more rows
//!
//! The row work put four added rows on the System tab, which already ships three. The user's words
//! on seeing it, 2026-09-23: "I asked for an additional tab so we could see all of the options ...
//! not to overcrowd the already crowded system tab." Seven rows under one gear icon is a list; the
//! rows belong somewhere of their own.
//!
//! # Where the number six is written down, and what this does about each
//!
//! `ds2-rva` carries the full reading beside [`ds2_rva::FE_INGAME_TOP_SELECT_CTOR`]. In short there
//! are six spellings of "six" and five of them are a literal inside a function, which is a detour
//! site:
//!
//! | spelling | what happens here |
//! |---|---|
//! | the six groups are inline in `FeGroupInGameTopSelect` | ours is not -- see below |
//! | [`ds2_rva::FE_INGAME_TOP_SELECT_TAB_TABLE`] returns null above index 5 | [`tab_table_detour`] |
//! | the strip's namer list is full at six | the stand-in `crate::install` already serves cells from |
//! | [`ds2_rva::FE_INGAME_TOP_SELECT_STRIP_INIT`] sets the count with a literal `6` | [`strip_init_detour`] |
//! | the caption path table holds five entries | nothing: six tabs already take its empty arm |
//! | the strip's `.flo` container holds six cell records | `crate::layout`, same raise a row tab gets |
//!
//! # The group is constructed by the game, not by us
//!
//! [`ds2_rva::FE_INGAME_GROUP_SELECT_CTOR`] writes nothing outside `group[0 ..= 0x2c]`, so a
//! [`ds2_rva::FE_INGAME_GROUP_SELECT_SIZE`] buffer is a complete one. Calling it is what makes this
//! a seventh tab rather than a Rust object that resembles one: the vtable, the item-vector copy, the
//! namer reference and the proxy are all installed by the same code that installs them for the six.
//!
//! What it is handed comes from the game as well. The System tab's own two builders produce a
//! descriptor and a namer, called through their trampolines so this crate's own row additions do
//! not appear twice, and then the descriptor's item vector is rewritten to carry our actions and
//! nothing else.
//!
//! **The layout path is deliberately left as the System tab's.** A group locates the container it
//! draws rows into through `group + 0x130`, copied out of `descriptor + 0x38`
//! ([`ds2_rva::FE_INGAME_MENU_TAB_INIT`] resolves it and plays sequence `0x65` on the result). Ours
//! points at the same panel the System tab uses, which is correct because only one tab is bound at a
//! time: switching to this tab rebinds that panel with our rows, and switching away rebinds it with
//! the System tab's three. It also means every piece of row machinery this crate already has -- the
//! panel substitution, the caret, the banner, the captions -- applies without a second copy.
//!
//! # What is not established
//!
//! Nothing in this module has been in front of a running game. The riskiest piece is not any one
//! detour but the lifetime: the top select is constructed once per `FeSceneInGame` and this crate
//! never sees it destroyed, so the group is built once and reused, and [`TOP_SELECT`] is compared on
//! every lookup so a second scene cannot be served a group built against the first one's proxy.

use core::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::LOG_PREFIX;
use crate::install::log;

/// `FUN_1400a41b0(topSelect, a, b) -> topSelect`.
type TopSelectCtorFn = unsafe extern "system" fn(*mut u8, usize, usize) -> *mut u8;
/// `FUN_1400a66d0(topSelect) -> *group`, null above the last tab.
type TabTableFn = unsafe extern "system" fn(*mut u8) -> *mut u8;
/// `FUN_1400a6da0(topSelect)`.
type StripInitFn = unsafe extern "system" fn(*mut u8);
/// `FUN_1400a40e0(group, proxy, *namer, descriptor) -> group`.
type GroupCtorFn = unsafe extern "system" fn(*mut u8, usize, *mut usize, *mut u8) -> *mut u8;
/// `FUN_1400a5b50(*out, proxy) -> ..` -- the System tab's cell namer builder.
type NamerBuilderFn = unsafe extern "system" fn(*mut usize, usize) -> *mut u8;
/// `FUN_140021b30(grid, count)`.
type GridSetItemCountFn = unsafe extern "system" fn(*mut u8, u64);
/// `FUN_140022140(grid) -> i32`.
type GridCurrentIndexFn = unsafe extern "system" fn(*mut u8) -> i32;
/// `FUN_1400a4d20(group)`.
type TabInitFn = unsafe extern "system" fn(*mut u8);

static CTOR_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static TAB_TABLE_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static STRIP_INIT_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// The live module base, stored by [`install`] so no detour has to ask for it.
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);

/// The top select our group was built against. Zero until the constructor detour has accepted one.
///
/// Compared on every [`tab_table_detour`] call rather than assumed, because the group holds that
/// top select's layout proxy: handing it to a second scene would draw into a freed one.
static TOP_SELECT: AtomicUsize = AtomicUsize::new(0);

/// Our constructed `FeGroupInGameGroupSelect`. Zero until the construction succeeded.
static GROUP: AtomicUsize = AtomicUsize::new(0);

/// How many rows this tab carries, which is every row the registry holds.
static ROWS: AtomicUsize = AtomicUsize::new(0);

/// Whether the three sites went in, decided once at [`install`] and never again.
///
/// **This, and not [`GROUP`], is what the System tab's own detours ask.** They run inside
/// `FUN_1400a41b0` -- before its detour's post-step has built anything -- so a group pointer is
/// still zero when the question is asked and would send every row onto the System tab as well. The
/// first run of this module did exactly that: the rows appeared on both tabs at once.
static ARMED: AtomicBool = AtomicBool::new(false);

/// True only while [`build_group`] is inside the game's namer builder.
///
/// The builder is detoured, and that detour is what adds a cell per row. Our call has to get those
/// cells and the System tab's own call must not, and the two are the same function -- so the
/// difference has to be a flag, and it is this one. Set and cleared around a single synchronous
/// call on the thread that is constructing the scene.
static BUILDING: AtomicBool = AtomicBool::new(false);

/// Counters, so a run's log says which of the three detours actually did anything.
static GROUPS_BUILT: AtomicUsize = AtomicUsize::new(0);
static TABS_SERVED: AtomicUsize = AtomicUsize::new(0);
static STRIPS_RAISED: AtomicUsize = AtomicUsize::new(0);

/// Bytes the System tab's item builder writes into its stack descriptor.
///
/// `FUN_1400a5900` addresses `descriptor + 0x30` as the item count, `+0x38` as the layout path and
/// `+0x60` as a second count it zeroes, and its callers give it a 144-byte stack slot. That slot
/// size is what is reproduced here rather than the highest offset touched, because
/// [`ds2_rva::FE_INGAME_GROUP_SELECT_CTOR`] reads `descriptor + 0x38` through a copy whose own
/// extent this crate has not measured.
const DESCRIPTOR_SIZE: usize = 144;

/// A descriptor buffer with the alignment the item vector's own addressing assumes.
///
/// The builder computes element `n` at `descriptor + (-(int)descriptor & 3) + n * 8`. On a
/// 16-aligned buffer that padding term is zero, which is the case every one of the game's own
/// callers produces by putting the descriptor on an aligned stack slot.
#[repr(C, align(16))]
struct Descriptor([u8; DESCRIPTOR_SIZE]);

/// The group's storage, 16-aligned for the same reason the descriptor is.
#[repr(C, align(16))]
struct GroupStorage([u8; ds2_rva::FE_INGAME_GROUP_SELECT_SIZE]);

/// Whether a pointer is plausibly a live object rather than a small integer or a misalignment.
fn sane(pointer: usize) -> bool {
    pointer >= 0x1_0000 && pointer.is_multiple_of(8)
}

/// Write `entries` into a descriptor's item vector, replacing whatever the builder left there.
///
/// # Safety
///
/// `descriptor` must be a [`DESCRIPTOR_SIZE`] buffer the game's own item builder has just filled.
unsafe fn rewrite_items(descriptor: *mut u8, entries: &[(u32, u32)]) -> bool {
    if entries.len() > ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY {
        return false;
    }
    let padding = (descriptor as usize).wrapping_neg() & 3;
    for (index, (action, gate)) in entries.iter().enumerate() {
        // SAFETY: `index < capacity`, so this is inside the vector's own inline storage, at the
        // address the builder and both readers compute for element `index`.
        unsafe {
            let entry = descriptor.add(padding + index * ds2_rva::FE_INGAME_MENU_ITEM_STRIDE);
            entry.cast::<u32>().write(*action);
            entry.add(4).cast::<u32>().write(*gate);
        }
    }
    // The count LAST, so a torn write cannot leave a count that outruns the entries. Nothing else
    // runs while this does, but the ordering costs nothing and the reverse is a habit worth not
    // having.
    // SAFETY: the count field of the same vector.
    unsafe {
        descriptor
            .add(ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_COUNT_OFFSET)
            .cast::<u64>()
            .write(entries.len() as u64);
    }
    true
}

/// Build the seventh group, once, against `top_select`.
///
/// Returns the group, or `None` with a log line saying which step declined.
///
/// # Safety
///
/// `top_select` must be a `FeGroupInGameTopSelect` whose own constructor has just returned, and
/// `base` the live module base.
unsafe fn build_group(base: usize, top_select: *mut u8, entries: &[(u32, u32)]) -> Option<*mut u8> {
    if entries.is_empty() {
        return None;
    }
    let proxy = top_select as usize + ds2_rva::FE_INGAME_TOP_SELECT_PROXY_OFFSET;

    // THE DESCRIPTOR, from the game's own builder and then rewritten. Through the trampoline rather
    // than the live entry, so this crate's own row additions -- which are aimed at the System tab --
    // do not also land here and give the new tab the shipped three rows on top of ours.
    // SAFETY: the caller guarantees `base` is the live module base, which is this function's own
    // safety condition.
    let builder = unsafe { crate::install::item_builder_original(base) };
    let mut descriptor = Box::new(Descriptor([0u8; DESCRIPTOR_SIZE]));
    let descriptor_ptr = descriptor.0.as_mut_ptr();
    // SAFETY: `builder` is the System tab's item builder, and the argument is a 16-aligned buffer
    // of the size its own callers give it.
    unsafe { builder(descriptor_ptr) };
    // AS MANY AS THE VECTOR HOLDS, and the rest are served by `crate::install`'s item lookup out of
    // its own entries -- the same split the System tab uses, with the difference that on this tab
    // none of the five slots is spoken for. Writing more than five here would be refused by the
    // game's own copy (`if (5 < src[0x30]) panic`) inside the constructor two calls later.
    let fits = entries
        .len()
        .min(ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY);
    // SAFETY: the game's own builder has just filled this descriptor, which is what `rewrite_items`
    // requires of it.
    if !unsafe { rewrite_items(descriptor_ptr, &entries[..fits]) } {
        log(format_args!(
            "{LOG_PREFIX} tab NOT built stage=items rows={} of-capacity={}",
            entries.len(),
            ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY
        ));
        return None;
    }

    // THE NAMER, from the game's own builder. Its cells are the System tab's, which is what this
    // tab wants: the rows are drawn in the same panel's row slots, so row 0 takes the slot the first
    // shipped row takes. Rows past the list's three are served by `crate::install`'s stand-ins.
    // SAFETY: the RVA is a `.pdata` function start recorded in `ds2-rva`, and `proxy` is the value
    // the game's own constructor passes it one call later.
    let namer_builder: NamerBuilderFn =
        unsafe { std::mem::transmute(base + ds2_rva::FE_INGAME_MENU_QUIT_TAB_NAMER as usize) };
    let mut namer: usize = 0;
    // THE FLAG IS THE ONLY THING THAT TELLS THE TWO CALLS APART. `crate::install`'s detour on this
    // builder adds a cell per row, and it must do that for this call and not for the System tab's.
    // Cleared unconditionally after, including on the paths that refuse below.
    BUILDING.store(true, Ordering::Release);
    // SAFETY: as above; the out-parameter is a live local this function owns.
    unsafe { namer_builder(&raw mut namer, proxy) };
    BUILDING.store(false, Ordering::Release);
    if !sane(namer) {
        log(format_args!(
            "{LOG_PREFIX} tab NOT built stage=namer namer=0x{namer:016x}"
        ));
        return None;
    }

    // THE GROUP. `FUN_1400a40e0` consumes the namer reference, which is why the namer is built
    // immediately above and handed straight over rather than cached.
    let storage = Box::leak(Box::new(GroupStorage(
        [0u8; ds2_rva::FE_INGAME_GROUP_SELECT_SIZE],
    )));
    let group = storage.0.as_mut_ptr();
    // SAFETY: the RVA is a `.pdata` function start recorded in `ds2-rva`; `group` is a zeroed
    // buffer of exactly the size the constructor writes into, `proxy` and `descriptor` are what the
    // game's own six calls pass, and `namer` is a reference this call takes ownership of.
    let group_ctor: GroupCtorFn =
        unsafe { std::mem::transmute(base + ds2_rva::FE_INGAME_GROUP_SELECT_CTOR as usize) };
    // SAFETY: as above.
    let built = unsafe { group_ctor(group, proxy, &raw mut namer, descriptor_ptr) };
    if built != group {
        log(format_args!(
            "{LOG_PREFIX} tab NOT built stage=ctor returned=0x{:016x} expected=0x{:016x}",
            built as usize, group as usize
        ));
        return None;
    }
    // WHAT THE CONSTRUCTOR MUST HAVE LEFT BEHIND, checked rather than assumed: the vtable it writes
    // twice, and the item count it copied. A group whose vtable slot is empty would be a crash on
    // the first virtual call with this crate nowhere on the stack.
    // SAFETY: the constructor has just returned against this buffer.
    let (vtable, count) = unsafe {
        (
            group.cast::<usize>().read(),
            group
                .add(ds2_rva::FE_INGAME_MENU_TAB_ITEM_VECTOR_OFFSET)
                .add(ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_COUNT_OFFSET)
                .cast::<u64>()
                .read() as usize,
        )
    };
    if !sane(vtable) || count != entries.len() {
        log(format_args!(
            "{LOG_PREFIX} tab NOT built stage=verify vtable=0x{vtable:016x} count={count} \
             expected={}",
            entries.len()
        ));
        return None;
    }
    let n = GROUPS_BUILT.fetch_add(1, Ordering::Relaxed) + 1;
    log(format_args!(
        "{LOG_PREFIX} tab built group=0x{:016x} proxy=0x{proxy:016x} rows={} vtable=0x{vtable:016x} \
         builds={n}",
        group as usize,
        entries.len()
    ));
    Some(group)
}

/// The top select's constructor: run the game's, then build ours behind it.
unsafe extern "system" fn ctor_detour(top_select: *mut u8, a: usize, b: usize) -> *mut u8 {
    let trampoline = CTOR_TRAMPOLINE.load(Ordering::Acquire);
    let result = if trampoline == 0 {
        top_select
    } else {
        // SAFETY: MinHook published this trampoline for exactly this site, and the signature is the
        // one the disassembled entry implements.
        let original: TopSelectCtorFn = unsafe { std::mem::transmute(trampoline) };
        // SAFETY: every argument is the game's own, forwarded unchanged.
        unsafe { original(top_select, a, b) }
    };
    let base = MODULE_BASE.load(Ordering::Acquire);
    let entries = crate::install::registered_entries();
    if base != 0 && !entries.is_empty() && sane(top_select as usize) {
        // ONE GROUP PER PROCESS, and the top select it was built against is remembered with it. A
        // second `FeSceneInGame` gets no seventh tab rather than one holding a stale proxy -- the
        // visible failure instead of the invisible one.
        if GROUP.load(Ordering::Acquire) == 0 {
            // SAFETY: the game's own constructor has just returned against this pointer.
            if let Some(group) = unsafe { build_group(base, top_select, &entries) } {
                ROWS.store(entries.len(), Ordering::Release);
                GROUP.store(group as usize, Ordering::Release);
                TOP_SELECT.store(top_select as usize, Ordering::Release);
            }
        } else if TOP_SELECT.load(Ordering::Acquire) != top_select as usize {
            log(format_args!(
                "{LOG_PREFIX} tab NOT offered to this scene -- the group was built against \
                 0x{:016x} and this top select is 0x{:016x}",
                TOP_SELECT.load(Ordering::Acquire),
                top_select as usize
            ));
        }
    }
    result
}

/// The tab lookup: ours when the cursor is on the seventh tab, the game's otherwise.
unsafe extern "system" fn tab_table_detour(top_select: *mut u8) -> *mut u8 {
    let group = GROUP.load(Ordering::Acquire);
    let base = MODULE_BASE.load(Ordering::Acquire);
    if group != 0 && base != 0 && top_select as usize == TOP_SELECT.load(Ordering::Acquire) {
        // SAFETY: the RVA is a `.pdata` function start recorded in `ds2-rva`, and the original reads
        // the cursor off this same object one instruction into itself.
        let current: GridCurrentIndexFn =
            unsafe { std::mem::transmute(base + ds2_rva::FE_INGAME_TOP_SELECT_TAB_INDEX as usize) };
        // SAFETY: as above.
        let index = unsafe { current(top_select) };
        if index >= 0 && index as usize == ds2_rva::FE_INGAME_TOP_SELECT_TABS {
            let n = TABS_SERVED.fetch_add(1, Ordering::Relaxed) + 1;
            if n <= 2 {
                log(format_args!(
                    "{LOG_PREFIX} tab served index={index} group=0x{group:016x} served={n} \
                     -- past the game's six-entry table, out of our own group"
                ));
            }
            return group as *mut u8;
        }
    }
    let trampoline = TAB_TABLE_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        // Null is what the original returns above the last tab, and every caller tests for it, so
        // it is the safe answer here as well.
        return std::ptr::null_mut();
    }
    // SAFETY: MinHook published this trampoline for exactly this site.
    let original: TabTableFn = unsafe { std::mem::transmute(trampoline) };
    // SAFETY: the argument is the game's own.
    unsafe { original(top_select) }
}

/// The strip's init: run the game's, then raise its cell count and initialise our group.
unsafe extern "system" fn strip_init_detour(top_select: *mut u8) {
    let trampoline = STRIP_INIT_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for exactly this site.
        let original: StripInitFn = unsafe { std::mem::transmute(trampoline) };
        // SAFETY: the argument is the game's own.
        unsafe { original(top_select) };
    }
    let group = GROUP.load(Ordering::Acquire);
    let base = MODULE_BASE.load(Ordering::Acquire);
    if group == 0 || base == 0 || top_select as usize != TOP_SELECT.load(Ordering::Acquire) {
        return;
    }
    // OUR GROUP GETS THE SAME INIT THE SIX GET, and it gets it before the count is raised: the
    // init is what binds the group's layout and sets its own item count, so a cursor that could
    // reach the tab before it was bound would reach an unbound one.
    // SAFETY: the RVA is a `.pdata` function start recorded in `ds2-rva`, and the argument is a
    // group the game's own constructor built.
    let tab_init: TabInitFn =
        unsafe { std::mem::transmute(base + ds2_rva::FE_INGAME_MENU_TAB_INIT as usize) };
    // SAFETY: as above.
    unsafe { tab_init(group as *mut u8) };

    let want = ds2_rva::FE_INGAME_TOP_SELECT_TABS + 1;
    // SAFETY: the RVA is a `.pdata` function start recorded in `ds2-rva`, and `top_select` is the
    // grid the original just called the same setter on with `6`.
    let set_item_count: GridSetItemCountFn =
        unsafe { std::mem::transmute(base + ds2_rva::FEX_GRID_SET_ITEM_COUNT as usize) };
    // SAFETY: as above.
    unsafe { set_item_count(top_select, want as u64) };
    let n = STRIPS_RAISED.fetch_add(1, Ordering::Relaxed) + 1;
    // SAFETY: the setter has just written this field.
    let items = unsafe {
        top_select
            .add(ds2_rva::FEX_GRID_ITEM_COUNT_OFFSET)
            .cast::<u32>()
            .read()
    };
    log(format_args!(
        "{LOG_PREFIX} strip count raised tabs={} -> items={items} tab-rows={} raises={n}{}",
        ds2_rva::FE_INGAME_TOP_SELECT_TABS,
        rows(),
        if items as usize == want {
            ""
        } else {
            " -- MISMATCH: the strip kept its own count, so the seventh tab cannot be reached"
        }
    ));
}

/// Install the three sites. All or nothing: a refusal leaves the six tabs the game shipped.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. `base` must be the live module base and
/// MinHook must already be initialised, which [`crate::install::install`] guarantees.
pub unsafe fn install(base: usize) -> bool {
    MODULE_BASE.store(base, Ordering::Release);
    let sites: [(u32, &[u8], *mut c_void, &AtomicUsize, &str); 3] = [
        (
            ds2_rva::FE_INGAME_TOP_SELECT_CTOR,
            &ds2_rva::FE_INGAME_TOP_SELECT_CTOR_PROLOGUE,
            ctor_detour as *mut c_void,
            &CTOR_TRAMPOLINE,
            "tab ctor",
        ),
        (
            ds2_rva::FE_INGAME_TOP_SELECT_TAB_TABLE,
            &ds2_rva::FE_INGAME_TOP_SELECT_TAB_TABLE_PROLOGUE,
            tab_table_detour as *mut c_void,
            &TAB_TABLE_TRAMPOLINE,
            "tab lookup",
        ),
        (
            ds2_rva::FE_INGAME_TOP_SELECT_STRIP_INIT,
            &ds2_rva::FE_INGAME_TOP_SELECT_STRIP_INIT_PROLOGUE,
            strip_init_detour as *mut c_void,
            &STRIP_INIT_TRAMPOLINE,
            "tab strip init",
        ),
    ];
    for (rva, prologue, detour, trampoline, what) in sites {
        // SAFETY: each RVA is a `.pdata` function start recorded in `ds2-rva` with its own prologue
        // beside it, and `hook_site` refuses unless the bytes at the site are those.
        if !unsafe { crate::install::hook_site(base, rva, prologue, detour, trampoline, what) } {
            return false;
        }
    }
    // PUBLISHED LAST, and this is what moves the rows. Until it is set the System tab's own detours
    // behave exactly as they did before this module existed; after it, they leave the System tab
    // alone and the rows belong to the seventh tab.
    ARMED.store(true, Ordering::Release);
    true
}

/// Whether the seventh tab's sites are in, which is what decides where a row goes.
///
/// Asked by the System tab's detours, which run before any group exists -- see [`ARMED`].
pub(crate) fn armed() -> bool {
    ARMED.load(Ordering::Acquire)
}

/// Whether the namer being constructed right now is the seventh tab's.
pub(crate) fn building() -> bool {
    BUILDING.load(Ordering::Acquire)
}

/// A pointer to the seventh group, for the detours in [`crate::install`] that have to recognise it.
///
/// Zero before the constructor detour has built one, which every caller must treat as "there is no
/// seventh tab" rather than as an error.
pub fn group() -> usize {
    GROUP.load(Ordering::Acquire)
}

/// How many rows the seventh tab carries.
pub fn rows() -> usize {
    ROWS.load(Ordering::Acquire)
}

/// The prologue bytes this module expects, for a test that does not need the game.
#[cfg(test)]
mod tests {
    /// The group buffer covers everything the constructor writes, and is 16-aligned.
    ///
    /// Both are load-bearing: a short buffer is a heap overrun inside the game's own constructor,
    /// and a misaligned one shifts the item vector's inline elements by up to three bytes because
    /// the vector's addressing adds `-(int)base & 3`.
    ///
    /// **At least, not exactly.** `0x168` is not a multiple of sixteen, so `align(16)` rounds the
    /// struct up to `0x170` and the last eight bytes are padding the game never sees. This test
    /// asserted equality first and failed on that eight, which is the assertion doing its job: the
    /// number that has to match the game is the array's length, not the struct's size.
    #[test]
    fn the_group_buffer_is_the_size_and_alignment_the_constructor_assumes() {
        assert_eq!(
            size_of::<super::GroupStorage>() % align_of::<super::GroupStorage>(),
            0
        );
        assert!(size_of::<super::GroupStorage>() >= ds2_rva::FE_INGAME_GROUP_SELECT_SIZE);
        assert_eq!(align_of::<super::GroupStorage>(), 16);
        assert_eq!(align_of::<super::Descriptor>(), 16);
        assert!(size_of::<super::Descriptor>() >= super::DESCRIPTOR_SIZE);
        // A 16-aligned buffer makes the vector's own padding term zero, which is the case every one
        // of the game's six calls produces.
        assert_eq!(16usize.wrapping_neg() & 3, 0);
    }

    /// The seventh tab's index is one past the six the game ships, and the strip has room for it.
    #[test]
    fn the_seventh_tab_is_inside_the_grids_own_column_bound() {
        assert_eq!(ds2_rva::FE_INGAME_TOP_SELECT_TABS, 6);
        const { assert!(ds2_rva::FE_INGAME_TOP_SELECT_TABS < ds2_rva::FEX_GRID_MAX_COLS) };
    }

    /// The group is one stride, which is how six of them tile the top select without a gap.
    #[test]
    fn a_group_is_exactly_one_tab_stride() {
        assert_eq!(
            ds2_rva::FE_INGAME_GROUP_SELECT_SIZE,
            ds2_rva::FE_INGAME_TOP_SELECT_TAB_STRIDE
        );
        let offsets = ds2_rva::FE_INGAME_TOP_SELECT_TAB_OFFSETS;
        for pair in offsets.windows(2) {
            assert_eq!(pair[1] - pair[0], ds2_rva::FE_INGAME_GROUP_SELECT_SIZE);
        }
        assert_eq!(
            offsets[offsets.len() - 1] + ds2_rva::FE_INGAME_GROUP_SELECT_SIZE,
            ds2_rva::FE_INGAME_TOP_SELECT_AFTER_TABS
        );
    }
}
