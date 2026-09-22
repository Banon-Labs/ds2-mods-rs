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
        let stop: Option<StopSfx> = unsafe { entry(ds2_rva::KATANA_SFX_STOP) };
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
    let system = unsafe { safe_read_usize(manager + ds2_rva::GAME_MANAGER_SFX_SYSTEM_OFFSET)? };
    if system == 0 {
        return None;
    }
    // SAFETY: as above.
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

impl Handle {
    /// Did the engine actually build something?
    ///
    /// Reads [`ds2_rva::KATANA_SFX_CTRL_NODE_OFFSET`] in both halves. Both null is the shape
    /// `0x140127240` leaves behind when the quality throttle discards a spawn.
    fn is_live(&self) -> bool {
        let base = std::ptr::from_ref(&*self.block) as usize;
        [0, ds2_rva::KATANA_SFX_CTRL_HALF_BYTES]
            .into_iter()
            .any(|half| {
                // SAFETY: an address inside this handle's own box.
                unsafe { safe_read_usize(base + half + ds2_rva::KATANA_SFX_CTRL_NODE_OFFSET) }
                    .is_some_and(|node| node != 0)
            })
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
