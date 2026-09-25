//! Putting a Prism Stone on the ground, and putting it out again.
//!
//! # The two calls
//!
//! ```text
//! 0x140beb670(sys, ctrl, id, &{float4 pos; float4 dir}, 0, 0xff, 0)   ; spawn
//! 0x140141530(ctrl)                                                   ; stop
//! 0x140a060f0(ctrl + 0x30) ; 0x140a060f0(ctrl)                        ; then destruct, both halves
//! ```
//!
//! `0x140beb590` is the same spawn behind a `float4x4`: it copies row 3 as the position, derives
//! a direction from the matrix and normalises it. Going in at `0x140beb670` with the eight floats
//! removes a matrix layout from the list of things that can be wrong, and the direction is one
//! this crate actually knows -- the way the route runs -- rather than one reconstructed from a
//! matrix built to carry it.
//!
//! # Game thread only, and this time it is not about races
//!
//! `Present` in DARK SOULS II runs on the **same thread as the simulation**: `0x140af09b0` calls
//! the tick and the draw step twenty bytes apart, and several `GameManagerImp` state handlers
//! call the Present wrapper themselves. So the repo's old prose -- "the game's render thread" --
//! is wrong in wording, and the thing it was used to justify was right for a different reason.
//!
//! The reason is the frame, not the thread. `0x140af02f0` Presents with the `KatanaMainApp` sync
//! object and the `DrawSystem` lock both held, *after* `EndDraw` has run and cleared the frame's
//! in-progress flag. Spawning an effect from in there means touching the FX manager's lists
//! inside two locks the draw-command thread and the task workers also take, in a frame the engine
//! considers finished. `NvNavigationSystem::Update` has neither problem: no lock is held and the
//! frame is mid-simulation, which is exactly where all 46 of the engine's own spawn sites are.
//!
//! Neither of these functions takes a lock of its own. Nothing in the spawn path or the stop path
//! contains a `lock` prefix, a `cmpxchg`, or a wait.
//!
//! # Which id
//!
//! [`ds2_rva::PRISM_STONE_SFX_IDS`] -- `833..=839`, the seven colours the game itself picks
//! between at random when you throw one. The config names a single id so a trail is one colour;
//! the doc on that constant has the whole derivation from `ItemParam` row `60450000` down to the
//! seven-dword table the glow spawner indexes.

use ds2_game_base::mem::{game_rva, safe_read_u8, safe_read_u32, safe_read_usize};

/// `(KatanaSfxSystem*, ctrl*, u32 id, const f32* pos_and_dir, u32, u8, u8) -> ctrl*`.
type SpawnSfx = unsafe extern "system" fn(usize, *mut u8, u32, *const f32, u32, u8, u8) -> *mut u8;
/// `(ctrl*)` -- stops both halves of a control block.
type StopSfx = unsafe extern "system" fn(*mut u8);
/// `(ctrl*)` -- destructs ONE half of a control block.
type DestroySfxCtrl = unsafe extern "system" fn(*mut u8);

/// The trailing three arguments, identical at every one of the engine's own call sites --
/// `0x140446ee0` writes `0`, `0xff`, `0` into the stack slots immediately before the call.
const SPAWN_TAIL: (u32, u8, u8) = (0, 0xff, 0);

/// The caller-owned control block `0x140beb670` builds two `FXCGSfxCtrl` into.
///
/// Sixteen-byte aligned because those constructors store vtable pointers with aligned moves, and
/// boxed because the engine links the block itself into the effect node's controller list -- the
/// node's `+0xf8` head points AT these bytes. Moving them after the spawn would leave the engine
/// writing through a pointer to where they used to be.
#[repr(C, align(16))]
struct Block([u8; ds2_rva::KATANA_SFX_CTRL_BYTES]);

