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
//! # Which stats it compares against
//!
//! The effective ones. The table at [`ds2_rva::FRONTEND_ROOT_PLAYER_STATS_OFFSET`] is written by
//! [`ds2_rva::FRONTEND_STAT_TABLE_WRITER`] from the same effective block the game's own mechanics
//! check reads, so a Ring of Blades counts here exactly as it counts when the weapon is swung, and
//! the badge cannot light up on a weapon the player can actually use. Whether spEffects are in that
//! block as well as rings is still open; the badge agrees with the game either way.
//!
//! # Two-handing, which the detail pane ignores and this badge does not
//!
//! DS2 does not scale Strength: the mechanics check halves the Strength requirement with
//! `shr cx,1` for grip states `2` and `3`, and the `1.5x` belongs to power stance
//! (`FUN_140350170`, grips `4`, `5`, `6`). The presentation check takes no grip argument, so the
//! detail pane still prints the full requirement in red. This badge reads the player's live grip
//! ([`ChrAsmEquip::grip`]) and, while two-handing, applies the same halving before the
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
//!
//! # Spells the character has no attunement slots for
//!
//! A spell that is not attuned is also marked when it cannot be attuned for want of slots. On the
//! Attune Spell picker that is the game's own greyed-out decision, which `SpellBookItemList`
//! leaves in `FeItemData + 5` ([`ds2_rva::FE_SPELLBOOK_LIST_GET_ITEM`]): free slots plus the cost
//! of the spell in the selected slot, against this spell's cost (column `0x40`). Elsewhere that
//! byte is zero, and the spell is marked when its cost exceeds the whole budget
//! ([`ds2_rva::ITEM_INVENTORY_ATTUNEMENT_BUDGET`]). Attuned spells are never marked for slots.
//!
//! # The attunement grid
//!
//! The Attune Spell screen binds its attuned-spell cells inline, with no infusion loop, so
//! [`cell_view_detour`] catches the one call of the cell-view builder that grid makes and writes
//! the badge there -- which is where an attuned spell whose Intelligence or Faith the character
//! lacks (it can be cast, and nothing happens) gets its X.

use core::mem::offset_of;
use std::sync::atomic::{AtomicUsize, Ordering};

use darksouls2::game::chr::{ChrAsmCtrl, ChrAsmEquip, PlayerCtrl};
use darksouls2::game::game_manager::GameManagerImp;

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
type BudgetFn = unsafe extern "system" fn(usize) -> u8;
type CellViewBuildFn = unsafe extern "system" fn(*mut u8, *mut u8) -> *mut u8;

/// The cell-view builder's own code, published by MinHook before the site is patched. Its detour
/// acts only for the attunement grid.
pub(crate) static CELL_VIEW_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// How many "could not ask" decisions (`None`) to write to the log before going quiet.
///
/// The bind runs per visible row per refresh, so an uncapped line here is a log that fills a disk
/// while the player scrolls. A handful is enough to show the check is being reached.
const LOGGED_DECISIONS: usize = 8;

/// How many real answers (`Some`) to write before going quiet. Kept separate from the `None` cap
/// because the equipment screen binds every slot, empty ones first: with one shared cap, a run on
/// 2026-09-26 spent all of its lines on empty slots and never showed an armour answer.
const LOGGED_ANSWERS: usize = 48;

/// How many "unmet" answers (`Some(true)`) to write, counted apart from the met ones.
///
/// An unmet answer is the one the badge exists for and the rarest, so it gets its own budget:
/// on 2026-09-26 the equipment screen's met answers used up the shared cap before the armour
/// picker's rows were bound, and the picker's answers went unlogged.
const LOGGED_UNMET: usize = 48;

static UNMET: AtomicUsize = AtomicUsize::new(0);
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
                offset_of!(GameManagerImp, player_ctrl),
                offset_of!(PlayerCtrl, base.chr_asm_ctrl),
                offset_of!(ChrAsmCtrl, equip),
            ],
        )
    }?;
    // SAFETY: the equip object is live and the grip is its `i32` field.
    Some(unsafe { ((equip + offset_of!(ChrAsmEquip, grip)) as *const i32).read_unaligned() })
}

