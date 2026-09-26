//! Does the player meet this weapon's requirements, and the one element that says so.
//!
//! # The comparison is the game's, not this crate's
//!
//! `FUN_1400bcde0` ([`ds2_rva::FE_STAT_ROW_COLOUR`]) is what turns a requirement number red in the
//! item detail pane, and stripped of its presentation it is three lookups and a compare:
//!
//! ```text
//! index    = FE_STAT_ROW_TABLE[key].stat_index        i16, negative on every non-requirement row
//! have     = playerStats[index]                       i32, stride 0x18, off GameManagerImp
//! required = the item param row's column for `key`
//! unmet    = required > have                          comiss / jbe, so strictly greater
//! ```
//!
//! Every one of those is performed here through the game's own functions -- the stat-index table is
//! read at runtime rather than transcribed, the column comes out of
//! [`ds2_rva::FE_ITEM_PARAM_COLUMN`], and the stat table is the one the game maintains.
//!
//! # Which of the game's two checks this is, and why that one
//!
//! There are two, they are separate implementations, and they disagree. The mechanics check is
//! `FUN_14034d3c0` (RVA `0x0034d3c0`): a continuous deficiency `sum(max(0, 1 - stat/required))`
//! feeding the damage penalty, reading `WeaponParam` directly and the effective stat block at
//! `chrStatus + 0x16`, and halving the Strength requirement for a two-handed grip. This crate uses
//! the presentation check instead, deliberately, because the badge sits on an item in a list: the
//! item is not being held, so there is no grip to ask about, and the number the badge is agreeing
//! with is the one in the detail pane a player opens to find out why.
//!
//! # What that leaves unproven
//!
//! Whether the table at [`ds2_rva::FRONTEND_ROOT_PLAYER_STATS_OFFSET`] holds base or modified
//! stats. Every cross-reference to `FUN_1404ffb20` is a reader and its writer has not been found,
//! so whether rings and spEffects are in these numbers is open -- on the gameplay side the
//! equivalent block is provably `clamp(base + modifiers, 1, 99)`, and this one has no such proof.
//! A badge computed from base stats would light up on a weapon the player can actually swing while
//! wearing a Ring of Blades.
//!
//! # Two-handing, which the detail pane ignores and this badge does not
//!
//! DS2 does not scale Strength: the mechanics check halves the Strength requirement with
//! `shr cx,1` for grip states `2` and `3`, and the `1.5x` belongs to power stance
//! (`FUN_140350170`, grips `4`, `5`, `6`). The presentation check takes no grip argument, so the
//! detail pane still prints the full requirement in red. This badge reads the player's live grip
//! ([`ds2_rva::EQUIP_GRIP_OFFSET`]) and, while two-handing, applies the same halving before the
//! compare -- the X answers "could I swing this the way I am holding my weapon now".
//!
//! # There is a cached answer, and it is the wrong shape for this
//!
//! `FUN_14034a980` caches the mechanics check as a `float` per weapon slot at
//! `equipObj + 0x50 + n * 0x48 + 0x38`, where `0.0` means every requirement is met. It covers the
//! eight records of the equip object -- six equipped slots and the two held weapons -- and this
//! badge is drawn on inventory rows too, most of which are not in any of them. Reading the cache
//! would therefore answer for the equipment screen and go blank for the list, so the check is
//! recomputed per cell on both instead. That is a consequence of where the badge lives rather than
//! an oversight, and it is also why one `unmet` serves two binds.
//!
//! # Weapons, shields, armour and spells
//!
//! The item type at the entry's `+0x1e` picks the requirement columns: `0`/`1` weapons and shields
//! (`0x33..0x36`, with the two-handed Strength rule), `2..=5` armour (`0x11..0x14`), `9` spells
//! (`0x42`, `0x43`). Rings (`7`) have no stat requirement and are never marked. The infusion
//! container the badge lives in is built for every item cell, not only weapons -- both binds run
//! their infusion loop with no type check; the `+0x1e <= 1` test only picks which infusion glyph
//! shows.

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::LOG_PREFIX;
use crate::install::{log, module_base};

