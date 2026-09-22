//! Finding the camera the frame is actually drawn through, and refusing when it cannot be found.
//!
//! # Why this searches instead of dereferencing one address
//!
//! The layout is settled: a `CameraOperator` holds a world-to-camera matrix at `+0x10` and the
//! projection at `+0x50`, both written by `0x140493290`, and the projection's shape comes
//! straight out of the builder at `0x140001a90`. What is *not* settled is which camera object a
//! given frame goes through -- `CameraManager` holds three operator pointers (free, player,
//! in-game) plus six embedded slots, and which of them is live depends on whether a cutscene, a
//! menu or ordinary play is on screen.
//!
//! Picking one and hoping is how an overlay ends up drawing lines in the wrong place with nothing
//! saying why. So this enumerates the candidates -- a fixed list, every one of them reached by a
//! pointer hop the engine itself performs -- and keeps the one that passes two independent tests:
//!
//! 1. **Shape.** The projection matrix matches the signature of what `0x140001a90` writes:
//!    `m23 == 1`, `m33 == 0`, ten specific zeros, `m00 <= m11`, `0 < m22 <= 1`, `m32 < 0`. An
//!    identity matrix fails it, a view matrix fails it, and a fresh camera slot -- which the
//!    constructor fills with two identities -- fails it.
//! 2. **Agreement with the world.** The combined matrix puts the LOCAL PLAYER, whose position is
//!    read independently in `crate::census`, somewhere a viewport could plausibly show them.
//!
//! Test 1 alone would accept a stale projection belonging to a camera nothing is rendering. Test
//! 2 alone would accept any matrix that happened to land the player on screen. Together they are
//! a candidate that is both shaped like a projection and consistent with where the game says the
//! player is -- which is not something a wrong guess produces.
//!
//! **No candidate passing is an answer, not a failure to handle.** The overlay draws nothing and
//! the log says which candidates were rejected and why. An overlay that guesses is worse than one
//! that is absent, because the player cannot tell a wrong line from a lying one.
//!
//! The winner is remembered so the search runs once rather than every frame, and it is re-checked
//! on every use: if the remembered candidate stops passing -- a cutscene taking over, a map load
//! freeing the object -- the search runs again.

use ds2_game_base::mem::{game_rva, read_bytes, safe_read_usize};

use crate::geometry::{Camera, Matrix, is_finite, looks_like_a_projection, multiply};

/// Where a candidate's two matrices live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Candidate {
    /// The object holding them.
    pub(crate) object: usize,
    /// Offset of the world-to-camera matrix within it.
    pub(crate) view: usize,
    /// Offset of the projection matrix within it.
    pub(crate) projection: usize,
    /// Where this candidate came from, for the log line. Nothing branches on it.
    pub(crate) origin: Origin,
}

/// Which of the three places a candidate was found.
///
/// Printed when a search succeeds, and that line is the whole point of distinguishing them: the
/// day it says `pointer[+0x0e8]` twice in two sessions, that offset can be pinned in `ds2-rva`
/// and the search deleted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// One of `CameraManager`'s three named `CameraOperator` pointers, by index into
    /// [`ds2_rva::CAMERA_MANAGER_OPERATOR_OFFSETS`]. Tried first because they are named.
    Operator(usize),
    /// One of the six embedded slots, by index.
    Slot(usize),
    /// An unnamed pointer field of `CameraManager`, by its offset.
    ///
    /// This is the one that does not assume a field name is right, and it exists because one on
    /// this family of structs already was not -- `CharacterManager+0x50` is labelled
    /// `player_ctrl` and is an array base.
    Pointer(usize),
}

impl core::fmt::Display for Origin {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Origin::Operator(index) => write!(f, "operator[{index}]"),
            Origin::Slot(index) => write!(f, "slot[{index}]"),
            Origin::Pointer(offset) => write!(f, "pointer[+0x{offset:03x}]"),
        }
    }
}

