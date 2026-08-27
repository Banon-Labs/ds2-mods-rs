//! Draw the title menu's unavailable rows instead of hiding them.
//!
//! # What the game does
//!
//! The top menu is a fixed vector of **six rows, always**. `0x1400f4250` appends the same six
//! descriptors on every path -- there is no branch that skips an append, and the count is 6 unless
//! the fixed vector overflows and panics. **Nothing is ever inserted or removed.** The only
//! per-row variable is one byte, [`ds2_rva::FE_TOP_MENU_ROW_ENABLED_OFFSET`], computed from three
//! facts: a save exists, online is available, and its inverse.
//!
//! [`ds2_rva::FE_TOP_MENU_APPLY_STATES`] turns that byte into **two independent writes**:
//!
//! | | available | unavailable |
//! | --- | --- | --- |
//! | cell state at `+0x8` | `3`, or `4` under the cursor | `2` |
//! | sequence played | `0x67` | `0x7a` |
//!
//! The state is what removes a row from cursor navigation: `FeObjectButtonEx::v16` at
//! `0x14004c5c0` is `vtable[3]() == 1 && [rcx+8] == 3`, and the navigation search consults it on
//! every candidate. The sequence is the entire visual difference -- **nothing in the image reads
//! `cell[+8] == 2` to decide how to draw**, so the appearance of an unavailable row is decided
//! solely by `0x7a`.
//!
//! On this layout `0x7a` does not grey a row, it removes it from the screen. That is the shipped
//! behaviour the player sees as "LOAD GAME is not there until you have a save", and it is what this
//! module changes.
//!
//! # What this does instead
//!
//! It swaps which of the two writes carries the "unavailable" meaning:
//!
//! * the row is styled as if available, so it is **drawn**;
//! * the cell state is put back to [`ds2_rva::FE_BUTTON_STATE_UNAVAILABLE`], so it is **still not
//!   selectable**.
//!
//! Both halves are the game's own values written to the game's own fields. The mod invents no
//! state and adds no notion of "available" of its own -- it reads the byte the game computed and
//! re-applies the same verdict through the other one of the two mechanisms the game already has.
//!
//! # The honest limitation
//!
//! **The row is drawn in its normal style, not a greyed one.** The sequence ids are indices into a
//! layout resource inside `GameDataEbl.bdt`, so what any given id looks like cannot be read out of
//! the executable; `0x67` is used because it is the only id this exact element is *proven* to
//! render, by the available branch of the very function being replaced. Finding a genuine
//! greyed-out sequence means decrypting the layout archive with `GameDataKeyCode.pem` and reading
//! its animation table, which is a separate piece of work and is not guessed at here. What ships is
//! "visible and inert", which is the half that can be established.
//!
//! # Why the state is re-asserted twice
//!
//! Once synchronously, inside the styling detour, so the corrected state is in place before
//! `FexGridControl`'s refresh runs and picks a cursor position -- the activate handler
//! `0x1400f4a60` does **no** enable check of its own, so a cursor that could rest on an unavailable
//! row would be able to fire its action. And once per frame from
//! [`ds2_rva::FE_TOP_MENU_UPDATE`], which is the only per-frame function specific to this menu, so
//! that anything else which rewrites the cell states cannot leave a row selectable for longer than
//! the frame it did it in.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use ds2_game_base::mem::{
    game_module_base, safe_read_f32, safe_read_i32, safe_read_u8, safe_read_u16, safe_read_usize,
};
use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;
use crate::install::log;

/// Trampoline back to the original enable-and-style pass.
static APPLY_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// Trampoline back to `FeSubStateTitleTopMenu`'s original update.
static UPDATE_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// Trampoline back to the original cell factory.
static BUILD_CELL_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// How many times the cell factory has been called.
static CELLS_BUILT: AtomicUsize = AtomicUsize::new(0);

/// Frames since the title screen started updating.
///
/// **There are no timestamps in the loader log**, and the question that matters here is entirely
/// about ordering in time: the rows are on screen at full brightness for about a second before the
/// fade takes, and every attempt to place the fade earlier has been applied and then evidently
/// overwritten. A frame number on each line turns "these things happened" into "this happened 60
/// frames before that", which is the difference between reading the log and guessing from it.
static FRAME: AtomicUsize = AtomicUsize::new(0);

/// Set once the per-frame refresh has run, so the first one is stamped.
static FIRST_REFRESH: AtomicUsize = AtomicUsize::new(0);

/// Set once the styling pass has run, so the first one is stamped.
static FIRST_STYLE: AtomicUsize = AtomicUsize::new(0);

/// Set once the sequence lengths have been reported.
static MEASURED: AtomicUsize = AtomicUsize::new(0);

/// Which rows are currently wearing the faded look, as this mod last played it.
static LOOKS_APPLIED: AtomicU32 = AtomicU32::new(0);

/// How many times a drawn row's look has disagreed with whether it can be selected.
static LOOK_VIOLATIONS: AtomicUsize = AtomicUsize::new(0);

/// How many times a row has been left half-posed -- some of its sprites following the sequence it
/// was played and others stuck where they were. That is what a blank row is made of.
static BLANK_VIOLATIONS: AtomicUsize = AtomicUsize::new(0);

/// How many times the mutually exclusive pair has been seen enabled together.
///
/// **A counter, not a flag, and checked every pass rather than once.** The failure it watches for
/// was reported as intermittent, and a one-shot probe cannot tell "never happened" from "happened
/// after I stopped looking". Every increment is logged with the frame it happened on, so a run
/// either ends with no violation line at all or with a count and a timeline.
static PAIR_VIOLATIONS: AtomicUsize = AtomicUsize::new(0);

/// The fade's span, as `f32` bits. Seeded with the measured value and replaced by the live one.
///
/// Seeded rather than left empty because the first row is faded before anything has been measured,
/// and replaced from the game rather than trusted because a constant is only true for the build it
/// was measured on.
static FADE_SPAN_BITS: AtomicU32 =
    AtomicU32::new(ds2_rva::FE_TOP_MENU_SEQUENCE_FADED_SEEK.to_bits());

/// How far into the fade to start it, so it lands on its last frame.
fn fade_span() -> f32 {
    f32::from_bits(FADE_SPAN_BITS.load(Ordering::Relaxed))
}

/// Advance the frame counter. Called from the title screen's own per-frame update.
pub(crate) fn tick_frame() {
    FRAME.fetch_add(1, Ordering::Relaxed);
}

/// The current frame number, for stamping a log line.
fn frame() -> usize {
    FRAME.load(Ordering::Relaxed)
}

/// Rows that have already been faded at construction, one bit each.
///
/// **This is the whole difference between the working version and the regression.** The factory is
/// a setup-time provider, not a one-shot builder: a measured run logged it past 256 calls, with row
/// indices running to 9 -- past the six that exist -- before it stopped. Fading on every call meant
/// replaying the fade animation from its first frame dozens of times per row, which is what was on
/// screen. Once per row is the same intent at the rate the intent actually meant.
static BORN_FADED_ROWS: AtomicU32 = AtomicU32::new(0);

/// Live module base, resolved once at install so neither detour repeats the lookup per frame.
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);

/// Bit `i` set means row `i` was computed unavailable by the game's own builder.
///
/// Written by the styling detour, which is the only place the descriptor list is in scope, and read
/// by the per-frame detour, which has no list of its own. Starts empty, so a per-frame pass that
/// runs before any styling pass does nothing at all rather than guessing.
static UNAVAILABLE_ROWS: AtomicU32 = AtomicU32::new(0);

/// Row count seen by the last styling pass, clamped to the vector's own capacity.
static ROW_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Rows the game removed from the screen and this mod did not ask to keep.
///
/// **These must never be painted.** A row the game marks unavailable and does not force-show is
/// played `0x7a` and is gone -- and it is gone on purpose, without whatever content a drawn row
/// gets. Painting the faded look over it puts an empty plate back on screen, which is exactly what
/// the blank Information row was: not a game behaviour, but this mod resurrecting a row it never
/// meant to show. The force-show allowlist governed the enable byte and never reached the code that
/// picks looks, so the two disagreed about which rows exist.
static HIDDEN_ROWS: AtomicU32 = AtomicU32::new(0);