/// The game's own cell bind, published by MinHook before the site is patched.
pub(crate) static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// The equipment screen's own, published the same way. Separate because it is a separate site
/// with a separate signature, and because it may refuse without taking the inventory's down.
pub(crate) static EQUIP_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

type CellBindFn = unsafe extern "system" fn(*mut u8, *const u8, u8);
type EquipBindFn = unsafe extern "system" fn(*mut u8, *const u8);
type ResolveFn = unsafe extern "system" fn(*mut u8, *mut u8, *const u8) -> *mut u8;
type SetVisibleFn = unsafe extern "system" fn(*mut u8, u8);
type DescriptorFn = unsafe extern "system" fn(*const u8, *mut u8) -> *mut u8;
type ParamRowsFn = unsafe extern "system" fn(usize, *mut u8, *const u8) -> u8;
type ParamColumnFn = unsafe extern "system" fn(usize, u32) -> u64;
type EntryLookupFn = unsafe extern "system" fn(usize, u16) -> usize;

/// How many "could not ask" decisions (`None`) to write to the log before going quiet.
///
/// The bind runs per visible row per refresh, so an uncapped line here is a log that fills a disk
/// while the player scrolls. A handful is enough to show the check is being reached.
const LOGGED_DECISIONS: usize = 8;

/// How many real answers (`Some`) to write before going quiet. Kept separate from the `None` cap
/// because the equipment screen binds every slot, empty ones first: with one shared cap, a run on
/// 2026-09-26 spent all of its lines on empty slots and never showed an armour answer.
const LOGGED_ANSWERS: usize = 48;

static DECISIONS: AtomicUsize = AtomicUsize::new(0);
static ANSWERS: AtomicUsize = AtomicUsize::new(0);
/// The item type the last [`unmet`] read, for the decision line; [`NO_KIND`] when it stopped
/// before reading one.
static LAST_KIND: AtomicUsize = AtomicUsize::new(NO_KIND);
const NO_KIND: usize = usize::MAX;
static MARKED: AtomicUsize = AtomicUsize::new(0);

/// A one-component element id path, in the shape [`ds2_rva::FE_ELEMENT_RESOLVE`] reads.
///
/// Aligned to 4 so the game's own `elements = base + (-base & 3)` idiom resolves to `base`; the
/// code below computes that offset anyway rather than relying on the alignment, because the idiom
/// is what the game does and a struct attribute is easy to lose in an edit.
#[repr(C, align(16))]
struct IdPath([u8; ds2_rva::FE_ELEMENT_PATH_SIZE]);

/// Scratch for one element accessor. The game builds these on its own stack and never destroys
/// them, which is what makes a zeroed buffer here sufficient.
#[repr(C, align(16))]
struct Accessor([u8; ds2_rva::FE_ELEMENT_ACCESSOR_SIZE]);

/// Read a pointer-sized field.
///
/// # Safety
///
/// `at` must be a live, readable address.
unsafe fn read_usize(at: usize) -> usize {
    // SAFETY: the caller guarantees the address is live.
    unsafe { (at as *const usize).read_unaligned() }
}

/// Follow a chain of pointer fields, stopping at the first null.
///
/// # Safety
///
/// `root` must be a live address and every offset must be inside the object it is applied to.
unsafe fn follow(root: usize, offsets: &[usize]) -> Option<usize> {
    let mut at = root;
    for offset in offsets {
        if at < 0x1_0000 {
            return None;
        }
        // SAFETY: the caller guarantees each step lands inside a live object.
        at = unsafe { read_usize(at + offset) };
    }
    (at >= 0x1_0000).then_some(at)
}

/// The local player's grip state, `None` wherever the chain is not built yet.
///
/// # Safety
///
/// `manager` must be the live `GameManagerImp`.
unsafe fn grip(manager: usize) -> Option<i32> {
    // SAFETY: the hops are the two vtable slots `FUN_14034f470` calls before it reads and writes
    // the grip, each a single field load; `follow` stops at the first null.
    let equip = unsafe {
        follow(
            manager,
            &[
                ds2_rva::PLAYER_CTRL_OFFSET,
                ds2_rva::PLAYER_CTRL_CHR_ASM_CTRL_OFFSET,
                ds2_rva::CHR_ASM_CTRL_EQUIP_OFFSET,
            ],
        )
    }?;
    // SAFETY: the equip object is live and the grip is the `i32` at `+0x10`.
    Some(unsafe { ((equip + ds2_rva::EQUIP_GRIP_OFFSET) as *const i32).read_unaligned() })
}