/// One live effect, and the storage the engine linked into itself to drive it.
///
/// **No `Drop` impl, on purpose.** One would have to call into the engine, and `Drop` runs
/// wherever the value happens to die -- for a `Vec` being cleared, that is the FX manager's
/// unlocked lists from somewhere they must not be touched. So there are exactly two honest ways
/// to be rid of one, and which is right depends on whether the engine still points at the block:
///
/// | | what happens | when it is right |
/// |---|---|---|
/// | [`Handle::extinguish`] | the effect stops, the block is unlinked, then freed | the effect is still live and in the same area it was spawned in |
/// | `core::mem::forget` | nothing is called, the storage stays allocated forever | the area changed, and whether the engine still holds a pointer is unknowable from here |
///
/// Letting one drop plainly is the third option and it is the wrong one: it frees bytes an
/// effect's controller list may still link to. `crate::gametick` never does it.
pub(crate) struct Handle {
    block: Box<Block>,
}

impl core::fmt::Debug for Handle {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Handle")
            .field("block", &std::ptr::from_ref(&*self.block))
            .finish()
    }
}

impl Handle {
    /// Stop the effect and unlink the block.
    ///
    /// Stop first, destruct second, and the destructor runs on each `0x30`-byte half descending
    /// -- see [`ds2_rva::KATANA_SFX_STOP`] for why that order is the one the engine's own call
    /// sites use and what reversing it risks.
    ///
    /// Safe on a block whose spawn was thrown away: every primitive underneath returns
    /// immediately on null node pointers.
    ///
    /// # Safety
    ///
    /// Game thread only. See the module header.
    pub(crate) unsafe fn extinguish(mut self) {
        let block = std::ptr::from_mut(&mut *self.block).cast::<u8>();
        // SAFETY: every pointer here is one the game handed this detour, or is derived from it by an
        // offset this crate validated before installing. The callee's own contract asks for exactly
        // that live object, and reads inside it go through the fault-tolerant readers.
        let stop: Option<StopSfx> = unsafe { entry(ds2_rva::KATANA_SFX_STOP) };
        // SAFETY: every pointer here is one the game handed this detour, or is derived from it by an
        // offset this crate validated before installing. The callee's own contract asks for exactly
        // that live object, and reads inside it go through the fault-tolerant readers.
        let destroy: Option<DestroySfxCtrl> = unsafe { entry(ds2_rva::KATANA_SFX_CTRL_DESTROY) };
        if let Some(stop) = stop {
            // SAFETY: `block` is this handle's own storage, still where the spawn left it.
            unsafe { stop(block) };
        }
        if let Some(destroy) = destroy {
            // SAFETY: the two halves, descending, exactly as `0x140446ee0` tears its own down.
            unsafe {
                destroy(block.add(ds2_rva::KATANA_SFX_CTRL_HALF_BYTES));
                destroy(block);
            }
        }
        // `self` -- and the box -- die here, after the engine has been told to let go of them.
    }
}

/// Resolve an RVA to a function pointer. See `crate::navquery::entry`, which is the same three
/// lines for the same reason; duplicated rather than shared because a `pub(crate)` generic
/// transmute helper is a thing worth having to write out at each use.
unsafe fn entry<T: Copy>(rva: u32) -> Option<T> {
    const {
        assert!(size_of::<T>() == size_of::<usize>());
    }
    let address = game_rva(rva).ok()?;
    // SAFETY: `T` is a pointer-sized function pointer (asserted above) and `address` is a
    // resolved RVA inside the loaded game image.
    Some(unsafe { std::mem::transmute_copy::<usize, T>(&address) })
}

/// The live `KatanaSfxSystem`, if it exists and says it is ready.
///
/// `None` covers three different states and all three mean "do not spawn": there is no
/// `GameManagerImp` yet, the system pointer is null, or its ready byte at
/// [`ds2_rva::KATANA_SFX_SYSTEM_READY_OFFSET`] is zero. `0x140446ee0` tests the same two things
/// in the same order before touching anything else.
pub(crate) fn system() -> Option<usize> {
    let manager = crate::navquery::game_manager()?;
    // SAFETY: a live `GameManagerImp`; the reader reports an unmapped page rather than faulting.
    // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one -- it reads
    // through `ReadProcessMemory`, which validates the range in the kernel. A game structure that
    // moved or was freed answers None rather than faulting.
    let system = unsafe { safe_read_usize(manager + ds2_rva::GAME_MANAGER_SFX_SYSTEM_OFFSET)? };
    if system == 0 {
        return None;
    }
    // SAFETY: as above.
    // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one -- it reads
    // through `ReadProcessMemory`, which validates the range in the kernel. A game structure that
    // moved or was freed answers None rather than faulting.
    let ready = unsafe { safe_read_u8(system + ds2_rva::KATANA_SFX_SYSTEM_READY_OFFSET)? };
    (ready != 0).then_some(system)
}