/// How many rows have been drawn that the game would have hidden.
static SHOWN: AtomicUsize = AtomicUsize::new(0);

/// Which rows currently carry the faded look. [`UNKNOWN_LOOKS`] until the first pass.
static FADED_LOOKS: AtomicU32 = AtomicU32::new(UNKNOWN_LOOKS);

/// Sentinel for [`FADED_LOOKS`]: nothing has been styled yet, so every row needs its look applied.
const UNKNOWN_LOOKS: u32 = u32::MAX;

/// Last observed title-scene animation state, logged on change for the same reason.
static LAST_ANIMATING: AtomicBool = AtomicBool::new(false);

/// The last `unavailable` mask that was logged, plus a bit that marks "nothing logged yet".
///
/// The states MOVE while the menu is on screen -- a machine can come online, and a save can finish
/// loading, after the menu is already up -- so a once-only log reports a snapshot and calls it the
/// answer. Logging on change instead costs a handful of lines per boot and produces the timeline.
static LAST_LOGGED: AtomicU32 = AtomicU32::new(NEVER_LOGGED);

/// Sentinel for [`LAST_LOGGED`]: no six-bit mask can collide with it.
const NEVER_LOGGED: u32 = u32::MAX;

/// Column of a cell within its grid. `+0x1c`, read by the navigation search at `0x140107b40`.
const CELL_COLUMN_OFFSET: usize = 0x1c;

/// Row of a cell within its grid. `+0x20`, read by the same search.
const CELL_ROW_OFFSET: usize = 0x20;

/// `void apply_states(group, list)` -- both in registers, no stack arguments.
type ApplyStatesFn = unsafe extern "system" fn(*mut u8, *mut u8);

/// `void update(this, float delta)` -- `this` in RCX, the frame delta in XMM1.
///
/// The float is carried for the same reason every other update detour in this crate carries it:
/// the family's signature puts the delta in XMM1, and a detour that declared only `this` would be
/// free to clobber it.
type UpdateFn = unsafe extern "system" fn(*mut u8, f32);

/// `longlong cell_for_index(group, int index)` -- returns the cell whose id matches, or null.
type CellForIndexFn = unsafe extern "system" fn(*mut u8, i32) -> *mut u8;

/// `SceneObjProxy* bind(group + 0x100, scratch, descriptor)` -- `0x140026790`.
///
/// Constructs a `FrontendEx::SceneObjProxy` into the caller's scratch and returns it. The game's
/// own styling pass builds one per row per pass and lets it die on the stack, which is why doing
/// the same here is the same lifetime the game already accepts rather than a new one.
type ProxyElementFn = unsafe extern "system" fn(*mut u8) -> *mut u8;

type BindProxyFn = unsafe extern "system" fn(*mut u8, *mut u8, *mut u8) -> *mut u8;

/// `proxy* build_cell(layout, proxy_out, coords)` -- `0x1400f36b0`. `coords` is `int[2]`.
type BuildCellFn = unsafe extern "system" fn(*mut u8, *mut u8, *const i32) -> *mut u8;

/// `list* build_rows(list)` -- `0x1400f4250`, filling a caller-supplied 352-byte buffer.
type BuildRowsFn = unsafe extern "system" fn(*mut u8) -> *mut u8;

/// `void play(frame_ctrl, int sequence, int, float)` -- vtable slot 0 of the proxy's
/// `ComponentFrameCtrl` at `+0x40`. RCX/EDX/R8D/XMM3, exactly as `0x1400f5087` calls it.
type PlaySequenceFn = unsafe extern "system" fn(*mut u8, i32, i32, f32);

/// Address of a row descriptor within the list, reproducing the game's own alignment fudge.
///
/// The builder and the styling pass both compute the element base as
/// `list + ((-list) & 7) + index * 0x38`, and the descriptors are only ever at those addresses. The
/// negate is done on the full pointer and masked to three bits, which is what the game's
/// `neg rdx; and edx,0x7` does.
fn row_at(list: usize, index: usize) -> usize {
    let align = (0usize.wrapping_sub(list)) & 7;
    list + align + index * ds2_rva::FE_TOP_MENU_ROW_STRIDE
}

/// Put every unavailable row's cell back to the state that keeps it out of cursor navigation.
///
/// Returns how many rows it wrote. Cells are looked up through the game's own by-index lookup
/// rather than by walking the group, because that lookup is the one the styling pass uses and it
/// returns null for an index with no cell -- which is the case this must skip rather than write
/// through.
///
/// # Safety
///
/// `group` must be the live `FeGroupTitleTopMenu`. Each cell is written only at the offset the
/// game's own styling pass writes, and only after a fault-tolerant read of that same offset has
/// succeeded.
unsafe fn reassert_unavailable(group: *mut u8) -> usize {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let mask = UNAVAILABLE_ROWS.load(Ordering::Acquire);
    let count = ROW_COUNT.load(Ordering::Acquire);
    if base == 0 || mask == 0 || count == 0 || group.is_null() {
        return 0;
    }
    // SAFETY: resolved from the live module base; called with the group pointer and the index its
    // own call site in `FE_TOP_MENU_APPLY_STATES` passes.
    let cell_for_index: CellForIndexFn = unsafe {
        std::mem::transmute::<usize, CellForIndexFn>(
            base + ds2_rva::FE_TOP_MENU_CELL_FOR_INDEX as usize,
        )
    };
    let mut written = 0;
    for index in 0..count {
        if mask & (1 << index) == 0 {
            continue;
        }
        let cell = unsafe { cell_for_index(group, index as i32) };
        if cell.is_null() {
            continue;
        }
        unsafe { measure_sequences(cell) };
        let state = cell as usize + ds2_rva::FE_BUTTON_STATE_OFFSET;
        // Read before write: a cell the lookup handed back is mapped, and the read also rejects
        // the case where it is already correct, which is the common one on later frames.
        match unsafe { safe_read_i32(state) } {
            Some(current) if current == ds2_rva::FE_BUTTON_STATE_UNAVAILABLE => continue,
            Some(_) => {}
            None => continue,
        }
        // SAFETY: the same field the game's own pass writes at `0x1400f509a`, just read without
        // faulting, and written with the value that pass writes.
        unsafe {
            cell.add(ds2_rva::FE_BUTTON_STATE_OFFSET)
                .cast::<i32>()
                .write(ds2_rva::FE_BUTTON_STATE_UNAVAILABLE)
        };
        written += 1;
    }
    written
}

/// Which rows the game will draw on this pass: its own enable bytes, plus whatever was forced.
///
/// Pure so it can be tested without a game. `saved` holds the bytes as the builder left them, read
/// before the forcing loop wrote anything, so this reconstructs exactly what the original styling
/// pass is about to read.
fn effective_enabled(saved: &[u8], forced: u32, count: usize) -> u32 {
    let mut effective = 0u32;
    for (index, slot) in saved.iter().enumerate().take(count) {
        if *slot != 0 || forced & (1 << index) != 0 {
            effective |= 1 << index;
        }
    }
    effective
}

/// What a sprite's own table says about a sequence it was just played.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpritePose {
    /// No animation resource or no table. This sprite is never moved by any play.
    NoTable,
    /// A table, but no entry for this sequence. The play was a no-op for this sprite.
    Missing,
    /// An entry, whose start frame this is.
    At(u16),
}