/// Whether the player fails any of this weapon's four stat requirements.
///
/// `None` means the question could not be asked -- no item under the cursor, no inventory entry,
/// no param row -- which is not the same answer as "met" and is why it is not folded into `false`.
///
/// # Safety
///
/// `item` must be the live `FeItemData` the game passed to the cell bind, and `base` the loaded
/// module base.
unsafe fn unmet(base: usize, item: *const u8) -> Option<bool> {
    if (item as usize) < 0x1_0000 {
        return None;
    }
    // SAFETY: a `FeItemData` is at least six bytes and the handle is the `u16` at `+2`.
    let handle = unsafe {
        item.add(ds2_rva::FE_ITEM_DATA_HANDLE_OFFSET)
            .cast::<u16>()
            .read_unaligned()
    };
    if handle == ds2_rva::FE_ITEM_DATA_NO_HANDLE {
        return None;
    }

    // SAFETY: `GAME_MANAGER_IMP` is a global pointer in the loaded image; the two offsets after it
    // are the walk `0x140034e83`..`0x140034e9a` performs before every entry lookup.
    let manager = unsafe { read_usize(base + ds2_rva::GAME_MANAGER_IMP as usize) };
    if manager < 0x1_0000 {
        return None;
    }
    // SAFETY: as above.
    let inventory = unsafe {
        follow(
            manager,
            &[
                ds2_rva::GAME_DATA_MANAGER_OFFSET,
                ds2_rva::GAME_DATA_MANAGER_ITEM_INVENTORY_OFFSET,
            ],
        )
    }?;

    // SAFETY: every pointer here is one the game handed this detour, or is derived from it by an
    // offset this crate validated before installing. The callee's own contract asks for exactly
    // that live object, and reads inside it go through the fault-tolerant readers.
    let lookup: EntryLookupFn = unsafe {
        std::mem::transmute::<usize, EntryLookupFn>(
            base + ds2_rva::ITEM_INVENTORY_ENTRY_LOOKUP as usize,
        )
    };
    // SAFETY: the two arguments are exactly what the game passes -- the inventory it just walked
    // to and the handle off the item -- and the function is a plain integer lookup.
    let entry = unsafe { lookup(inventory, handle) };
    if entry < 0x1_0000 {
        return None;
    }
    // SAFETY: the lookup returned an entry, and the type is the `u8` at `+0x1e`.
    let kind = unsafe {
        (entry as *const u8)
            .add(ds2_rva::ITEM_ENTRY_TYPE_OFFSET)
            .read()
    };
    LAST_KIND.store(usize::from(kind), Ordering::Relaxed);
    // Which requirement columns this kind of item has. Weapons and shields use the weapon keys and
    // the two-handed Strength rule; armour and spells have their own keys and no grip rule. Rings
    // and everything else have no stat requirement, so nothing to mark.
    let (keys, halves_strength): (&[u32], bool) = if kind <= ds2_rva::ITEM_ENTRY_TYPE_MAX_INFUSABLE
    {
        (&ds2_rva::FE_ITEM_PARAM_WEAPON_REQUIREMENTS, true)
    } else if ds2_rva::ITEM_ENTRY_TYPE_ARMOUR.contains(&kind) {
        (&ds2_rva::FE_ITEM_PARAM_ARMOUR_REQUIREMENTS, false)
    } else if kind == ds2_rva::ITEM_ENTRY_TYPE_SPELL {
        (&ds2_rva::FE_ITEM_PARAM_SPELL_REQUIREMENTS, false)
    } else {
        return Some(false);
    };

    let mut descriptor = [0u8; ds2_rva::FE_ITEM_DESCRIPTOR_SIZE];
    // SAFETY: every pointer here is one the game handed this detour, or is derived from it by an
    // offset this crate validated before installing. The callee's own contract asks for exactly
    // that live object, and reads inside it go through the fault-tolerant readers.
    let describe: DescriptorFn = unsafe {
        std::mem::transmute::<usize, DescriptorFn>(base + ds2_rva::FE_ITEM_DESCRIPTOR as usize)
    };
    // SAFETY: the buffer is the size the game's own callers give this function, and `item` is the
    // live `FeItemData`.
    unsafe { describe(item, descriptor.as_mut_ptr()) };
    let allocator = usize::from_le_bytes(
        descriptor[ds2_rva::FE_ITEM_DESCRIPTOR_ALLOCATOR_SLOT * 8..][..8]
            .try_into()
            .ok()?,
    );
    if allocator < 0x1_0000 {
        return None;
    }

    let mut rows = [0u8; ds2_rva::FE_ITEM_PARAM_ROWS_SIZE];
    // SAFETY: every pointer here is one the game handed this detour, or is derived from it by an
    // offset this crate validated before installing. The callee's own contract asks for exactly
    // that live object, and reads inside it go through the fault-tolerant readers.
    let resolve_rows: ParamRowsFn = unsafe {
        std::mem::transmute::<usize, ParamRowsFn>(base + ds2_rva::FE_ITEM_PARAM_ROWS as usize)
    };
    // SAFETY: the three arguments are the ones the game passes from `FUN_14003c080`, and the
    // buffer is the `out[0..=8]` that function writes.
    if unsafe { resolve_rows(allocator, rows.as_mut_ptr(), descriptor.as_ptr()) } == 0 {
        return None;
    }
    let row = usize::from_le_bytes(rows[..8].try_into().ok()?);
    if row < 0x1_0000 {
        return None;
    }

    // SAFETY: the frontend root and the stat table are the two hops `FUN_1400bcde0` makes at
    // `0x1400bce07` and `0x1400bce14`.
    let stats = unsafe {
        follow(
            manager,
            &[
                ds2_rva::GAME_MANAGER_FRONTEND_ROOT_OFFSET,
                ds2_rva::FRONTEND_ROOT_PLAYER_STATS_OFFSET,
            ],
        )
    }?;

    // SAFETY: `manager` is the live `GameManagerImp` read above, and `grip` stops at a null hop.
    let two_handed =
        unsafe { grip(manager) }.is_some_and(|grip| ds2_rva::EQUIP_GRIP_TWO_HANDED.contains(&grip));

    // SAFETY: every pointer here is one the game handed this detour, or is derived from it by an
    // offset this crate validated before installing. The callee's own contract asks for exactly
    // that live object, and reads inside it go through the fault-tolerant readers.
    let column: ParamColumnFn = unsafe {
        std::mem::transmute::<usize, ParamColumnFn>(base + ds2_rva::FE_ITEM_PARAM_COLUMN as usize)
    };
    for &key in keys {
        if key >= ds2_rva::FE_STAT_ROW_TABLE_ENTRIES {
            continue;
        }
        let entry_at = base
            + ds2_rva::FE_STAT_ROW_TABLE as usize
            + key as usize * ds2_rva::FE_STAT_ROW_TABLE_STRIDE;
        // SAFETY: the key is inside the table's own bound, checked above against the `cmp rax,0x60`
        // the game's accessor applies.
        let index = unsafe {
            ((entry_at + ds2_rva::FE_STAT_ROW_STAT_INDEX_OFFSET) as *const i16).read_unaligned()
        };
        if index < 0 {
            // Not a requirement column on this build. The game takes the same branch.
            continue;
        }
        // SAFETY: the row is what the game's own resolver returned and the key is one its switch
        // handles with a plain load; the return is in `RAX` with no allocation behind it.
        let mut required = unsafe { column(row, key) } as u16;
        if halves_strength && two_handed && key == ds2_rva::FE_ITEM_PARAM_WEAPON_REQUIRED_STRENGTH {
            // The mechanics check's own `shr cx,1`, so an odd requirement rounds down as it does.
            required >>= 1;
        }
        let required = f32::from(required);
        // SAFETY: the table is the game's, the stride is the one its own indexing uses, and the
        // index came out of the game's table rather than out of this crate.
        let have = unsafe {
            ((stats + index as usize * ds2_rva::PLAYER_STAT_STRIDE) as *const i32).read_unaligned()
        } as f32;
        if required > have {
            let n = MARKED.fetch_add(1, Ordering::Relaxed) + 1;
            if n <= LOGGED_DECISIONS {
                log(format_args!(
                    "{LOG_PREFIX} unmet handle={handle:#06x} kind={kind} key={key:#04x} \
                     stat={index} required={required} have={have} two_handed={two_handed} \
                     marked={n}"
                ));
            }
            return Some(true);
        }
    }
    Some(false)
}