/// `Some(pointer)` unless it is null.
const fn non_null(pointer: usize) -> Option<usize> {
    if pointer == 0 { None } else { Some(pointer) }
}

/// `CameraManager`, or `None` before there is a world.
fn camera_manager() -> Option<usize> {
    let manager_address = game_rva(ds2_rva::GAME_MANAGER_IMP).ok()?;
    // SAFETY: a resolved RVA inside the loaded image, read fault-safely -- the global is null
    // until the game builds it.
    let game_manager = non_null(unsafe { safe_read_usize(manager_address)? })?;
    non_null(unsafe {
        safe_read_usize(game_manager + ds2_rva::GAME_MANAGER_CAMERA_MANAGER_OFFSET)?
    })
}

/// Read sixteen `f32` at `at`, refusing anything that is not sixteen real numbers.
fn matrix(at: usize) -> Option<Matrix> {
    let mut bytes = [0u8; 64];
    // SAFETY: `read_bytes` reports an unmapped page rather than faulting; `at` is always an
    // engine pointer plus a recorded offset.
    if !unsafe { read_bytes(at, &mut bytes) } {
        return None;
    }
    let mut out = [0.0f32; 16];
    for (index, slot) in out.iter_mut().enumerate() {
        let word: [u8; 4] = bytes[index * 4..index * 4 + 4]
            .try_into()
            .expect("a 4-byte window of a 64-byte array");
        *slot = f32::from_le_bytes(word);
    }
    if is_finite(&out) { Some(out) } else { None }
}

/// Every place a camera could be, in the order they are tried.
///
/// The three operator pointers come first because that is where the layout is *proven* -- both
/// the constructor that fills the two matrices and the function that writes them are read. The
/// six embedded slots come after because they are the right shape and in the right place but
/// nobody has read their writer; if one of them ever wins, that is worth a note in `ds2-rva`
/// rather than a shrug.
pub(crate) fn candidates() -> Vec<Candidate> {
    let mut out = Vec::new();
    let Some(manager) = camera_manager() else {
        return out;
    };
    for (index, offset) in ds2_rva::CAMERA_MANAGER_OPERATOR_OFFSETS
        .into_iter()
        .enumerate()
    {
        // SAFETY: `manager` is the engine's own `CameraManager`; the read is fault-safe and the
        // pointer is null before the operator is built.
        let Some(operator) = (unsafe { safe_read_usize(manager + offset) }) else {
            continue;
        };
        let Some(operator) = non_null(operator) else {
            continue;
        };
        out.push(Candidate {
            object: operator,
            view: ds2_rva::CAMERA_OPERATOR_VIEW_OFFSET,
            projection: ds2_rva::CAMERA_OPERATOR_PROJECTION_OFFSET,
            origin: Origin::Operator(index),
        });
    }
    for index in 0..ds2_rva::CAMERA_MANAGER_SLOT_COUNT {
        let slot = manager
            + ds2_rva::CAMERA_MANAGER_SLOT_BASE
            + index * ds2_rva::CAMERA_MANAGER_SLOT_STRIDE;
        let [first, second] = ds2_rva::CAMERA_MANAGER_SLOT_MATRIX_OFFSETS;
        // Both orderings, because which of the two blocks is the projection is exactly what has
        // not been established. The shape test answers it in one comparison, and answering it
        // that way is cheaper and more honest than asserting an order.
        out.push(Candidate {
            object: slot,
            view: first,
            projection: second,
            origin: Origin::Slot(index),
        });
        out.push(Candidate {
            object: slot,
            view: second,
            projection: first,
            origin: Origin::Slot(index),
        });
    }

    // EVERY OTHER POINTER THE OBJECT HOLDS, and this is the part that stopped being optional.
    //
    // The first live run logged `no camera matched the projection shape`. The three named
    // operator pointers are camera CONTROLLERS -- `IngameCameraOperator`'s constructor chains to
    // `0x140b455a0`, not to `CameraOperator`'s `0x140ae8180`, so it is a different class with a
    // different layout and no matrices where this looks for them. Meanwhile the object that DOES
    // own them is reached by some pointer on this struct, and the field names here have already
    // been wrong once (`CharacterManager+0x50`).
    //
    // So: try them all. This is a search rather than a spray because the oracle is strong and was
    // derived statically -- a candidate must have the exact shape `0x140001a90` emits AND put the
    // player where a viewport could show them. Every read is fault-safe, the bound is one struct,
    // and it runs once per session because the winner is remembered.
    //
    // The offset that wins is LOGGED. Two sessions agreeing is what turns this back into a
    // constant in `ds2-rva` and deletes the search.
    for offset in (0..ds2_rva::CAMERA_MANAGER_SIZE).step_by(core::mem::size_of::<usize>()) {
        if ds2_rva::CAMERA_MANAGER_OPERATOR_OFFSETS.contains(&offset) {
            continue;
        }
        // SAFETY: inside the object the engine's own `CameraManager` occupies; fault-safe.
        let Some(pointer) = (unsafe { safe_read_usize(manager + offset) }) else {
            continue;
        };
        // A pointer-shaped value, not a float pair or a small integer that happens to sit here.
        // SAFETY: a pure predicate on the value; it dereferences nothing.
        if !unsafe { ds2_game_base::mem::is_heap_aligned_ptr(pointer) } {
            continue;
        }
        out.push(Candidate {
            object: pointer,
            view: ds2_rva::CAMERA_OPERATOR_VIEW_OFFSET,
            projection: ds2_rva::CAMERA_OPERATOR_PROJECTION_OFFSET,
            origin: Origin::Pointer(offset),
        });
    }
    out
}