/// Whether one sprite actually took the sequence it was just played.
///
/// Pure so it can be tested without a game. `entry_start` is the sprite's own table entry for the
/// sequence, or `None` when the table has no entry for it -- and `None` is a definite failure, not
/// an unknown: the play scans the table and returns without touching the sprite, so a sprite with
/// no entry provably did not move. When there is an entry, the play sets the position to
/// `start + seek` exactly (`0x140b6c54d`-`0x140b6c57b`), so this compares against that. The
/// tolerance is half a frame: the field is a `f32` built by `CVTDQ2PS` from a `u16` plus our own
/// `f32` offset, so exact equality is reasonable, but half a frame costs nothing and avoids a
/// violation report resting on the last bit of a float.
fn sprite_followed(pose: SpritePose, position: f32, seek: f32) -> bool {
    match pose {
        // A sprite with no table at all is static by construction -- no play was ever going to
        // move it, so it is not what makes a row blank. CALIBRATED, NOT ASSUMED: without this arm
        // the check reported one stuck sprite on every row for every sequence, including row 0
        // under `0x67`, which is NEW GAME rendering correctly. A detector that fires on a row known
        // to be fine reports nothing at all, so the tableless sprite had to come out of the count.
        SpritePose::NoTable => true,
        // A table that exists and does not list this sequence is the real signal: the play scanned
        // it, missed, and returned, leaving this sprite behind while its siblings moved.
        SpritePose::Missing => false,
        SpritePose::At(start) => (position - (f32::from(start) + seek)).abs() < 0.5,
    }
}

/// Look a sequence id up in one sprite's own table. `None` when absent or unreadable.
///
/// # Safety
///
/// `sprite` must be a live `FeComponentSprite`. Every read is checked.
unsafe fn sprite_pose(sprite: usize, sequence: i32) -> SpritePose {
    let Some(resource) = (unsafe { safe_read_usize(sprite + ds2_rva::FE_SPRITE_RESOURCE_OFFSET) })
    else {
        return SpritePose::NoTable;
    };
    let table = if resource == 0 {
        0
    } else {
        unsafe { safe_read_usize(resource + ds2_rva::FE_SPRITE_TABLE_OFFSET) }.unwrap_or(0)
    };
    if table == 0 {
        return SpritePose::NoTable;
    }
    let entries =
        unsafe { safe_read_usize(table + ds2_rva::FE_SPRITE_TABLE_ENTRIES_OFFSET) }.unwrap_or(0);
    let count = unsafe { safe_read_u16(table + ds2_rva::FE_SPRITE_TABLE_COUNT_OFFSET) }.unwrap_or(0)
        as usize;
    if entries == 0 || count == 0 {
        return SpritePose::NoTable;
    }
    for i in 0..count.min(64) {
        let at = entries + i * ds2_rva::FE_SPRITE_TABLE_ENTRY_STRIDE;
        if unsafe { safe_read_i32(at) } == Some(sequence) {
            return match unsafe { safe_read_u16(at + ds2_rva::FE_SPRITE_TABLE_ENTRY_START_OFFSET) }
            {
                Some(start) => SpritePose::At(start),
                None => SpritePose::Missing,
            };
        }
    }
    SpritePose::Missing
}

/// Resolve a bound proxy to the layout element it addresses, the way the game does.
///
/// # Safety
///
/// `proxy` must be a `SceneObjProxy` the binder returned. Both hops are checked reads and the call
/// is the game's own slot 0; see [`ds2_rva::FE_SCENE_OBJ_PROXY_ELEMENT_SLOT`].
unsafe fn proxy_element(proxy: *mut u8) -> usize {
    let Some(vtable) = (unsafe { safe_read_usize(proxy as usize) }) else {
        return 0;
    };
    let Some(entry) =
        (unsafe { safe_read_usize(vtable + ds2_rva::FE_SCENE_OBJ_PROXY_ELEMENT_SLOT * 8) })
    else {
        return 0;
    };
    // SAFETY: slot 0 of the proxy's own vtable, called with the proxy exactly as `0x14001e4e0`
    // does, and its result null-checked exactly as `0x14001e4e2` does.
    let resolve: ProxyElementFn = unsafe { std::mem::transmute::<usize, ProxyElementFn>(entry) };
    (unsafe { resolve(proxy) }) as usize
}

/// THE BLANK-ROW SEMAPHORE. Did every sprite under this row follow the sequence it was played?
///
/// **A row goes blank when its sprites disagree.** A play recurses through `FeComponentObject` to
/// every child, but only sprites act on it, and each looks the id up in its OWN table; a miss is a
/// silent no-op. So a row can end up half-posed -- the plate at the new sequence, the part carrying
/// the caption still parked wherever `0x7a` left it -- and that is an empty plate on screen. This
/// reads the outcome rather than predicting it: for every sprite in the tree, is its playback
/// position where the sequence says it should be?
///
/// Returns the number of sprites that did not follow.
///
/// # Safety
///
/// `element` must be the element from [`proxy_element`]. Every read is checked; nothing is written.
unsafe fn count_stuck_sprites(element: usize, sequence: i32, seek: f32, base: usize) -> usize {
    const MAX_NODES: usize = 64;
    let mut stack = [0usize; MAX_NODES];
    let mut top = 1;
    stack[0] = element;
    let mut seen = 0;
    let mut stuck = 0;
    while top > 0 && seen < MAX_NODES {
        top -= 1;
        let node = stack[top];
        seen += 1;
        let vtable = unsafe { safe_read_usize(node) }.unwrap_or(0);
        if vtable.wrapping_sub(base) == ds2_rva::FE_COMPONENT_SPRITE_VTABLE as usize {
            let pose = unsafe { sprite_pose(node, sequence) };
            let position = unsafe { safe_read_f32(node + ds2_rva::FE_SPRITE_POSITION_OFFSET) }
                .unwrap_or(f32::NAN);
            if !sprite_followed(pose, position, seek) {
                stuck += 1;
            }
        }
        for offset in [
            ds2_rva::FE_COMPONENT_SIBLING_OFFSET,
            ds2_rva::FE_COMPONENT_CHILD_OFFSET,
        ] {
            if let Some(next) = unsafe { safe_read_usize(node + offset) }
                && next != 0
                && top < MAX_NODES
            {
                stack[top] = next;
                top += 1;
            }
        }
    }
    stuck
}

/// Which drawn rows wear a look that contradicts their selectability. Zero means consistent.
///
/// Pure so it can be tested without a game. Hidden rows are excluded by `drawn` rather than by
/// checking them and forgiving them: a row that is not on screen has no look to be wrong about.
fn look_mismatches(applied_faded: u32, unselectable: u32, drawn: u32) -> u32 {
    (applied_faded ^ unselectable) & drawn
}

/// Whether a mutually exclusive pair is about to be drawn together.
///
/// Every bit in `pair` set in `effective` is the violation. Written against a mask rather than
/// hard-coded row numbers so a second exclusive pair, if the menu ever grows one, is a constant
/// change rather than a code change.
fn breaks_pair(effective: u32, pair: u32) -> bool {
    pair != 0 && effective & pair == pair
}

/// Which rows are currently not selectable, read from the cells rather than from the descriptors.
///
/// **The cell state is the signal, and it is the game's own.** A row is unselectable either because
/// its enable byte was false, or because [`ds2_rva::FE_TOP_MENU_DISABLE_ALL`] turned the whole list
/// off while the title scene is still animating in. Both end as
/// [`ds2_rva::FE_BUTTON_STATE_UNAVAILABLE`] in the field the navigation predicate reads, so one
/// read covers both and no notion of "ready yet" has to be invented here.
///
/// # Safety
///
/// `group` must be the live `FeGroupTitleTopMenu`.
unsafe fn unselectable_rows(group: *mut u8, count: usize) -> u32 {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 || group.is_null() {
        return 0;
    }
    // SAFETY: resolved from the live module base and called exactly as its own call site does.
    let cell_for_index: CellForIndexFn = unsafe {
        std::mem::transmute::<usize, CellForIndexFn>(
            base + ds2_rva::FE_TOP_MENU_CELL_FOR_INDEX as usize,
        )
    };
    // While the save-load system still has a request in flight the menu is drawn but nothing on it
    // can be used, so every row reads as unselectable regardless of its own state. This is not a
    // second mechanism bolted on: it is the same question -- "can the player act on this row" --
    // asked at the level that actually holds the answer during that window.
    // TWO WAYS TO BE UNUSABLE, and the second one is this mod's own doing. `title_settle` and the
    // forced sequence gate put the menu on screen without waiting for the title scene to finish
    // animating in, which is exactly the behaviour that was wanted -- and it means the menu is
    // drawn during a stretch where nothing on it can be acted on. Asking the ORIGINAL gate, rather
    // than the detour that always says yes, is what makes that stretch nameable.
    let animating = matches!(
        unsafe { crate::title::title_sequence_settled() },
        Some(false)
    );
    let mut mask = if animating { (1u32 << count) - 1 } else { 0 };
    if animating != LAST_ANIMATING.swap(animating, Ordering::Relaxed) {
        log(format_args!(
            "{LOG_PREFIX} title-scene f={} screen=top-menu animating={animating}",
            frame()
        ));
    }
    for index in 0..count {
        let cell = unsafe { cell_for_index(group, index as i32) };
        if cell.is_null() {
            continue;
        }
        if unsafe { safe_read_i32(cell as usize + ds2_rva::FE_BUTTON_STATE_OFFSET) }
            == Some(ds2_rva::FE_BUTTON_STATE_UNAVAILABLE)
        {
            mask |= 1 << index;
        }
    }
    mask
}