/// Resolve one element id under an infusion container, into a buffer that does not move.
///
/// The accessor is filled in place and never returned by value, because it is self-referential:
/// `FE_ELEMENT_SET_VISIBLE` starts with `mov rcx,[rcx]` on `accessor+0x08`, and a run that returned
/// this struct by value crashed the game there -- the copy carried an interior pointer that aimed
/// at the dead frame it was built in.
///
/// # Safety
///
/// `container` must be the live infusion-container accessor -- for the inventory, the cell view's
/// own at [`ds2_rva::FE_ITEM_CELL_INFUSION_ACCESSOR_OFFSET`]; for the equipment screen, the first
/// argument of [`ds2_rva::FE_EQUIP_SLOT_BIND`], which is already that accessor. `base` must be the
/// module base.
unsafe fn resolve_element(base: usize, container: *mut u8, id: u32, into: &mut Accessor) {
    let mut path = IdPath([0; ds2_rva::FE_ELEMENT_PATH_SIZE]);
    // The game's own alignment idiom, rather than a reliance on this struct's `align`.
    let start = (path.0.as_ptr() as usize).wrapping_neg() & 3;
    path.0[start..][..4].copy_from_slice(&id.to_le_bytes());
    path.0[ds2_rva::FE_ELEMENT_PATH_COUNT_OFFSET..][..8].copy_from_slice(&1u64.to_le_bytes());

    // SAFETY: every pointer here is one the game handed this detour, or is derived from it by an
    // offset this crate validated before installing. The callee's own contract asks for exactly
    // that live object, and reads inside it go through the fault-tolerant readers.
    let resolve: ResolveFn = unsafe {
        std::mem::transmute::<usize, ResolveFn>(base + ds2_rva::FE_ELEMENT_RESOLVE as usize)
    };
    // SAFETY: the parent is an infusion-container accessor -- the same pointer the game's own bind
    // loops hand this function sixteen times, on either screen -- and both buffers are the sizes
    // the game gives them on its own stack.
    unsafe { resolve(container, into.0.as_mut_ptr(), path.0.as_ptr()) };
}