/// The SFX system's current quality level.
///
/// Worth logging once per session because [`ds2_rva::KATANA_SFX_QUALITY_DROP_THRESHOLD`] and
/// above means spawns are being discarded and the caller is handed a control block that looks
/// exactly like a success. "The trail stopped working in the boss arena" is otherwise an
/// unanswerable report.
pub(crate) fn quality(system: usize) -> Option<u32> {
    // SAFETY: a live `KatanaSfxSystem`; the reader refuses an unmapped page.
    // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one -- it reads
    // through `ReadProcessMemory`, which validates the range in the kernel. A game structure that
    // moved or was freed answers None rather than faulting.
    unsafe { safe_read_u32(system + ds2_rva::KATANA_SFX_SYSTEM_QUALITY_OFFSET) }
}

/// Spawn one effect at `position`, facing `direction`.
///
/// Returns `None` when nothing was created -- which includes the ordinary case of the engine's
/// quality throttle silently discarding it. That case is detected rather than assumed: a
/// discarded spawn comes back as a control block built by `0x140127240` with both of its
/// [`ds2_rva::KATANA_SFX_CTRL_NODE_OFFSET`] pointers null, and is indistinguishable from a
/// success by any other means.
///
/// The block is still destructed on that path, because an empty block is harmless to destruct
/// and leaking one would be a slow leak of 0x68-byte boxes for as long as the throttle lasts.
///
/// # Safety
///
/// Game thread only. See the module header.
pub(crate) unsafe fn spawn(
    system: usize,
    sfx_id: u32,
    position: [f32; 3],
    direction: [f32; 3],
) -> Option<Handle> {
    if sfx_id == 0 || !position.iter().all(|c| c.is_finite()) {
        return None;
    }
    // SAFETY: every pointer here is one the game handed this detour, or is derived from it by an
    // offset this crate validated before installing. The callee's own contract asks for exactly
    // that live object, and reads inside it go through the fault-tolerant readers.
    let spawn: SpawnSfx = unsafe { entry(ds2_rva::KATANA_SFX_SPAWN)? };

    // The direction must be a UNIT vector: `0x140beb590` normalises before calling this, so the
    // core is entitled to assume it. A route segment of zero length gives nothing to normalise,
    // and "up" is the honest answer for a marker that has no direction of travel -- it is the
    // one axis a ground effect can use without claiming a heading it does not have.
    let unit = crate::geometry::normalize(direction).unwrap_or([0.0, 1.0, 0.0]);
    let pose = Pose::new(position, unit);

    let mut block = Box::new(Block([0u8; ds2_rva::KATANA_SFX_CTRL_BYTES]));
    let raw = std::ptr::from_mut(&mut *block).cast::<u8>();
    // SAFETY: `system` is the engine's own singleton and has said it is ready; `raw` is 0x68
    // zeroed 16-byte-aligned bytes this call owns and does not move; `pose` is the eight floats
    // the function reads. The trailing three are the constants every engine call site passes.
    unsafe {
        spawn(
            system,
            raw,
            sfx_id,
            pose.as_ptr(),
            SPAWN_TAIL.0,
            SPAWN_TAIL.1,
            SPAWN_TAIL.2,
        );
    }

    let handle = Handle { block };
    if handle.is_live() {
        Some(handle)
    } else {
        // SAFETY: game thread, and the primitives short-circuit on the null node pointers that
        // just proved this block controls nothing.
        unsafe { handle.extinguish() };
        None
    }
}