/// Give every row the look that matches whether it can be used.
///
/// Faded for unselectable, normal for the rest, and only for rows whose verdict actually changed --
/// the cursor moving rewrites cell states between `3` and `4` every frame, and re-playing a
/// sequence on that would restart the animation continuously.
///
/// # Safety
///
/// `group` and `list` must be a live pair. The proxy is built by the game's own binder into a
/// scratch of the size its own callers use, and the play call carries the four registers
/// `0x1400f5087` sets.
unsafe fn apply_looks(group: *mut u8, list: *mut u8, count: usize, unselectable: u32) {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 || group.is_null() || list.is_null() {
        return;
    }
    let previous = FADED_LOOKS.swap(unselectable, Ordering::AcqRel);
    // Every row on the first pass, only the changed ones after that.
    let changed = if previous == UNKNOWN_LOOKS {
        u32::MAX
    } else {
        previous ^ unselectable
    };
    if changed == 0 {
        return;
    }
    // SAFETY: resolved from the live module base, with the argument shapes read off `0x1400f5071`.
    let bind: BindProxyFn = unsafe {
        std::mem::transmute::<usize, BindProxyFn>(base + ds2_rva::FE_BIND_SCENE_OBJ_PROXY as usize)
    };
    let list_address = list as usize;
    let hidden = HIDDEN_ROWS.load(Ordering::Acquire);
    for index in 0..count {
        if changed & (1 << index) == 0 {
            continue;
        }
        // A row the game took off the screen stays off it. Fading is for rows that are drawn.
        if hidden & (1 << index) != 0 {
            continue;
        }
        // EVERY unselectable row that is drawn wears the faded look. Narrowing this to an
        // allowlist was tried and is what took the dimming off the whole menu: the rows are all
        // unselectable while the title scene animates in, and that is exactly the window the
        // dimming exists to show. `LOOK_VIOLATIONS` below is the guard against doing it again.
        let faded = unselectable & (1 << index) != 0;
        let sequence = if faded {
            ds2_rva::FE_TOP_MENU_SEQUENCE_FADED
        } else {
            ds2_rva::FE_TOP_MENU_SEQUENCE_AVAILABLE
        };
        // The game's own callers give this 144 bytes.
        let mut scratch = [0u64; 18];
        let descriptor = row_at(list_address, index) as *mut u8;
        let proxy = unsafe { bind(group.add(0x100), scratch.as_mut_ptr().cast(), descriptor) };
        if proxy.is_null() {
            continue;
        }
        // The fade starts at its END so a row is faded on the frame it is asked to be, rather than
        // playing the fade's own frames to get there. The offset is relative to the sequence start,
        // so its span is exactly the distance to its last frame. The un-fade is left at 0.0: a row
        // becoming usable is a change worth seeing happen.
        let seek = if sequence == ds2_rva::FE_TOP_MENU_SEQUENCE_FADED {
            fade_span()
        } else {
            0.0
        };
        unsafe { play_sequence(proxy, sequence, seek, faded) };
        // Did the whole row follow, or only part of it? Checked immediately after the play, on
        // the element the proxy itself resolves to rather than a guessed offset.
        let element = unsafe { proxy_element(proxy) };
        if element != 0 {
            let stuck = unsafe { count_stuck_sprites(element, sequence, seek, base) };
            if stuck != 0 {
                let total = BLANK_VIOLATIONS.fetch_add(1, Ordering::Relaxed) + 1;
                log(format_args!(
                    "{LOG_PREFIX} INVARIANT-VIOLATION f={} screen=top-menu row-half-posed \
                     row={index} sequence=0x{sequence:02x} seek={seek} stuck-sprites={stuck} \
                     count={total}",
                    frame()
                ));
            }
        }
        // Recorded AFTER the play, so a row whose bind or play was skipped keeps its old bit and
        // shows up as a mismatch rather than being quietly assumed correct.
        if faded {
            LOOKS_APPLIED.fetch_or(1 << index, Ordering::Relaxed);
        } else {
            LOOKS_APPLIED.fetch_and(!(1u32 << index), Ordering::Relaxed);
        }
    }

    // THE DIMMING SEMAPHORE. A drawn row's look must agree with whether it can be selected.
    //
    // The menu is drawn before it is usable -- `FE_TOP_MENU_DISABLE_ALL` puts every cell in state
    // 2 while the title scene animates -- and dimming is how that window is shown. So "unselectable
    // but wearing the bright look" is not a matter of taste, it is the mod telling the player a row
    // is ready when the game says it is not. The inverse, a selectable row left dimmed, is the same
    // lie the other way round.
    let drawn = !hidden & ((1u32 << count) - 1);
    let mismatched = look_mismatches(LOOKS_APPLIED.load(Ordering::Relaxed), unselectable, drawn);
    if mismatched != 0 {
        let total = LOOK_VIOLATIONS.fetch_add(1, Ordering::Relaxed) + 1;
        log(format_args!(
            "{LOG_PREFIX} INVARIANT-VIOLATION f={} screen=top-menu look-disagrees-with-state \
             mismatched=0b{mismatched:06b} faded=0b{:06b} unselectable=0b{unselectable:06b} \
             drawn=0b{drawn:06b} count={total}",
            frame(),
            LOOKS_APPLIED.load(Ordering::Relaxed)
        ));
    }
    if previous != unselectable {
        log(format_args!(
            "{LOG_PREFIX} looks f={} screen=top-menu unselectable=0b{unselectable:06b} \
             faded=0x{:02x} normal=0x{:02x}",
            frame(),
            ds2_rva::FE_TOP_MENU_SEQUENCE_FADED,
            ds2_rva::FE_TOP_MENU_SEQUENCE_AVAILABLE
        ));
    }
}

/// `row:(column,row)` for every cell the group currently has, for the log line.
///
/// Read-only, and every read is fault-tolerant: this runs on a drawing path and a menu that cannot
/// be described is not a menu worth crashing over. A row whose cell is absent reports `-`.
///
/// # Safety
///
/// `group` must be the live `FeGroupTitleTopMenu`.
unsafe fn describe_grid(group: *mut u8, count: usize) -> String {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 || group.is_null() {
        return String::from("unknown");
    }
    // SAFETY: resolved from the live module base and called exactly as its own call site does.
    let cell_for_index: CellForIndexFn = unsafe {
        std::mem::transmute::<usize, CellForIndexFn>(
            base + ds2_rva::FE_TOP_MENU_CELL_FOR_INDEX as usize,
        )
    };
    let mut out = String::new();
    for index in 0..count {
        if index != 0 {
            out.push(',');
        }
        let cell = unsafe { cell_for_index(group, index as i32) };
        if cell.is_null() {
            out.push_str(&format!("{index}:-"));
            continue;
        }
        let column = unsafe { safe_read_i32(cell as usize + CELL_COLUMN_OFFSET) };
        let row = unsafe { safe_read_i32(cell as usize + CELL_ROW_OFFSET) };
        match (column, row) {
            (Some(column), Some(row)) => out.push_str(&format!("{index}:({column},{row})")),
            _ => out.push_str(&format!("{index}:?")),
        }
    }
    out
}