/// The component an accessor stands for, by the same two loads and one indirect call
/// [`ds2_rva::FE_ELEMENT_SET_VISIBLE`] makes before it touches anything.
///
/// The accessor is not a struct with the component in a field. `0x14001e276..0x14001e27f` is
/// `mov rcx,[rcx]` / `mov rax,[rcx]` / `call [rax]`, so the sub-object at
/// [`ds2_rva::FE_ELEMENT_ACCESSOR_VISIBLE_OFFSET`] holds a pointer to an object whose first
/// virtual returns the component. Reading `accessor+0x00` instead returns
/// `FrontendEx::SceneObjProxy::vftable` -- which is what an earlier run reported as a position,
/// and is why this goes through the game's own path rather than a field offset.
///
/// # Safety
///
/// `accessor` must have been filled by [`resolve_element`].
unsafe fn component_of(accessor: &Accessor) -> usize {
    // SAFETY: the resolve either filled the sub-object or left it zeroed, and every hop is
    // range-checked before it is followed.
    unsafe {
        let holder =
            read_usize(accessor.0.as_ptr() as usize + ds2_rva::FE_ELEMENT_ACCESSOR_VISIBLE_OFFSET);
        if holder < 0x1_0000 {
            return 0;
        }
        let vtable = read_usize(holder);
        if vtable < 0x1_0000 {
            return 0;
        }
        let first = read_usize(vtable);
        if first < 0x1_0000 {
            return 0;
        }
        let get = std::mem::transmute::<usize, unsafe extern "system" fn(usize) -> usize>(first);
        get(holder)
    }
}