/// [`spawn`], with the three things that distinguish identical-looking failures sampled around
/// it.
///
/// Used by the self-check rather than by the trail, because the missing-effect tree walk costs a
/// descent per attempt and the trail places stones sixty times a second. The trail wants speed;
/// a diagnostic wants to know WHY, and a report of "I see nothing" is only worth reading if the
/// log can separate the reasons.
///
/// Note the ORDER: the quality byte and the tree are read BEFORE the call, and the tree again
/// after. Reading the quality afterwards would report the level during the frame's next load
/// spike rather than the one that made the decision, and reading the tree only afterwards cannot
/// tell "this attempt's lookup failed" from "some earlier attempt's did".
///
/// # Safety
///
/// Game thread only, exactly as [`spawn`].
pub(crate) unsafe fn spawn_reporting(
    system: usize,
    sfx_id: u32,
    position: [f32; 3],
    direction: [f32; 3],
) -> Attempt {
    let quality = quality(system);
    let missing_before = id_is_missing(system, sfx_id);
    // SAFETY: forwarded from this function's own contract.
    let handle = unsafe { spawn(system, sfx_id, position, direction) };
    let missing_after = id_is_missing(system, sfx_id);
    Attempt {
        handle,
        quality,
        missing_before,
        missing_after,
    }
}

impl Handle {
    /// Did the engine actually build something?
    ///
    /// Reads [`ds2_rva::KATANA_SFX_CTRL_NODE_OFFSET`] in both halves. Both null is the shape
    /// `0x140127240` leaves behind when the quality throttle discards a spawn.
    ///
    /// **This is the engine's own predicate, inlined.** `0x140a06580` -- the function everything
    /// else in the image calls to ask the same question -- is a five-byte `jmp` into Arxan whose
    /// entire body is `cmp [rcx+0x10],0` and `cmp [rcx+0x18],0`. See
    /// [`ds2_rva::KATANA_SFX_CTRL_IS_EMPTY`], which exists to record that equivalence rather
    /// than to be called: a hand-written prototype over three stack-swapping Arxan fragments is
    /// a worse way to learn what two fault-safe reads already say.
    fn is_live(&self) -> bool {
        self.nodes().into_iter().any(|node| node != 0)
    }

    /// The two effect-node pointers, zero where there is none.
    fn nodes(&self) -> [usize; 2] {
        let base = std::ptr::from_ref(&*self.block) as usize;
        [0, ds2_rva::KATANA_SFX_CTRL_HALF_BYTES].map(|half| {
            // SAFETY: an address inside this handle's own box.
            // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one -- it reads
            // through `ReadProcessMemory`, which validates the range in the kernel. A game structure that
            // moved or was freed answers None rather than faulting.
            unsafe { safe_read_usize(base + half + ds2_rva::KATANA_SFX_CTRL_NODE_OFFSET) }
                .unwrap_or(0)
        })
    }

    /// Is the effect still playing?
    ///
    /// **The one question that says whether an effect LINGERS**, and it is not the same question
    /// as [`Handle::is_live`]: that one asks whether the spawn produced anything at all, this one
    /// asks whether what it produced is still going N seconds later.
    ///
    /// Bit 30 of `node + 0x58` -- see [`ds2_rva::KATANA_SFX_NODE_ALIVE_BIT`], read off the engine
    /// handle method that self-nulls a control block the moment that bit goes clear. A block the
    /// engine has already disowned reads as all-null nodes and answers `false` here without
    /// dereferencing anything.
    ///
    /// # Safety
    ///
    /// Read-only and fault-safe, but the node it reads belongs to the area the effect was spawned
    /// in. Call it from the game thread, in that area.
    pub(crate) unsafe fn alive(&self) -> bool {
        self.nodes().into_iter().any(|node| {
            node != 0
                // SAFETY: `node` is an engine FX node this block is linked into; the read
                // refuses an unmapped page rather than faulting.
                && unsafe { safe_read_u32(node + ds2_rva::KATANA_SFX_NODE_ALIVE_OFFSET) }
                    .is_some_and(|word| word & ds2_rva::KATANA_SFX_NODE_ALIVE_BIT != 0)
        })
    }
}