/// Style every row as available so all six are drawn, then put the unavailable ones back to a state
/// that cannot be selected.
///
/// The enable bytes are restored before returning. The list is the caller's stack buffer and this
/// is the only function that reads it, so leaving it edited would be harmless -- it is restored
/// anyway, because a detour that hands back a buffer it was not given the right to change is a
/// detour whose blast radius depends on a fact about a caller rather than on its own code.
unsafe extern "system" fn detour_apply_states(group: *mut u8, list: *mut u8) {
    let trampoline = APPLY_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 || group.is_null() || list.is_null() {
        // Nothing safe to do without the original: this detour's whole job is to bracket it.
        return;
    }
    // SAFETY: MinHook published this trampoline for this site, and both arguments are forwarded
    // exactly as received.
    let original: ApplyStatesFn =
        unsafe { std::mem::transmute::<usize, ApplyStatesFn>(trampoline) };

    if FIRST_STYLE.swap(1, Ordering::Relaxed) == 0 {
        log(format_args!(
            "{LOG_PREFIX} first f={} screen=top-menu site=styling-pass",
            frame()
        ));
    }
    let list_address = list as usize;
    let count = unsafe { safe_read_i32(list_address + ds2_rva::FE_TOP_MENU_LIST_COUNT_OFFSET) }
        .filter(|count| *count >= 0)
        .map(|count| (count as usize).min(ds2_rva::FE_TOP_MENU_ROW_CAPACITY))
        .unwrap_or(0);
    if count == 0 {
        unsafe { original(group, list) };
        return;
    }

    // Read the game's verdict before touching anything, then force the available path onto the
    // rows this mod is allowed to draw -- and ONLY those. `unavailable` is what the game decided
    // and is reported; `forced` is what was acted on. Every row outside
    // `FE_TOP_MENU_FORCE_SHOWN_ROWS` is left exactly as the game built it, descriptor and cell
    // both, because a row that shares its screen slot with another is not one this can draw
    // without drawing two labels in one place.
    let mut unavailable = 0u32;
    let mut forced = 0u32;
    let mut saved = [1u8; ds2_rva::FE_TOP_MENU_ROW_CAPACITY];
    for (index, slot) in saved.iter_mut().enumerate().take(count) {
        let enabled = row_at(list_address, index) + ds2_rva::FE_TOP_MENU_ROW_ENABLED_OFFSET;
        let Some(value) = (unsafe { safe_read_u8(enabled) }) else {
            // A row whose own descriptor cannot be read is left exactly as the game built it.
            continue;
        };
        *slot = value;
        if value != 0 {
            continue;
        }
        unavailable |= 1 << index;
        if ds2_rva::FE_TOP_MENU_FORCE_SHOWN_ROWS & (1 << index) == 0 {
            continue;
        }
        forced |= 1 << index;
        // SAFETY: the byte was just read without faulting, and this writes the same field the
        // builder writes at `0x1400f437b`, with a value that field already takes.
        unsafe { (enabled as *mut u8).write(1) };
    }
    UNAVAILABLE_ROWS.store(forced, Ordering::Release);
    HIDDEN_ROWS.store(unavailable & !forced, Ordering::Release);
    ROW_COUNT.store(count, Ordering::Release);

    // THE SEMAPHORE. Checked on every pass, against the bytes actually being handed to the game.
    //
    // `enabled` is what the original reads to choose between drawing a row and playing `0x7a` on
    // it, so a row is on screen for this pass exactly when its byte is non-zero here -- either
    // because the game set it or because the loop above forced it. Rows 2 and 3 are mutually
    // exclusive by the builder's own arithmetic, so both being set is not a look that needs
    // judging, it is an invariant of the game's data that this mod has broken.
    let effective = effective_enabled(&saved, forced, count);
    if breaks_pair(effective, ds2_rva::FE_TOP_MENU_PAIR_MUTUALLY_EXCLUSIVE) {
        let total = PAIR_VIOLATIONS.fetch_add(1, Ordering::Relaxed) + 1;
        log(format_args!(
            "{LOG_PREFIX} INVARIANT-VIOLATION f={} screen=top-menu both-of-pair-shown \
             effective=0b{effective:06b} game-said=0b{:06b} forced=0b{forced:06b} count={total}",
            frame(),
            !unavailable & ((1u32 << count) - 1)
        ));
    }

    unsafe { original(group, list) };

    // Put the buffer back the way it was handed over.
    for (index, slot) in saved.iter().enumerate().take(count) {
        if forced & (1 << index) == 0 {
            continue;
        }
        let enabled = row_at(list_address, index) + ds2_rva::FE_TOP_MENU_ROW_ENABLED_OFFSET;
        // SAFETY: written one line above through the same address.
        unsafe { (enabled as *mut u8).write(*slot) };
    }

    // SYNCHRONOUSLY, not on the next frame: `FeGroupTitleTopMenu::v25` calls
    // `FexGridControl::FUN_140023690` immediately after this pass, and that is what settles the
    // cursor. The activate handler does no enable check of its own, so a cursor allowed to rest on
    // a row this mod made look selectable would be able to fire that row's action.
    let written = unsafe { reassert_unavailable(group) };
    let total = SHOWN.fetch_add(written, Ordering::Relaxed) + written;
    // The states are settled for this pass, so the looks can follow them straight away rather than
    // waiting a frame -- which would otherwise show one frame of a row styled as offered.
    let unselectable = unsafe { unselectable_rows(group, count) };
    unsafe { apply_looks(group, list, count, unselectable) };

    if LAST_LOGGED.swap(unavailable, Ordering::Relaxed) != unavailable {
        // The mask is the evidence for what the menu actually decided on this machine: which rows
        // the game would have hidden is the whole reason this feature exists, and a run where the
        // mask is 0 means it changed nothing and would otherwise be indistinguishable from a run
        // where the hook never fired.
        // The grid coordinates are the point of this line as much as the masks are. Two rows that
        // report the same (column,row) share a screen slot, and a slot the game only ever puts one
        // label in is one this mod must not put two in. That is a fact about the layout resource,
        // which is inside `GameDataEbl.bdt` and cannot be read statically -- but the cells carry
        // their own coordinates, so it can be measured from here instead of guessed.
        log(format_args!(
            "{LOG_PREFIX} shown screen=top-menu rows={count} unavailable=0b{unavailable:06b} \
             forced=0b{forced:06b} allowed=0b{:06b} state={} total={total} grid={}",
            ds2_rva::FE_TOP_MENU_FORCE_SHOWN_ROWS,
            ds2_rva::FE_BUTTON_STATE_UNAVAILABLE,
            unsafe { describe_grid(group, count) }
        ));
    }
}

/// Re-assert the unavailable rows once per frame, after the game's own update has run.
///
/// The scene tick that handles this menu's input runs inside the original, so writing afterwards
/// means the states navigation reads on the next frame are the corrected ones.
unsafe extern "system" fn detour_top_menu_update(this: *mut u8, delta: f32) {
    let trampoline = UPDATE_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for this site, and `delta` is forwarded
        // untouched.
        let original: UpdateFn = unsafe { std::mem::transmute::<usize, UpdateFn>(trampoline) };
        unsafe { original(this, delta) };
    }
    unsafe { refresh_looks() };
}