/// How far into a candidate object the view matrix is looked for.
///
/// `0x140` covers `CAMERA_OPERATOR_VIEW_OFFSET` (`+0x10`), the slot offsets (`+0x00`, `+0x40`)
/// and a generous margin past `CAMERA_OPERATOR_PROJECTION_OFFSET` (`+0x50`). Not larger: past
/// this the odds of some unrelated sixteen floats happening to satisfy the on-screen test stop
/// being negligible, and a search that can guess right by accident is not a search.
const VIEW_SEARCH_SPAN: usize = 0x140;

/// Matrices in this engine are written with `movaps`, so they are sixteen-byte aligned and the
/// search only ever needs to look on those boundaries.
const MATRIX_ALIGNMENT: usize = 16;

/// The projection at a candidate's recorded offset, if it is one at all.
///
/// This is test 1 of the two in this module's header, on its own. It says nothing about whether
/// the camera is the live one.
pub(crate) fn projection_at(candidate: Candidate) -> Option<Matrix> {
    let projection = matrix(candidate.object + candidate.projection)?;
    looks_like_a_projection(&projection).then_some(projection)
}

/// Combine a view matrix read from `object + view_offset` with an already-validated projection.
pub(crate) fn pair(object: usize, view_offset: usize, projection: Matrix) -> Option<Camera> {
    let view = matrix(object + view_offset)?;
    let view_projection = multiply(&view, &projection);
    if !is_finite(&view_projection) {
        return None;
    }
    Some(Camera {
        view,
        view_projection,
    })
}

/// Read a candidate whole -- both matrices at their recorded offsets.
///
/// Used to re-check a REMEMBERED candidate, where both offsets are already known to work. The
/// search path uses [`projection_at`] and [`pair`] instead, because there the view offset is the
/// unknown.
pub(crate) fn resolve(candidate: Candidate) -> Option<Camera> {
    let projection = projection_at(candidate)?;
    pair(candidate.object, candidate.view, projection)
}