/// Is `id` in the system's missing-effect tree?
///
/// `None` means the tree could not be walked -- a torn read or a depth blow-out -- which is a
/// third answer and must not be reported as either of the other two.
///
/// # What a `true` means
///
/// The engine looked this id up, did not find it, and recorded that. For a self-check this is the
/// line between **"the effect is not resident in this map"** and **"it spawned and you cannot see
/// it"**, which look identical on the ground and are the most expensive pair in the whole feature
/// to separate by walking around.
///
/// The walk is the `lower_bound` descent out of `0x140beb400` with the insert arm removed, so it
/// is read-only: no allocation, no engine call, no write. It is bounded by
/// [`ds2_rva::KATANA_SFX_MISSING_MAX_DEPTH`] because the game is free to rebalance the tree
/// while this reads it, and a torn read must end the walk rather than spin on the simulation
/// thread.
pub(crate) fn id_is_missing(system: usize, id: u32) -> Option<bool> {
    // SAFETY: a live `KatanaSfxSystem`; every read below refuses an unmapped page.
    // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one -- it reads
    // through `ReadProcessMemory`, which validates the range in the kernel. A game structure that
    // moved or was freed answers None rather than faulting.
    let head = unsafe { safe_read_usize(system + ds2_rva::KATANA_SFX_MISSING_IDS_OFFSET)? };
    if head == 0 {
        return Some(false);
    }
    // The head is not a node: its `_Parent` is the root, and the tree is empty when that is the
    // head itself.
    // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one -- it reads
    // through `ReadProcessMemory`, which validates the range in the kernel. A game structure that
    // moved or was freed answers None rather than faulting.
    let mut node = unsafe { safe_read_usize(head + ds2_rva::KATANA_SFX_MISSING_PARENT_OFFSET)? };
    let mut best: Option<u32> = None;
    for _ in 0..ds2_rva::KATANA_SFX_MISSING_MAX_DEPTH {
        if node == 0 {
            return None;
        }
        // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one -- it reads
        // through `ReadProcessMemory`, which validates the range in the kernel. A game structure that
        // moved or was freed answers None rather than faulting.
        let nil = unsafe { safe_read_u8(node + ds2_rva::KATANA_SFX_MISSING_ISNIL_OFFSET)? };
        if nil != 0 {
            // Off the bottom of the tree: `best` holds the lower bound, and the id is present
            // only if that bound is the id itself.
            return Some(best == Some(id));
        }
        // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one -- it reads
        // through `ReadProcessMemory`, which validates the range in the kernel. A game structure that
        // moved or was freed answers None rather than faulting.
        let key = unsafe { safe_read_u32(node + ds2_rva::KATANA_SFX_MISSING_KEY_OFFSET)? };
        node = if key < id {
            // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one -- it reads
            // through `ReadProcessMemory`, which validates the range in the kernel. A game structure that
            // moved or was freed answers None rather than faulting.
            unsafe { safe_read_usize(node + ds2_rva::KATANA_SFX_MISSING_RIGHT_OFFSET)? }
        } else {
            best = Some(key);
            // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one -- it reads
            // through `ReadProcessMemory`, which validates the range in the kernel. A game structure that
            // moved or was freed answers None rather than faulting.
            unsafe { safe_read_usize(node + ds2_rva::KATANA_SFX_MISSING_LEFT_OFFSET)? }
        };
    }
    None
}

/// Everything one spawn attempt found out, for the self-check's log.
///
/// The three fields beyond the handle are the three ways a spawn can produce nothing while
/// looking exactly like a success, and they have to be sampled AT the attempt -- the quality
/// level moves with the frame's load, and the missing-effect tree is written by the attempt
/// itself.
pub(crate) struct Attempt {
    /// The live effect, or `None` if the block came back empty.
    pub(crate) handle: Option<Handle>,
    /// [`ds2_rva::KATANA_SFX_SYSTEM_QUALITY_OFFSET`] read immediately before the call.
    pub(crate) quality: Option<u32>,
    /// Was the id already in the missing-effect tree before this attempt?
    pub(crate) missing_before: Option<bool>,
    /// Is it in there now? `false -> true` across one attempt is this attempt's own lookup
    /// failing, which is the strongest evidence available that the id is not in this map.
    pub(crate) missing_after: Option<bool>,
}