/// Bring the menu's looks back in line with what can actually be used, from any per-frame site.
///
/// Split out of the per-frame detour rather than inlined, so the sequence of reads from the module
/// base down to the group is stated once.
///
/// # Safety
///
/// Every hop from the module base to the group is a checked read, and the group is only used if all
/// of them succeed.
unsafe fn refresh_looks() {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return;
    }
    if FIRST_REFRESH.swap(1, Ordering::Relaxed) == 0 {
        log(format_args!(
            "{LOG_PREFIX} first f={} screen=top-menu site=substate-update",
            frame()
        ));
    }
    let Some(globals) = (unsafe { safe_read_usize(base + ds2_rva::FE_TITLE_GLOBALS as usize) })
    else {
        return;
    };
    let Some(scene) = (unsafe { safe_read_usize(globals + ds2_rva::FE_TITLE_SCENE_OFFSET) }) else {
        return;
    };
    if scene == 0 {
        return;
    }
    let Some(group) = (unsafe { safe_read_usize(scene + ds2_rva::FE_TOP_MENU_GROUP_OFFSET) })
    else {
        return;
    };
    if group == 0 {
        return;
    }
    // SAFETY: the group pointer came from the field the scene's own builder writes at
    // `0x1400f4950`, through fault-tolerant reads that all succeeded.
    let group = group as *mut u8;
    let written = unsafe { reassert_unavailable(group) };
    if written != 0 {
        SHOWN.fetch_add(written, Ordering::Relaxed);
    }
    let count = ROW_COUNT.load(Ordering::Acquire);
    if count == 0 {
        return;
    }
    let unselectable = unsafe { unselectable_rows(group, count) };
    if unselectable == FADED_LOOKS.load(Ordering::Acquire) {
        return;
    }
    let mut list = [0u64; 44];
    // SAFETY: resolved from the live module base and given a 352-byte 8-aligned buffer, which is
    // what all four of its own callers pass.
    let build: BuildRowsFn = unsafe {
        std::mem::transmute::<usize, BuildRowsFn>(base + ds2_rva::FE_TOP_MENU_BUILD_ROWS as usize)
    };
    let list = unsafe { build(list.as_mut_ptr().cast()) };
    if list.is_null() {
        return;
    }
    unsafe { apply_looks(group, list, count, unselectable) };
}

/// `float bracket(element, int sequence)` -- the element's slots `0x58` and `0x60`.
///
/// `FeObjectButtonEx` calls both with a sequence id and divides one span by another to convert a
/// position in one sequence into a position in another, which is what identifies them as the start
/// and end of a sequence in whatever clock the play offset is measured in.
type SequenceBracketFn = unsafe extern "system" fn(*mut u8, i32) -> f32;

/// `int current_sequence(element)` -- the element's slot `0x50`, compared against `0x85` at
/// `0x14010ada1`.
type CurrentSequenceFn = unsafe extern "system" fn(*mut u8) -> i32;

/// MEASUREMENT ONLY. Report how long the fade sequence actually is.
///
/// `1000.0` as a play offset produced a row that never dimmed, so the offset is real and a value
/// past the end leaves the sequence with nothing to play. The end is therefore somewhere between
/// `0.0` and `1000.0`, and this asks the element rather than bisecting it in front of the person
/// watching the screen.
///
/// # Safety
///
/// `cell` must be a live `FeObjectButtonEx`. Both getters are called with the argument shape their
/// own call sites use, and only after the element pointer has been read without faulting.
unsafe fn measure_sequences(cell: *mut u8) {
    // PROVENANCE WARNING: `cell+0x40`/`cell+0x30` is NOT this row's element -- all six rows resolve
    // to one shared object through it. The only verified route is `proxy_element`, slot 0 of the
    // bound proxy. The span this reports (`0x6c` 89..103) happens to agree with
    // `FE_TOP_MENU_SEQUENCE_FADED_SEEK`, which is why it never looked wrong, but it is measuring
    // something other than what its name claims and must not be trusted for per-row facts.
    if MEASURED.swap(1, Ordering::Relaxed) != 0 {
        return;
    }
    // `FeObjectButtonEx`'s own accessors: slot 21 returns `[this+0x40]`, and slot 19 returns
    // `[this+0x30]` when that is null.
    let element = (unsafe { safe_read_usize(cell as usize + 0x40) })
        .filter(|element| *element != 0)
        .or_else(|| unsafe { safe_read_usize(cell as usize + 0x30) })
        .filter(|element| *element != 0);
    let Some(element) = element else {
        log(format_args!(
            "{LOG_PREFIX} measure screen=top-menu element=none"
        ));
        return;
    };
    let element = element as *mut u8;
    let Some(vtable) = (unsafe { safe_read_usize(element as usize) }) else {
        return;
    };
    // THE VTABLE ADDRESS IS THE POINT OF THIS LINE. The element's class cannot be pinned down
    // statically -- every route to it runs through a pointer chain that only exists at runtime --
    // so its slots cannot be read out of the image without first learning which vtable to read.
    // Printing the live address, minus the module base, gives an RVA that can then be walked in the
    // static image like any other.
    let base = MODULE_BASE.load(Ordering::Acquire);
    log(format_args!(
        "{LOG_PREFIX} element f={} screen=top-menu vtable=0x{vtable:016x} rva=0x{:08x}",
        frame(),
        vtable.wrapping_sub(base)
    ));
    // WALK THE CHILD CHAIN. `FeComponentObject`'s play does nothing but forward the same three
    // arguments to each child's own slot 0xc0, and each of its getters forwards the same way, so
    // the class that actually moves an animation is somewhere below this one. Its vtable cannot be
    // named statically -- the chain is pointers -- but one RVA per hop is enough to read the rest
    // out of the image afterwards.
    let mut node = unsafe { safe_read_usize(element as usize + 0x38) }.unwrap_or(0);
    for depth in 0..6 {
        if node == 0 {
            break;
        }
        let node_vtable = unsafe { safe_read_usize(node) }.unwrap_or(0);
        let next = unsafe { safe_read_usize(node + 0x28) }.unwrap_or(0);
        let child = unsafe { safe_read_usize(node + 0x38) }.unwrap_or(0);
        log(format_args!(
            "{LOG_PREFIX} chain f={} screen=top-menu depth={depth} vtable-rva=0x{:08x} \
             same-as-parent={} next={} child={}",
            frame(),
            node_vtable.wrapping_sub(base),
            node_vtable == vtable,
            next != 0,
            child != 0
        ));
        // Down before across: the leaf that implements playback is at the bottom of the child
        // links, and following siblings first would walk the widest part of the tree instead.
        node = if child != 0 { child } else { next };
    }
    let Some(start_entry) = (unsafe { safe_read_usize(vtable + 0x58) }) else {
        return;
    };
    let Some(end_entry) = (unsafe { safe_read_usize(vtable + 0x60) }) else {
        return;
    };
    let Some(current_entry) = (unsafe { safe_read_usize(vtable + 0x50) }) else {
        return;
    };
    // SAFETY: three slots of the element's own vtable, each called exactly as `0x14010ada1`
    // onwards calls it.
    let start: SequenceBracketFn =
        unsafe { std::mem::transmute::<usize, SequenceBracketFn>(start_entry) };
    let end: SequenceBracketFn =
        unsafe { std::mem::transmute::<usize, SequenceBracketFn>(end_entry) };
    let current: CurrentSequenceFn =
        unsafe { std::mem::transmute::<usize, CurrentSequenceFn>(current_entry) };
    for sequence in [
        ds2_rva::FE_TOP_MENU_SEQUENCE_FADED,
        ds2_rva::FE_TOP_MENU_SEQUENCE_AVAILABLE,
    ] {
        let from = unsafe { start(element, sequence) };
        let to = unsafe { end(element, sequence) };
        if sequence == ds2_rva::FE_TOP_MENU_SEQUENCE_FADED && to > from {
            FADE_SPAN_BITS.store((to - from).to_bits(), Ordering::Relaxed);
        }
        log(format_args!(
            "{LOG_PREFIX} measure f={} screen=top-menu sequence=0x{sequence:02x} \
             start={from:.4} end={to:.4} span={:.4} playing=0x{:02x}",
            frame(),
            to - from,
            unsafe { current(element) }
        ));
    }
}