/// Show or hide the badge on one cell, given that cell's infusion-container accessor.
///
/// # Safety
///
/// `container` must be a live infusion-container accessor and `base` the module base.
unsafe fn show(base: usize, container: *mut u8, visible: bool) {
    let mut accessor = Accessor([0; ds2_rva::FE_ELEMENT_ACCESSOR_SIZE]);
    // SAFETY: the container is the live accessor the caller took off the game's own bind.
    unsafe {
        resolve_element(
            base,
            container,
            ds2_rva::FE_ITEM_WARN_ELEMENT,
            &mut accessor,
        )
    };
    if visible {
        // SAFETY: as above.
        let component = unsafe { component_of(&accessor) };
        if component > 0x1_0000 {
            // Every bind, not once: the element outlives any one cell, and the write is absolute
            // so repeating it changes nothing. See [`crate::place`] for why the corner is written
            // into the component's own quad rather than into the record's transform.
            // SAFETY: `component_of` returns either a live component or zero, checked above.
            unsafe { crate::place::place(component, base) };
        }
    }
    // SAFETY: every pointer here is one the game handed this detour, or is derived from it by an
    // offset this crate validated before installing. The callee's own contract asks for exactly
    // that live object, and reads inside it go through the fault-tolerant readers.
    let set_visible: SetVisibleFn = unsafe {
        std::mem::transmute::<usize, SetVisibleFn>(base + ds2_rva::FE_ELEMENT_SET_VISIBLE as usize)
    };
    // SAFETY: the sub-object is the one all four of the bind's own `setVisible` calls take, and an
    // accessor that resolved to nothing reports nothing rather than faulting -- which is what the
    // bind's own seven unauthored slots rely on every frame.
    unsafe {
        set_visible(
            accessor
                .0
                .as_mut_ptr()
                .add(ds2_rva::FE_ELEMENT_ACCESSOR_VISIBLE_OFFSET),
            u8::from(visible),
        )
    };
}

pub(crate) unsafe extern "system" fn detour(cell: *mut u8, item: *const u8, show_icon: u8) {
    let trampoline = TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        // Published before the site is patched, so unreachable. There is no original to call and
        // no cell to write, so the only honest thing is to do neither.
        return;
    }
    // SAFETY: MinHook published this trampoline for exactly this site, and the signature is the
    // one the decompiled entry implements.
    let original: CellBindFn = unsafe { std::mem::transmute::<usize, CellBindFn>(trampoline) };
    // The original first, always, and unconditionally. Its own loop hides every one of the sixteen
    // infusion slots including the badge's, so running it first is what makes the write below the
    // last word on this cell for this frame.
    // SAFETY: all three arguments are the game's own, passed through unchanged.
    // SAFETY: `original` is the trampoline MinHook produced for this target, so calling it runs the
    // bytes the detour displaced. The arguments are this detour's own, passed through untouched.
    unsafe { original(cell, item, show_icon) };

    if !crate::mark::armed() || (cell as usize) < 0x1_0000 {
        return;
    }
    // SAFETY: the cell is live, and this is the accessor its own bind loop resolves against.
    let container = unsafe { cell.add(ds2_rva::FE_ITEM_CELL_INFUSION_ACCESSOR_OFFSET) };
    // SAFETY: `item` is the game's own and `container` is inside the live cell view.
    unsafe { decide(container, item, "inventory") };
}

/// The equipment screen's bind: `fn(container, item)`.
///
/// Same job, one indirection fewer. [`ds2_rva::FE_EQUIP_SLOT_BIND`] is handed the infusion
/// container's accessor directly, because `FUN_140097150` resolved the slot down to
/// [`ds2_rva::FE_EQUIP_SLOT_CONTAINER_ELEMENT`] before calling it -- so there is no cell view here
/// and nothing to offset into.
///
/// **This is a second hook and not a second call site of the first one.** The equipment screen
/// never goes through [`ds2_rva::FE_ITEM_CELL_BIND`] at all: its slot cells are laid out by
/// `def 0x0133` of `l02_01_In-Game.flo` and refreshed from the slot table at `PTR_DAT_141561ef0`,
/// a path that shares nothing with the inventory list but the container definition. The badge was
/// already in those cells -- the container detour builds it there -- and nothing was switching it
/// on.
pub(crate) unsafe extern "system" fn equip_detour(container: *mut u8, item: *const u8) {
    let trampoline = EQUIP_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        return;
    }
    // SAFETY: MinHook published this trampoline for exactly this site, and the signature is the
    // one the disassembled body implements.
    let original: EquipBindFn = unsafe { std::mem::transmute::<usize, EquipBindFn>(trampoline) };
    // The original first, for the same reason the inventory's runs first: its loop hides all
    // sixteen infusion ids, the badge's among them.
    // SAFETY: both arguments are the game's own, passed through unchanged.
    // SAFETY: `original` is the trampoline MinHook produced for this target, so calling it runs the
    // bytes the detour displaced. The arguments are this detour's own, passed through untouched.
    unsafe { original(container, item) };

    if !crate::mark::armed() || (container as usize) < 0x1_0000 {
        return;
    }
    // SAFETY: `container` is the live accessor the game just used sixteen times.
    unsafe { decide(container, item, "equipment") };
}