impl Attempt {
    /// One line saying what happened, in the order a reader needs it.
    pub(crate) fn describe(&self, id: u32) -> String {
        let quality = self.quality.map_or_else(
            || "quality ?".to_string(),
            |level| format!("quality {level}"),
        );
        let throttled = self
            .quality
            .is_some_and(|level| level >= ds2_rva::KATANA_SFX_QUALITY_DROP_THRESHOLD);
        let tree = match (self.missing_before, self.missing_after) {
            (_, Some(false)) => "the id resolved".to_string(),
            (Some(false), Some(true)) => {
                "THIS attempt's lookup failed -- the effect is not resident in this map".to_string()
            }
            (Some(true), Some(true)) => {
                "the id was already recorded as missing before this attempt".to_string()
            }
            _ => "missing-effect tree unreadable".to_string(),
        };
        match &self.handle {
            Some(_) => format!("id {id}: spawned, {quality}, {tree}"),
            None if throttled => format!(
                "id {id}: EMPTY -- {quality}, AT OR ABOVE THE THRESHOLD AT WHICH THE ENGINE \
                 DISCARDS SPAWNS, so this says nothing about the id; {tree}"
            ),
            None => format!("id {id}: EMPTY -- {quality} (not throttled), {tree}"),
        }
    }
}

/// The eight floats `0x140beb670` reads: a world position, then a unit direction.
///
/// Sixteen-byte aligned for the same reason `crate::navquery::Aligned4` is -- the engine loads
/// both halves with `movaps`.
#[repr(C, align(16))]
struct Pose([f32; 8]);

impl Pose {
    fn new(position: [f32; 3], direction: [f32; 3]) -> Self {
        Self([
            position[0],
            position[1],
            position[2],
            0.0,
            direction[0],
            direction[1],
            direction[2],
            0.0,
        ])
    }

    fn as_ptr(&self) -> *const f32 {
        self.0.as_ptr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layout contract with the engine.
    ///
    /// **The box is bigger than the block, and that is `align(16)` doing its job rather than a
    /// mistake.** `0x68` is 104 bytes, which is not a multiple of sixteen, so Rust rounds the
    /// type up to 112 to keep arrays of it aligned. The engine writes the first 104 and the
    /// remaining eight are slack this crate owns and nothing reads. What must hold is that the
    /// storage is AT LEAST what the engine writes and aligned the way its constructors' `movaps`
    /// requires -- asserting equality instead is how a passing test gets deleted for being wrong
    /// about the language rather than about the game.
    #[test]
    fn the_control_block_is_at_least_the_size_and_the_alignment_the_engine_writes() {
        assert!(size_of::<Block>() >= ds2_rva::KATANA_SFX_CTRL_BYTES);
        assert_eq!(size_of::<Block>() % 16, 0);
        assert_eq!(align_of::<Block>(), 16);
        assert_eq!(
            ds2_rva::KATANA_SFX_CTRL_HALF_BYTES * 2 + 8,
            ds2_rva::KATANA_SFX_CTRL_BYTES,
            "the block is two halves plus the `param_2[0xc] = 0` at +0x60"
        );
    }

    #[test]
    fn a_pose_is_eight_aligned_floats_in_the_engines_order() {
        let pose = Pose::new([1.0, 2.0, 3.0], [0.0, 1.0, 0.0]);
        assert_eq!(size_of::<Pose>(), 32);
        assert_eq!(align_of::<Pose>(), 16);
        assert_eq!(pose.0, [1.0, 2.0, 3.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    }

    /// The seven ids are the seven colours of one item, so any of them is a legitimate setting
    /// and none of them is zero -- zero is the config's "off".
    #[test]
    fn the_prism_stone_ids_are_usable_as_settings() {
        assert_eq!(ds2_rva::PRISM_STONE_SFX_IDS.len(), 7);
        assert!(ds2_rva::PRISM_STONE_SFX_IDS.iter().all(|id| *id != 0));
        assert!(
            ds2_rva::PRISM_STONE_SFX_IDS
                .iter()
                .all(|id| *id <= ds2_rva::KATANA_SFX_BASE_ID_MAX),
            "an id above the base maximum would skip the engine's own variant lookup"
        );
    }
}