/// Play a sequence on a bound proxy's frame control.
///
/// # Safety
///
/// `proxy` must be a `FrontendEx::SceneObjProxy` the game's binder returned. Both vtable hops are
/// checked reads, and the call carries the four registers `0x1400f5087` sets.
unsafe fn play_sequence(proxy: *mut u8, sequence: i32, seek: f32, hold: bool) {
    let frame_ctrl = unsafe { proxy.add(ds2_rva::FE_SCENE_OBJ_PROXY_FRAME_CTRL) };
    let Some(vtable) = (unsafe { safe_read_usize(frame_ctrl as usize) }) else {
        return;
    };
    let Some(entry) = (unsafe { safe_read_usize(vtable) }) else {
        return;
    };
    // SAFETY: slot 0 of the frame control's own vtable, read out of a live object.
    let play: PlaySequenceFn = unsafe { std::mem::transmute::<usize, PlaySequenceFn>(entry) };
    // THE THIRD ARGUMENT IS "KEEP PLAYING", INVERTED, AND IT IS WHY A FADED ROW WENT BLANK.
    //
    // The play ends `TEST DIL,DIL; SETZ DL; CALL [RAX+0xb0]` (`0x140b6c572`), and slot `0xb0`
    // (`0x140b6a880`) is `[this+0x10] = DL`. The sprite's own per-frame update at `0x140b6c6c0`
    // opens by reading that byte and, when it is set, advancing the position by `dt * speed`. So
    // passing `0` -- which is what the game passes, and what this passed for every sequence --
    // means "seek here and then carry on playing".
    //
    // For the available look that is right: the row should animate in. For the faded look it is
    // fatal. `0x6c` occupies frames 89..103 of a shared timeline whose next marker is `0x7a` at
    // 104, the removal -- so a row seeked into the faded segment and left playing walks out of it
    // and removes itself. Every value ever tried produced that: `0.0` faded over about a second
    // then vanished, `14.0` vanished at once, and anything larger landed past the end and never
    // drew at all. Holding the frame is what makes a pose a look instead of a departure.
    let keep_playing = i32::from(!hold);
    unsafe { play(frame_ctrl, sequence, keep_playing, seek) };
}

/// Build the cell as the game does, then fade it once, before anything draws it.
///
/// **This is the only point early enough to have no window before it.** Every other pass -- the
/// styling pass, the substate updates -- runs after the rows are already on screen, which is what
/// the second of full visibility was.
///
/// Faded without asking the CELL anything, because at construction the cell's state field has not
/// been written and every pass that decides what a row can do runs later. Starting faded and
/// letting those passes lift it is the ordering with no gap in it. And faded ONCE per row -- see
/// [`BORN_FADED_ROWS`] for why that qualifier is the difference between this working and being a
/// regression.
///
/// **But the allowlist IS answerable here**, and this used to paint all six rows because that was
/// missed. A row outside [`FE_TOP_MENU_FORCE_SHOWN_ROWS`] is one the game is about to take off the
/// screen with `0x7a`, and fading it first stamps a plate into a slot that is not free: rows 2 and
/// 3 -- INFORMATION and GO ONLINE -- are authored at the same screen position, and their enable
/// bytes are `online` and `!online`, so exactly one of the pair is live at every instant and the
/// slot is never empty on its own. Painting the dead one covers the live one until the styling
/// pass removes it. Painting it also draws it WRONG: play forwards down the element tree and only
/// `FeComponentSprite` nodes answer it (`FeComponentBase`'s slot `0xc0`, `0x140b6a970`, is
/// `return;`), each looking `0x6c` up in its own table and silently ignoring a miss, so a row
/// whose caption sprite has no `0x6c` entry comes up as an empty plate.
unsafe extern "system" fn detour_build_cell(
    layout: *mut u8,
    proxy_out: *mut u8,
    coords: *const i32,
) -> *mut u8 {
    let trampoline = BUILD_CELL_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        return proxy_out;
    }
    // SAFETY: MinHook published this trampoline for this site; all three arguments are forwarded
    // exactly as received, and the original's return value is passed straight back.
    let original: BuildCellFn = unsafe { std::mem::transmute::<usize, BuildCellFn>(trampoline) };
    let proxy = unsafe { original(layout, proxy_out, coords) };
    CELLS_BUILT.fetch_add(1, Ordering::Relaxed);
    if proxy.is_null() || coords.is_null() {
        return proxy;
    }
    // Out-of-range coordinates get an empty cell from `0x140027980` rather than a bound proxy, and
    // an empty cell has no layout element to style. The measured run saw row indices up to 9.
    let Some(column) = (unsafe { safe_read_i32(coords as usize) }) else {
        return proxy;
    };
    let Some(row) = (unsafe { safe_read_i32(coords as usize + 4) }) else {
        return proxy;
    };
    if column != 0 || row < 0 || row as usize >= ds2_rva::FE_TOP_MENU_ROW_CAPACITY {
        return proxy;
    }
    let bit = 1u32 << row;
    // Only the rows this mod is allowed to draw. Everything else is the game's to place, and at
    // this point it is about to remove it.
    if ds2_rva::FE_TOP_MENU_FORCE_SHOWN_ROWS & bit == 0 {
        return proxy;
    }
    if BORN_FADED_ROWS.fetch_or(bit, Ordering::Relaxed) & bit != 0 {
        return proxy;
    }
    unsafe {
        play_sequence(
            proxy,
            ds2_rva::FE_TOP_MENU_SEQUENCE_FADED,
            fade_span(),
            true,
        )
    };
    log(format_args!(
        "{LOG_PREFIX} born-faded f={} screen=top-menu row={row} sequence=0x{:02x} calls={}",
        frame(),
        ds2_rva::FE_TOP_MENU_SEQUENCE_FADED,
        CELLS_BUILT.load(Ordering::Relaxed)
    ));
    proxy
}

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Unavailable rows are now drawn rather than hidden.
    pub show_unavailable: bool,
}

/// Install the top-menu hooks.
///
/// Both sites go in together or not at all: the per-frame re-assert exists only to protect the
/// invariant the styling detour breaks, so installing one without the other would either change
/// nothing or leave a selectable row that should not be.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Must run after `neuter_arxan` and before the
/// title flow reaches the top menu.
pub unsafe fn install() -> Outcome {
    let mut outcome = Outcome {
        show_unavailable: false,
    };
    let base = match game_module_base() {
        Ok(base) => base,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} menu-install-failed stage=module-base error={error}"
            ));
            return outcome;
        }
    };
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} menu-install-failed stage=MH_Initialize status={status:?}"
        ));
        return outcome;
    }
    // Published before either site is patched: a detour that fired with a zero here could not
    // resolve the cell lookup, and would leave rows it had just made visible still selectable.
    MODULE_BASE.store(base, Ordering::Release);

    let apply_site = base + ds2_rva::FE_TOP_MENU_APPLY_STATES as usize;
    let apply_hook = match unsafe {
        MhHook::new(
            apply_site as *mut c_void,
            detour_apply_states as *mut c_void,
        )
    } {
        Ok(hook) => {
            // Published BEFORE the site is patched: this detour brackets the original and does
            // nothing at all without it, so a zero would mean the menu drew no styles whatsoever.
            APPLY_TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
            let status = unsafe { MH_EnableHook(apply_site as *mut c_void) };
            if status == MH_STATUS::MH_OK {
                true
            } else {
                log(format_args!(
                    "{LOG_PREFIX} hook-failed gate=top-menu-styles va=0x{apply_site:016x} \
                     stage=MH_EnableHook status={status:?}"
                ));
                false
            }
        }
        Err(status) => {
            log(format_args!(
                "{LOG_PREFIX} hook-failed gate=top-menu-styles va=0x{apply_site:016x} \
                 stage=MH_CreateHook status={status:?}"
            ));
            false
        }
    };
    if !apply_hook {
        return outcome;
    }

    let cell_site = base + ds2_rva::FE_TOP_MENU_BUILD_CELL as usize;
    match unsafe { MhHook::new(cell_site as *mut c_void, detour_build_cell as *mut c_void) } {
        Ok(hook) => {
            // Published BEFORE the site is patched: this detour returns the ORIGINAL's proxy, so a
            // zero here would mean handing back an unbound buffer as if it were a built cell.
            BUILD_CELL_TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
            let status = unsafe { MH_EnableHook(cell_site as *mut c_void) };
            if status != MH_STATUS::MH_OK {
                log(format_args!(
                    "{LOG_PREFIX} hook-failed gate=top-menu-cell va=0x{cell_site:016x} \
                     stage=MH_EnableHook status={status:?}"
                ));
            }
        }
        Err(status) => log(format_args!(
            "{LOG_PREFIX} hook-failed gate=top-menu-cell va=0x{cell_site:016x} \
             stage=MH_CreateHook status={status:?}"
        )),
    }

    let update_site = base + ds2_rva::FE_TOP_MENU_UPDATE as usize;
    match unsafe {
        MhHook::new(
            update_site as *mut c_void,
            detour_top_menu_update as *mut c_void,
        )
    } {
        Ok(hook) => {
            UPDATE_TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
            let status = unsafe { MH_EnableHook(update_site as *mut c_void) };
            if status == MH_STATUS::MH_OK {
                outcome.show_unavailable = true;
                log(format_args!(
                    "{LOG_PREFIX} hooked gate=top-menu rva=0x{:08x} va=0x{apply_site:016x} \
                     update-rva=0x{:08x} va=0x{update_site:016x}",
                    ds2_rva::FE_TOP_MENU_APPLY_STATES,
                    ds2_rva::FE_TOP_MENU_UPDATE
                ));
            } else {
                log(format_args!(
                    "{LOG_PREFIX} hook-failed gate=top-menu-update va=0x{update_site:016x} \
                     stage=MH_EnableHook status={status:?}"
                ));
            }
        }
        Err(status) => log(format_args!(
            "{LOG_PREFIX} hook-failed gate=top-menu-update va=0x{update_site:016x} \
             stage=MH_CreateHook status={status:?}"
        )),
    }

    outcome
}