/// The character's attunement budget, through the game's own getter; `None` before the inventory
/// has its inner object.
///
/// # Safety
///
/// `base` must be the module base and `inventory` the live manager `ITEM_INVENTORY_ENTRY_LOOKUP`
/// takes.
unsafe fn attunement_budget(base: usize, inventory: usize) -> Option<u8> {
    // SAFETY: the hop `FUN_1401ac110` makes at `0x1401ac11d` before calling the getter.
    let inner = unsafe {
        follow(
            inventory,
            &[ds2_rva::ITEM_INVENTORY_ATTUNEMENT_INNER_OFFSET],
        )
    }?;
    // SAFETY: a three-instruction getter taking that object; the result is the `u8` in `AL`.
    let budget: BudgetFn = unsafe {
        std::mem::transmute::<usize, BudgetFn>(
            base + ds2_rva::ITEM_INVENTORY_ATTUNEMENT_BUDGET as usize,
        )
    };
    // SAFETY: `inner` is the live object the game passes it.
    Some(unsafe { budget(inner) })
}

/// Whether the player fails any of this item's stat requirements -- or, for a spell that is not
/// attuned, cannot attune it for want of slots.
///
/// `None` means the question could not be asked -- no item under the cursor, no inventory entry,
/// no param row -- which is not the same answer as "met" and is why it is not folded into `false`.
///
/// # Safety
///
/// `item` must be the live `FeItemData` the game passed to the cell bind, and `base` the loaded
/// module base.
unsafe fn unmet(base: usize, item: *const u8, picker: bool) -> Option<bool> {
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

    // A spell that is not attuned can also fail for want of slots. The Attune Spell picker has
    // already decided that and left it in the item (`FE_ITEM_DATA_GREYED_OFFSET`, written by
    // `FE_SPELLBOOK_LIST_GET_ITEM`); every other list leaves that byte zero, so there the same
    // column is compared against the whole budget -- the spell is too expensive for this
    // character even with every slot empty. An attuned spell already has its slots and is skipped,
    // which is also what keeps the picker's other reason for greying (already attuned) out.
    if kind == ds2_rva::ITEM_ENTRY_TYPE_SPELL {
        // SAFETY: `entry` is the live inventory entry the lookup returned; the flags are its `u8`.
        let flags = unsafe {
            (entry as *const u8)
                .add(ds2_rva::ITEM_ENTRY_FLAGS_OFFSET)
                .read()
        };
        if flags & ds2_rva::ITEM_ENTRY_FLAG_EQUIPPED == 0 {
            // Read only on the picker: the byte is `SpellBookItemList`'s, and no other list is
            // known to mean the same thing by it.
            // SAFETY: a `FeItemData` is six bytes and the flag is the last of them.
            let greyed =
                picker && unsafe { item.add(ds2_rva::FE_ITEM_DATA_GREYED_OFFSET).read() } != 0;
            // SAFETY: as for the requirement columns below; key `0x40` is a plain byte load.
            let cost = unsafe { column(row, ds2_rva::FE_ITEM_PARAM_SPELL_SLOT_COST) } as u16;
            // SAFETY: `base` is the module base and `inventory` the live manager walked above.
            let budget = unsafe { attunement_budget(base, inventory) };
            let over_budget = budget.is_some_and(|budget| cost > u16::from(budget));
            if greyed || over_budget {
                let n = MARKED.fetch_add(1, Ordering::Relaxed) + 1;
                if n <= LOGGED_DECISIONS {
                    log(format_args!(
                        "{LOG_PREFIX} unmet handle={handle:#06x} kind={kind} reason=slots \
                         greyed={greyed} cost={cost} budget={budget:?} marked={n}"
                    ));
                }
                return Some(true);
            }
        }
    }

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
                    "{LOG_PREFIX} unmet handle={handle:#06x} kind={kind} reason=stat key={key:#04x} \
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

/// Show or hide one element under an infusion container, through the same resolve and
/// `setVisible` the game's bind loop uses.
///
/// # Safety
///
/// `container` must be a live infusion-container accessor and `base` the module base.
unsafe fn set_element_visible(base: usize, container: *mut u8, id: u32, visible: bool) {
    let mut accessor = Accessor([0; ds2_rva::FE_ELEMENT_ACCESSOR_SIZE]);
    // SAFETY: the caller's container is the live accessor, and the buffers are the game's sizes.
    unsafe { resolve_element(base, container, id, &mut accessor) };
    // SAFETY: the function is the game's own `setVisible`, handed the sub-object all of the bind's
    // own calls hand it; an accessor that resolved to nothing reports nothing.
    unsafe {
        let set_visible = std::mem::transmute::<usize, SetVisibleFn>(
            base + ds2_rva::FE_ELEMENT_SET_VISIBLE as usize,
        );
        set_visible(
            accessor
                .0
                .as_mut_ptr()
                .add(ds2_rva::FE_ELEMENT_ACCESSOR_VISIBLE_OFFSET),
            u8::from(visible),
        );
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

/// The entry the item-cell bind is patched to: the caller's `rbx` into `r9`, then on to [`detour`].
///
/// The bind takes three arguments, so `r9` is free. Its only caller, the list rebuild
/// `FUN_1400bc2b0`, holds the `ItemSelectDialog` in `rbx` for the whole loop (`mov rbx,rcx` at
/// `0x1400bc2c5`), and the dialog's list says which list the row came from. Assembly for the same
/// reason as [`cell_view_thunk`]: no Rust expression reads a callee-saved register on entry.
#[unsafe(naked)]
pub(crate) unsafe extern "system" fn cell_bind_thunk(_cell: *mut u8, _item: *const u8, _icon: u8) {
    core::arch::naked_asm!(
        "mov r9, rbx",
        "jmp {detour}",
        detour = sym detour,
    )
}

/// Whether the dialog handed over in `rbx` is the Attune Spell picker: its item list at
/// `FE_ITEM_SELECT_DIALOG_LIST_OFFSET` carries `SpellBookItemList`'s vtable.
///
/// # Safety
///
/// `dialog` must be the caller's `rbx` at the bind's entry.
unsafe fn is_attune_picker(base: usize, dialog: usize) -> bool {
    if dialog < 0x1_0000 || !dialog.is_multiple_of(8) {
        return false;
    }
    // SAFETY: `rbx` at the bind is the list rebuild's `ItemSelectDialog`, and `+0x148` is the field
    // that function itself reads the list from; the vtable is the list's first qword.
    unsafe {
        let Some(list) = follow(dialog, &[ds2_rva::FE_ITEM_SELECT_DIALOG_LIST_OFFSET]) else {
            return false;
        };
        read_usize(list) == base + ds2_rva::FE_SPELLBOOK_ITEM_LIST_VTABLE as usize
    }
}

unsafe extern "system" fn detour(cell: *mut u8, item: *const u8, show_icon: u8, dialog: usize) {
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
    let base = module_base();
    // SAFETY: `dialog` is the caller's `rbx`, handed over by `cell_bind_thunk`.
    let picker = base != 0 && unsafe { is_attune_picker(base, dialog) };
    let screen = if picker { "attune-picker" } else { "inventory" };
    // SAFETY: `item` is the game's own and `container` is inside the live cell view.
    unsafe { decide(container, item, screen, picker) };
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
    unsafe { decide(container, item, "equipment", false) };
}

/// The entry the cell-view builder is patched to: the game's return address into `r8` and the
/// caller's `rsp` at the call into `r9`, then straight on to [`cell_view_detour`].
///
/// The builder takes two arguments, so `r8`/`r9` are free, and the caller already reserved the
/// shadow space the detour's four arguments use. Assembly because no stable Rust expression yields
/// the current function's return address -- the same reason `ds2-boot-timeline`'s `sleep_thunk`
/// exists. The attunement grid's slot item lives in the caller's frame, not in either argument.
#[unsafe(naked)]
pub(crate) unsafe extern "system" fn cell_view_thunk(_view: *mut u8, _cell: *mut u8) -> *mut u8 {
    core::arch::naked_asm!(
        "mov r8, [rsp]",
        "lea r9, [rsp + 8]",
        "jmp {detour}",
        detour = sym cell_view_detour,
    )
}

/// The cell-view builder, for the Attune Spell screen's attunement grid.
///
/// The grid (`FE_ATTUNE_GRID_REFRESH`) binds each attuned spell's cell inline: icon, name,
/// highlight -- and no infusion loop, so the badge the container detour built into those cells is
/// written by nobody else. After the original returns, `view + 0x2d0` is the cell's infusion
/// container, and the slot's `FeItemData` is at `[caller rsp + 0x20]`, where the grid read it just
/// before the call. Every other caller is passed through untouched.
unsafe extern "system" fn cell_view_detour(
    view: *mut u8,
    cell: *mut u8,
    return_address: usize,
    caller_rsp: usize,
) -> *mut u8 {
    let trampoline = CELL_VIEW_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        // Published before the site is patched, so unreachable; there is no view to hand back
        // either way, and the caller's own null check is all that is left.
        return std::ptr::null_mut();
    }
    // SAFETY: MinHook published this trampoline for exactly this site; the signature is the one
    // the disassembled body implements (`mov rax,rdi` before `ret` returns the view).
    let original: CellViewBuildFn =
        unsafe { std::mem::transmute::<usize, CellViewBuildFn>(trampoline) };
    // SAFETY: both arguments are the game's own, passed through unchanged.
    let built = unsafe { original(view, cell) };

    let base = module_base();
    if base == 0
        || return_address != base + ds2_rva::FE_ATTUNE_GRID_CELL_RETURN as usize
        || !crate::mark::armed()
        || (view as usize) < 0x1_0000
    {
        return built;
    }
    let item = (caller_rsp + ds2_rva::FE_ATTUNE_GRID_ITEM_OFFSET) as *const u8;
    // SAFETY: `view` is the cell view the original just filled.
    let container = unsafe { view.add(ds2_rva::FE_ITEM_CELL_INFUSION_ACCESSOR_OFFSET) };
    // The grid has no infusion loop of its own, and its cells' container is one `crate::mark` gave
    // them, so its nine glyphs would otherwise draw as authored. Hidden here the way the inventory
    // bind hides them, before the badge is decided.
    for slot in 0..ds2_rva::FE_ITEM_CELL_INFUSION_SLOTS {
        let id = ds2_rva::FE_ITEM_CELL_INFUSION_ELEMENT_BASE + slot;
        if id != ds2_rva::FE_ITEM_WARN_ELEMENT {
            // SAFETY: as below; an id with no element resolves to nothing and writes nothing.
            unsafe { set_element_visible(base, container, id, false) };
        }
    }
    // SAFETY: the return address proves the caller is the grid refresh, whose frame holds the
    // slot's `FeItemData` at this offset for the whole call; `view` is the cell view the original
    // just filled, and its infusion accessor is the one the inventory bind uses at the same offset.
    unsafe { decide(container, item, "attunement", false) };
    built
}

/// Ask the question and write the answer onto one cell's badge.
///
/// # Safety
///
/// `container` must be a live infusion-container accessor and `item` the `FeItemData` the bind was
/// called with.
unsafe fn decide(container: *mut u8, item: *const u8, screen: &str, picker: bool) {
    let base = module_base();
    if base == 0 {
        return;
    }
    LAST_KIND.store(NO_KIND, Ordering::Relaxed);
    // SAFETY: `item` is the game's own and `base` is the live module base.
    let answer = unsafe { unmet(base, item, picker) };
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
    let (n, cap) = if answer == Some(true) {
        (UNMET.fetch_add(1, Ordering::Relaxed) + 1, LOGGED_UNMET)
    } else if answer.is_some() {
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

    /// The slot-cost column is inside the column switch's bound and is not a stat requirement.
    #[test]
    fn the_slot_cost_column_is_its_own_key() {
        const {
            assert!(ds2_rva::FE_ITEM_PARAM_SPELL_SLOT_COST < ds2_rva::FE_STAT_ROW_TABLE_ENTRIES);
            assert!(
                ds2_rva::FE_ITEM_DATA_GREYED_OFFSET > ds2_rva::FE_ITEM_DATA_HANDLE_OFFSET + 1,
                "the greyed flag sits after the u16 handle"
            );
        };
        assert!(
            !ds2_rva::FE_ITEM_PARAM_SPELL_REQUIREMENTS
                .contains(&ds2_rva::FE_ITEM_PARAM_SPELL_SLOT_COST)
        );
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