/// Ask the question and write the answer onto one cell's badge.
///
/// # Safety
///
/// `container` must be a live infusion-container accessor and `item` the `FeItemData` the bind was
/// called with.
unsafe fn decide(container: *mut u8, item: *const u8, screen: &str) {
    let base = module_base();
    if base == 0 {
        return;
    }
    LAST_KIND.store(NO_KIND, Ordering::Relaxed);
    // SAFETY: `item` is the game's own and `base` is the live module base.
    let answer = unsafe { unmet(base, item) };
    let kind = match LAST_KIND.load(Ordering::Relaxed) {
        NO_KIND => "?".to_string(),
        kind => kind.to_string(),
    };
    // A write on every bind, including the binds where the question could not be asked. The scene
    // element outlives the cell view -- the view is a stack temporary, the element is not -- so a
    // bind that returned early would leave the previous item's badge standing over a different
    // item. It also makes this the last word on the element rather than the game's own loop: an
    // entry carrying an infusion nibble the layout does not author would otherwise light the badge
    // up on its own, and after this write it cannot.
    let visible = answer.unwrap_or(false);
    let (n, cap) = if answer.is_some() {
        (ANSWERS.fetch_add(1, Ordering::Relaxed) + 1, LOGGED_ANSWERS)
    } else {
        (
            DECISIONS.fetch_add(1, Ordering::Relaxed) + 1,
            LOGGED_DECISIONS,
        )
    };
    if n <= cap {
        log(format_args!(
            "{LOG_PREFIX} decided screen={screen} kind={kind} unmet={answer:?} shown={visible} \
             decisions={n}"
        ));
    }
    // SAFETY: `container` is the live accessor the caller took off the game's own bind.
    unsafe { show(base, container, visible) };
}

#[cfg(test)]
mod tests {
    /// Every key this checks is inside the table's own bound and carries a requirement stat.
    #[test]
    fn the_weapon_keys_are_in_range_and_consecutive() {
        let keys = ds2_rva::FE_ITEM_PARAM_WEAPON_REQUIREMENTS;
        for key in keys {
            assert!(key < ds2_rva::FE_STAT_ROW_TABLE_ENTRIES);
        }
        for pair in keys.windows(2) {
            assert_eq!(
                pair[1],
                pair[0] + 1,
                "the four columns are consecutive in the table, which is how they were found"
            );
        }
    }

    /// The badge's element is resolved under the infusion container and nowhere else.
    #[test]
    fn the_badge_hangs_off_the_infusion_container() {
        assert_eq!(
            ds2_rva::FE_ITEM_CELL_INFUSION_ACCESSOR_OFFSET,
            0x2d0,
            "the accessor the bind loop's sixteen resolves use"
        );
        const {
            assert!(
                ds2_rva::FE_ITEM_WARN_ELEMENT >= ds2_rva::FE_ITEM_CELL_INFUSION_ELEMENT_BASE,
                "the badge id has to be one of the ids that loop drives"
            )
        };
    }

    /// The path buffer is big enough for the count field the resolver reads.
    #[test]
    fn the_path_holds_its_count() {
        const {
            assert!(
                ds2_rva::FE_ELEMENT_PATH_COUNT_OFFSET + 8 <= ds2_rva::FE_ELEMENT_PATH_SIZE,
                "the count would be written past the end of the buffer"
            )
        };
    }
}