#[cfg(test)]
mod tests {
    use super::{SpritePose, breaks_pair, effective_enabled, look_mismatches, sprite_followed};

    /// The pair mask this module defends, spelled out so the test fails if the constant moves.
    const PAIR: u32 = (1 << 2) | (1 << 3);

    #[test]
    fn effective_enabled_is_the_game_s_bytes_or_ours() {
        // Row 1 hidden by the game and forced by us; rows 0 and 4 the game already offers.
        let saved = [1u8, 0, 0, 1, 1, 1];
        assert_eq!(
            effective_enabled(&saved, 1 << 1, 6),
            0b111011,
            "a forced row counts as drawn even though the game said no"
        );
        assert_eq!(
            effective_enabled(&saved, 0, 6),
            0b111001,
            "with nothing forced this is exactly what the builder decided"
        );
    }

    #[test]
    fn effective_enabled_ignores_rows_past_the_count() {
        let saved = [1u8, 1, 1, 1, 1, 1];
        assert_eq!(effective_enabled(&saved, 0, 2), 0b000011);
    }

    /// THE POSITIVE CONTROL. This is the regression the semaphore exists for: forcing either half
    /// of the exclusive pair while the game has the other half enabled.
    #[test]
    fn forcing_either_half_of_the_pair_is_a_violation() {
        // Offline: the game enables row 3 and hides row 2. Forcing row 2 draws both.
        let offline = [1u8, 1, 0, 1, 1, 1];
        assert!(breaks_pair(effective_enabled(&offline, 1 << 2, 6), PAIR));
        // Online: the mirror image, and forcing row 3 is the same defect.
        let online = [1u8, 1, 1, 0, 1, 1];
        assert!(breaks_pair(effective_enabled(&online, 1 << 3, 6), PAIR));
    }

    /// THE NEGATIVE CONTROL. The shipped mask forces row 1 only, which must never trip it.
    #[test]
    fn the_shipped_force_mask_never_breaks_the_pair() {
        for saved in [[1u8, 0, 0, 1, 1, 1], [1, 0, 1, 0, 1, 1], [1, 1, 0, 1, 1, 1]] {
            let effective = effective_enabled(&saved, ds2_rva::FE_TOP_MENU_FORCE_SHOWN_ROWS, 6);
            assert!(
                !breaks_pair(effective, PAIR),
                "row 1 forcing must not touch the pair, saved={saved:?}"
            );
        }
    }

    #[test]
    fn the_game_s_own_bytes_never_break_the_pair() {
        // The builder computes row 3 as `row2 == 0`, so these two are the only shapes it produces.
        for saved in [[1u8, 1, 1, 0, 1, 1], [1, 1, 0, 1, 1, 1]] {
            assert!(!breaks_pair(effective_enabled(&saved, 0, 6), PAIR));
        }
    }

    #[test]
    fn one_half_alone_is_not_a_violation() {
        assert!(!breaks_pair(0b000100, PAIR));
        assert!(!breaks_pair(0b001000, PAIR));
        assert!(!breaks_pair(0, PAIR));
    }

    /// THE POSITIVE CONTROL for the dimming. This is the regression that prompted it: narrowing
    /// the faded look to an allowlist while every row is still unselectable, which is exactly the
    /// window the dimming exists to show.
    #[test]
    fn bright_rows_while_the_menu_is_unusable_are_a_violation() {
        let all_unselectable = 0b111111;
        let drawn = 0b111111;
        // What the allowlist produced: only row 1 faded, the other five bright.
        assert_eq!(
            look_mismatches(1 << 1, all_unselectable, drawn),
            0b111101,
            "every row that came up bright while unselectable must be named"
        );
        // The inverse lie: a selectable row left dimmed.
        assert_eq!(look_mismatches(0b000010, 0b000000, drawn), 0b000010);
    }

    /// THE NEGATIVE CONTROL. Dimming exactly the unselectable drawn rows is silent.
    #[test]
    fn looks_that_track_selectability_are_silent() {
        assert_eq!(look_mismatches(0b111111, 0b111111, 0b111111), 0);
        assert_eq!(look_mismatches(0b000010, 0b000010, 0b111111), 0);
        assert_eq!(look_mismatches(0, 0, 0b111111), 0);
    }

    /// A hidden row is off screen, so whatever bit it left behind is not a violation.
    #[test]
    fn hidden_rows_are_not_judged() {
        // Row 2 hidden: its stale faded bit and its unselectable bit both fall outside `drawn`.
        assert_eq!(look_mismatches(1 << 2, 0, 0b111011), 0);
        // But the same disagreement on a drawn row is reported.
        assert_eq!(look_mismatches(1 << 2, 0, 0b111111), 1 << 2);
    }

    /// THE POSITIVE CONTROL for blankness. A sprite whose table has no entry for the sequence is
    /// the blank-row mechanism itself: the play scans, misses, and returns without touching it.
    #[test]
    fn a_sprite_without_the_sequence_never_followed() {
        assert!(!sprite_followed(SpritePose::Missing, 0.0, 0.0));
        assert!(!sprite_followed(SpritePose::Missing, 103.0, 14.0));
    }

    /// The other half: an entry exists but the sprite is not where the play would have put it,
    /// so something else moved it or the play never reached it.
    #[test]
    fn a_sprite_left_behind_never_followed() {
        // `0x6c` starts at frame 89; played with seek 14 the sprite must land on 103.
        assert!(
            !sprite_followed(SpritePose::At(89), 104.0, 14.0),
            "parked by 0x7a at 104"
        );
        assert!(
            !sprite_followed(SpritePose::At(89), 1.0, 14.0),
            "still at the timeline start"
        );
    }

    /// THE NEGATIVE CONTROL. A sprite sitting exactly where `start + seek` puts it is fine.
    #[test]
    fn a_sprite_at_the_played_position_followed() {
        assert!(
            sprite_followed(SpritePose::At(89), 103.0, 14.0),
            "0x6c faded, seeked to its last frame"
        );
        assert!(
            sprite_followed(SpritePose::At(1), 1.0, 0.0),
            "0x67 played from its start"
        );
        assert!(
            sprite_followed(SpritePose::At(104), 104.0, 0.0),
            "0x7a, the removal"
        );
        // Half a frame of float slop is tolerated; a whole frame is not.
        assert!(sprite_followed(SpritePose::At(89), 103.4, 14.0));
        assert!(!sprite_followed(SpritePose::At(89), 104.0, 14.0));
    }

    /// An unreadable position must not read as success.
    /// CALIBRATION. Every row carries a sprite with no table, so counting it made the detector
    /// fire on rows that render correctly -- including NEW GAME, which is never hidden.
    #[test]
    fn a_tableless_sprite_is_not_counted_as_stuck() {
        assert!(sprite_followed(SpritePose::NoTable, 0.0, 0.0));
        assert!(sprite_followed(SpritePose::NoTable, f32::NAN, 14.0));
    }

    #[test]
    fn a_nan_position_never_followed() {
        assert!(!sprite_followed(SpritePose::At(89), f32::NAN, 14.0));
    }
}