/// Which candidate the last successful search chose, so the search is not repeated every frame.
///
/// Deliberately not an `AtomicUsize` cache of the resolved matrices: the matrices change every
/// frame and only the *location* is stable. Re-reading them is four cache lines; re-searching for
/// them is a dozen fault-safe probes.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Tracker {
    remembered: Option<Candidate>,
    /// What the last search saw. Read by the caller for its refusal line.
    pub(crate) last_probe: Probe,
}

/// What one search looked at, so a refusal can say WHICH of the two tests everything failed.
///
/// `shaped = 0` means nothing in the object looked like a projection matrix at all -- the search
/// is in the wrong place. `shaped > 0` with no winner means projections were found and none of
/// them agreed with where the player is -- the search is in the right place and picking wrong.
/// Those want opposite next moves, and a bare "no camera" cannot tell them apart. The first live
/// run proved that: it said no camera, and said nothing about which half had failed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Probe {
    /// Candidates examined.
    pub(crate) tried: usize,
    /// Of those, ones whose projection matrix passed the shape test.
    pub(crate) shaped: usize,
}

/// What a search did, for the log.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Found {
    /// The remembered candidate still works. Not logged; this is the ordinary frame.
    Remembered,
    /// A search ran and chose this one. Worth one line -- and the line carries the whole
    /// candidate, not just where it was found, because the view and projection offsets are what
    /// a later reader needs to pin this down as a constant and delete the search.
    Chose(Candidate),
}

impl core::fmt::Display for Candidate {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{} obj=0x{:x} view=+0x{:03x} proj=+0x{:03x}",
            self.origin, self.object, self.view, self.projection
        )
    }
}

impl Tracker {
    /// The camera for this frame, or `None` if none of the candidates is both shaped like a
    /// projection and consistent with where the player is.
    ///
    /// `local` is the local player's world position, read independently in `crate::census`, and
    /// `screen` is the back buffer's size. Both are what makes test 2 a test rather than a
    /// restatement of test 1.
    pub(crate) fn acquire(&mut self, local: [f32; 3], screen: [f32; 2]) -> Option<(Camera, Found)> {
        if let Some(remembered) = self.remembered
            && let Some(camera) = resolve(remembered)
            && camera.plausibly_on_screen(local, screen)
        {
            return Some((camera, Found::Remembered));
        }
        self.remembered = None;
        let mut probe = Probe::default();
        for candidate in candidates() {
            probe.tried += 1;
            let Some(projection) = projection_at(candidate) else {
                continue;
            };
            probe.shaped += 1;
            // THE PROJECTION FOUND ITS OBJECT; NOW FIND ITS PARTNER.
            //
            // The second live run said `96 candidates tried, 1 shaped like a projection`, which
            // is the good half of a failure: the search is in the right object and exactly one
            // real projection matrix is in it. What failed was the PAIRING -- this used to assume
            // the view matrix sat at `CAMERA_OPERATOR_VIEW_OFFSET` relative to the same object,
            // and an object whose `+0x10` is still the identity the constructor left there
            // projects world coordinates straight through and lands the player thousands of
            // pixels away. Indistinguishable, from one bit of output, from having no camera.
            //
            // So the view offset is searched too, bounded to the object's own head, and the
            // on-screen test adjudicates. Twenty extra reads, once per session.
            for view_offset in (0..VIEW_SEARCH_SPAN).step_by(MATRIX_ALIGNMENT) {
                if view_offset == candidate.projection {
                    continue;
                }
                let Some(camera) = pair(candidate.object, view_offset, projection) else {
                    continue;
                };
                if !camera.plausibly_on_screen(local, screen) {
                    continue;
                }
                let winner = Candidate {
                    view: view_offset,
                    ..candidate
                };
                self.last_probe = probe;
                self.remembered = Some(winner);
                return Some((camera, Found::Chose(winner)));
            }
        }
        self.last_probe = probe;
        None
    }

    /// Forget the remembered candidate. Called when the overlay is switched off, so a session
    /// that changes maps does not carry a stale object across the gap.
    pub(crate) fn forget(&mut self) {
        self.remembered = None;
    }
}
