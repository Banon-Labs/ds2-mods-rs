//! Catching the view-projection matrix as the renderer uploads it.
//!
//! # Why searching memory was the wrong technique
//!
//! `crate::camera` looks for the matrix where the engine might have filed it: every aligned
//! offset in every camera object reachable from `CameraManager`, read as a matrix in both
//! conventions, and also *built* from a unit quaternion paired against every candidate position.
//! Five independent oracles gate the result. Live, nothing passed them -- and the oracles are
//! sound, because each one caught a wrong candidate that the others had let through.
//!
//! The conclusion from that is not "search harder". It is that the matrix the renderer uses is
//! not reachable from `CameraManager` at all, and no amount of scanning the wrong object finds
//! it.
//!
//! So this stops looking for where it is KEPT and watches where it GOES. Every frame the engine
//! writes a view-projection into a Direct3D constant buffer for its own shaders. That upload is
//! a `Map`/`Unmap` pair on the immediate context, and between the two calls the bytes are
//! complete and the pointer is still valid. Detour both, read the buffer at `Unmap`, and the
//! matrix arrives without anyone having guessed a single offset.
//!
//! # What makes a candidate the right one
//!
//! A constant buffer holds many things that are sixteen floats, so recognising one is the whole
//! problem -- and the answer is a property of the sixteen floats, with nothing supplied from
//! outside them. Write the four world columns of `VP = V * P`: they are a rotation's columns,
//! scaled. So three of them are mutually perpendicular, the forward one has length one, the third
//! is parallel to it, and the ratio of the first two is a screen's aspect. See
//! `crate::geometry::is_an_uploaded_view_projection`.
//!
//! What stood here before was "the local player must project near the middle of the frame" plus a
//! head-height scale test. Both read the player, and the first is false about this game: a DARK
//! SOULS II character is not centred, and the offset grows while they move. Measured 2026-09-24,
//! zero acquisitions across a whole session.
//!
//! None of this was derived here. `scripts/frida/arrow.js` is the probe that found the matrix,
//! and its comments carry the measurements -- 46 distinct destination resources, exactly one
//! survivor, 955 consecutive looks, offset `0x60`, read transposed. When this file disagrees with
//! that file, this file is wrong.
//!
//! # Finding it once is not the job
//!
//! The version that first worked stored the matrix it recognised and stopped looking. That draws
//! a correct arrow for exactly one frame: the renderer overwrites the camera every frame, and an
//! overlay holding the old one aims with wherever the camera stood when the game first had a
//! character on screen. On screen it reads as a line that starts somewhere near the player and
//! then, as soon as they turn, floats off into the sky on its own -- which is what the second
//! screenshot of this feature showed.
//!
//! So what is remembered is the PLACE -- the byte offset into an upload, and whether it needed
//! transposing -- and every later upload is re-read there ([`refresh`]).
//!
//! # The buffer is not the thing either
//!
//! The first version of that remembered the buffer as well, and re-read only that one. DARK
//! SOULS II rotates its constant buffers: live, the log filled with the capture letting go and
//! re-scanning, and the arrow blinked out every second or two while it did. Which resource an
//! upload lands in is not a property of the camera.
//!
//! So resource identity is dropped, and what stands in for it is the recognition test itself,
//! applied to every upload at the acquired offset. An upload that does not frame the character is
//! not this frame's camera, whatever buffer it arrived in.
//!
//! # Cost
//!
//! `Map` and `Unmap` are called constantly, so the detours do as little as possible. Before
//! acquisition, only buffers in the size range a constant buffer occupies are scanned at all;
//! after it, each upload costs a sixty-four byte read and two projections -- no search, and no
//! allocation.

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use ds2_hook::{MH_ApplyQueued, MhHook};
use windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext;
use windows::core::Interface;

use crate::geometry::{Camera, Matrix, is_finite};
use crate::log::log;

/// Slot of `ID3D11DeviceContext::Map` in its vtable.
///
/// `IUnknown` contributes three, `ID3D11DeviceChild` four more (`GetDevice`, `GetPrivateData`,
/// `SetPrivateData`, `SetPrivateDataInterface`), then `VSSetConstantBuffers`,
/// `PSSetShaderResources`, `PSSetShader`, `PSSetSamplers`, `VSSetShader`, `DrawIndexed` and
/// `Draw` -- fourteen before `Map`.
const MAP_SLOT: usize = 14;

/// Slot of `ID3D11DeviceContext::Unmap`, immediately after [`MAP_SLOT`].
const UNMAP_SLOT: usize = 15;

/// Slot of `ID3D11DeviceContext::UpdateSubresource`.
///
/// THE OTHER WAY A CONSTANT BUFFER IS FILLED, and for a `DEFAULT`-usage buffer it is the only
/// way -- `Map` requires `DYNAMIC`. An engine that never calls `Map` on its camera buffer is
/// invisible to the `Map`/`Unmap` pair above, which is what a live run that installed the
/// capture and then recognised nothing looks like.
///
/// Counted from the start of the vtable: three `IUnknown`, four `ID3D11DeviceChild`, then
/// forty-one `ID3D11DeviceContext` methods before it.
const UPDATE_SUBRESOURCE_SLOT: usize = 48;

/// Frames the latched offset may go unconfirmed before uploads are scanned again.
///
/// Two, not sixty. Sixty was the staleness check in [`arm`], and using it as the only way
/// back meant a second of a dark overlay every time the camera changed where it arrives. Two
/// frames is long enough that an ordinary frame -- where `refresh` succeeds and stamps
/// `CONFIRMED_FRAME` -- never reaches the scan, and short enough that a buffer rotation costs a
/// blink rather than a second.
const RESCAN_AFTER_FRAMES: usize = 2;

/// Smallest buffer worth scanning, in bytes. A matrix is sixty-four.
const MIN_BUFFER: usize = 64;

/// How far into an upload of unstated extent to look, in bytes.
///
/// Taken from `scripts/frida/arrow.js`, which found this game's view-projection at offset `0x60`
/// of one: a constant buffer holding a camera is small, and this covers the usual 256-byte
/// register window twice over without reading into another allocation.
const SCAN_BYTES: usize = 512;

/// Largest buffer worth scanning, in bytes.
///
/// Constant buffers are small; a megabyte of mapped vertex data is not where a camera lives, and
/// walking it every frame inside the renderer would cost more than the overlay is worth.
const MAX_BUFFER: usize = 64 * 1024;

/// The matrix the renderer last uploaded, once one has been recognised.
static FOUND: Mutex<Option<Matrix>> = Mutex::new(None);

/// Uploads the detours saw, uploads actually walked, sixteen-float windows tested, and windows
/// that passed the shape test.
///
/// An instrument that cannot tell "found nothing" from "never ran" is worse than no instrument --
/// `scripts/frida/arrow.js` makes the point twice and pays for it twice. This file has now spent
/// two sessions reporting a dark overlay with nothing to say about why, and the answer both times
/// was upstream of the test: first the detour was rejecting every upload on a `srcRowPitch` that
/// is meaningless for a buffer, then the scan window was sixty-four bytes wide when the matrix
/// sits at `0x60`. Neither is visible in "no camera". `1.2M uploads, 0 walked` and
/// `1.2M uploads, 1.2M walked, 38M windows, 0 shaped` are different faults with different fixes,
/// and both read as silence without these four numbers.
static UPLOADS: AtomicUsize = AtomicUsize::new(0);
static WALKED: AtomicUsize = AtomicUsize::new(0);
static WINDOWS: AtomicUsize = AtomicUsize::new(0);
static SHAPED: AtomicUsize = AtomicUsize::new(0);

/// What the capture has tried, for the frame path's `no camera` line.
pub(crate) fn probe() -> Probe {
    Probe {
        uploads: UPLOADS.load(Ordering::Relaxed),
        walked: WALKED.load(Ordering::Relaxed),
        windows: WINDOWS.load(Ordering::Relaxed),
        shaped: SHAPED.load(Ordering::Relaxed),
    }
}

/// The counts behind [`probe`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Probe {
    pub uploads: usize,
    pub walked: usize,
    pub windows: usize,
    pub shaped: usize,
}

/// Set once a matrix has ever been recognised, so acquisition stops and the hot path is one
/// relaxed load. NOT a licence to stop reading: see [`refresh`].
static HAVE: AtomicBool = AtomicBool::new(false);

/// WHERE in an upload the matrix sits -- the byte offset, and whether it had to be transposed to
/// read as row-major.
///
/// The offset and NOT the buffer. Which resource an upload lands in is not a property of the
/// camera: DARK SOULS II rotates its constant buffers, so a capture keyed to the buffer it first
/// saw loses the camera within seconds. See [`refresh`].
static TARGET_OFFSET: AtomicUsize = AtomicUsize::new(0);
static TARGET_TRANSPOSED: AtomicBool = AtomicBool::new(false);

/// Grounded passes the latched place has produced since it was latched.
///
/// A place with fewer than [`GROUNDED_TO_BELIEVE`] of these is being watched, not believed: it is
/// re-read every upload and counted, and the draw path is told nothing.
static PASSES: AtomicUsize = AtomicUsize::new(0);

/// Grounded passes before a place is believed, taken from `scripts/frida/arrow.js`.
///
/// Sixty, which at the rate the camera's buffer comes round is about a second of unbroken
/// agreement. See [`refresh`] for what it is buying.
const GROUNDED_TO_BELIEVE: usize = 60;

/// The pointer the last `Map` handed out and how many bytes it covers.
///
/// A single slot rather than a table keyed by resource: `Map`/`Unmap` on the immediate context
/// are not interleaved -- the context is single-threaded by contract -- so the last `Map` is the
/// one `Unmap` is closing.
static MAPPED_AT: AtomicUsize = AtomicUsize::new(0);
static MAPPED_LEN: AtomicUsize = AtomicUsize::new(0);

/// Frames drawn, and the frame in which an upload last carried a matrix that framed the
/// character. Their difference is how long the capture has been getting nothing it recognises.
static FRAME: AtomicUsize = AtomicUsize::new(0);
static CONFIRMED_FRAME: AtomicUsize = AtomicUsize::new(usize::MAX);

/// Trampolines back to the real functions.
static ORIGINAL_MAP: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_UNMAP: AtomicUsize = AtomicUsize::new(0);

/// Where the local character is standing, as of the last frame, and whether anyone is.
///
/// Read by the detours to tell a view-projection from a bare projection. The back buffer's
/// extent used to live here too and does not any more: the aspect test is a band rather than a
/// match against the presented size.
static STANDING: Mutex<Option<[f32; 3]>> = Mutex::new(None);

/// Whether the frame path has reached the point where a camera is worth hunting for.
static ARMED: AtomicBool = AtomicBool::new(false);

/// Where the character stood on the last frame, if the detour can get at it without blocking.
///
/// `try_lock` rather than `lock`, because this runs inside the renderer: an upload that cannot
/// read the position is one upload not judged, which the next one fixes. A stalled render thread
/// is not something the next frame fixes.
fn standing() -> Option<[f32; 3]> {
    *STANDING.try_lock().ok()?
}

/// Arm the capture, tell it where the character is, and tick the frame counter.
///
/// Called from the frame path once a character and a back buffer exist.
pub(crate) fn arm(player: [f32; 3]) {
    if let Ok(mut standing) = STANDING.try_lock() {
        *standing = Some(player);
    }
    let frame = FRAME.fetch_add(1, Ordering::Relaxed) + 1;
    ARMED.store(true, Ordering::Relaxed);
    // GO AND LOOK AGAIN WHEN THE OFFSET STOPS PAYING OUT. Nothing guarantees the camera keeps
    // arriving at the byte offset it was acquired at -- a map change rebuilds the renderer's
    // buffers, and a different pass may lay its constants out differently. Without this the
    // capture would go on reading an offset that no longer holds a camera and the overlay would
    // stay dark for the rest of the session with nothing to say about why. A second of no upload
    // that frames the character is the signal.
    const STALE_FRAMES: usize = 60;
    if HAVE.load(Ordering::Relaxed) {
        let confirmed = CONFIRMED_FRAME.load(Ordering::Relaxed);
        if confirmed <= frame && frame - confirmed > STALE_FRAMES {
            HAVE.store(false, Ordering::Relaxed);
            // The passes belonged to the place, and the place is gone. Leaving them standing
            // would let the next place inherit a believed count it never earned.
            PASSES.store(0, Ordering::Relaxed);
            // And stop drawing with the last matrix that place produced. A second of no upload
            // framing the character means the camera is not arriving there any more; what is
            // held is a stale view of a world that has moved, and drawing through it is the
            // line-across-the-sky failure in a slower form.
            if let Ok(mut slot) = FOUND.try_lock() {
                *slot = None;
            }
            if RELATCH_REPORTS
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                    left.checked_sub(1)
                })
                .is_ok()
            {
                log(format_args!(
                    "camera: nothing has framed the character for a second -- looking for it again"
                ));
            }
        }
    }
}

/// How many re-scan lines still get written, so a session that cannot find a camera at all
/// cannot fill the log with the same sentence once a second.
static RELATCH_REPORTS: AtomicUsize = AtomicUsize::new(8);

/// How many acquisition lines still get written, for the same reason.
static ACQUIRE_REPORTS: AtomicUsize = AtomicUsize::new(8);

/// The captured camera, as of the most recent upload.
pub(crate) fn camera() -> Option<Camera> {
    let view_projection = (*FOUND.try_lock().ok()?)?;
    // The view half is not recoverable from a combined matrix, and nothing in the draw path
    // needs it except the arrowhead's opening direction -- which falls back to world up.
    Some(Camera {
        view: IDENTITY,
        view_projection,
    })
}

/// A row-major identity, for the `view` half of a captured camera.
const IDENTITY: Matrix = [
    1.0, 0.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, 0.0, //
    0.0, 0.0, 1.0, 0.0, //
    0.0, 0.0, 0.0, 1.0,
];

/// Is this sixteen floats the matrix the frame was drawn with?
fn recognises(candidate: &Matrix) -> bool {
    if !is_finite(candidate) {
        return false;
    }
    // THE MATRIX ALONE DECIDES, and the two tests that used to stand here are gone because they
    // are the reason nothing was ever recognised.
    //
    // They were "the character projects near the middle of the frame" and "their head projects a
    // character's height above their feet". Both read the player, and the first is false about
    // this game: a DARK SOULS II character is NOT centred, and the offset grows while they move,
    // so a tight bound rejects the real camera on ordinary frames while admitting dense parameter
    // blobs that happen to land near the principal point. Measured 2026-09-24: zero acquisitions
    // across a whole session, the overlay falling through to a memory search, and a green line
    // drawn across the sky touching neither the player nor the target.
    //
    // What replaces them is a property of the sixteen floats and of nothing else -- mutually
    // perpendicular scaled rotation columns, a unit forward axis, and a column-length ratio in
    // the band that screens occupy. See `geometry::is_an_uploaded_view_projection` for the
    // derivation, and `scripts/frida/arrow.js` for the run that established it.
    crate::geometry::is_an_uploaded_view_projection(candidate)
}

/// Walk a mapped buffer looking for the matrix.
///
/// Sixteen-byte steps, because a constant buffer's contents are register-aligned and a matrix
/// never starts off one. `0x60`, where this game's camera lands, is on that stride.
fn scan(at: usize, len: usize, player: [f32; 3]) -> Option<(usize, bool, Matrix)> {
    if len < MIN_BUFFER {
        return None;
    }
    WALKED.fetch_add(1, Ordering::Relaxed);
    let mut offset = 0usize;
    while offset + 64 <= len {
        let mut candidate = [0.0f32; 16];
        // SAFETY: `at` is the pointer Direct3D returned from `Map` and `len` the extent it
        // reported; `offset + 64` is inside it by the loop condition, and the mapping is still
        // live because this runs inside `Unmap` before the original is called.
        unsafe {
            core::ptr::copy_nonoverlapping((at + offset) as *const f32, candidate.as_mut_ptr(), 16);
        }
        // BOTH ORIENTATIONS, and the transposed one is the likelier of the two. HLSL's default
        // packing for `float4x4` is COLUMN-major, so an engine whose own maths is row-vector
        // uploads the transpose of what it computed. A constant buffer is therefore the one
        // place where the transposed reading is the expected one rather than the fallback.
        // THE SHAPE TEST DOES NOT GET TO DECIDE ALONE, and the walk does not stop when it passes.
        //
        // This returned on the first structural match. The first structural match in an upload is
        // the bare projection at `+0x0` -- see `grounds` -- so the scan stopped there, latched it,
        // and never reached the view-projection at `+0x60`. Requiring a grounded pass here is
        // what lets the walk go past a matrix that is shaped like a camera and has none in it.
        WINDOWS.fetch_add(1, Ordering::Relaxed);
        if recognises(&candidate) {
            SHAPED.fetch_add(1, Ordering::Relaxed);
            if crate::geometry::grounds_a_camera(&candidate, player) {
                return Some((offset, false, candidate));
            }
        }
        let flipped = crate::geometry::transpose(&candidate);
        if recognises(&flipped) {
            SHAPED.fetch_add(1, Ordering::Relaxed);
            if crate::geometry::grounds_a_camera(&flipped, player) {
                return Some((offset, true, flipped));
            }
        }
        offset += 16;
    }
    None
}

/// How much of an upload is readable. `None` is a buffer whose extent the caller did not state.
type Extent = Option<usize>;

/// Re-read the matrix from the place a previous frame found it.
///
/// THE POINT OF THE MODULE. The renderer writes a fresh view-projection into the same buffer
/// every frame; what is worth remembering is the address, not the sixteen floats that were at it
/// once. Re-reading costs a pointer compare on every upload that is not the camera's, and two
/// projections on the one that is.
///
/// `true` means this upload was the latched one -- handled, whether or not it was stored -- so
/// the caller does not fall through to a full scan.
fn refresh(at: usize, len: Extent, player: [f32; 3]) -> bool {
    if !HAVE.load(Ordering::Relaxed) {
        return false;
    }
    let offset = TARGET_OFFSET.load(Ordering::Relaxed);
    // An unstated extent is trusted at the offset acquisition already read: `UpdateSubresource`
    // on a buffer states no length, and the scan that found the matrix read the same bytes.
    if len.is_some_and(|len| offset + 64 > len) {
        return false;
    }
    let mut candidate = [0.0f32; 16];
    // SAFETY: `at` is the pointer the caller is about to have Direct3D read from, and `offset`
    // is inside it -- checked above when the extent is known, and no further than the scan that
    // acquired the matrix reads when it is not.
    unsafe {
        core::ptr::copy_nonoverlapping((at + offset) as *const f32, candidate.as_mut_ptr(), 16);
    }
    if TARGET_TRANSPOSED.load(Ordering::Relaxed) {
        candidate = crate::geometry::transpose(&candidate);
    }
    // THE BUFFER IS NOT THE THING; THE MATRIX IS. An earlier version keyed this to the resource
    // the matrix was first seen in and re-read only that one. Live, DARK SOULS II rotates its
    // constant buffers: the log filled with `stopped carrying it -- looking again` and the arrow
    // blinked out every second or two while the re-scan ran. The buffer an upload lands in is
    // not a property of the camera.
    //
    // So identity is dropped and the test is applied instead -- every upload, at the acquired
    // offset. It is the same test acquisition used and it costs two projections, which is
    // nothing next to a scan. An upload that does not frame the character is simply not this
    // frame's camera, whatever buffer it arrived in, and the previous matrix stands.
    if !recognises(&candidate) || !crate::geometry::grounds_a_camera(&candidate, player) {
        return false;
    }
    CONFIRMED_FRAME.store(FRAME.load(Ordering::Relaxed), Ordering::Relaxed);
    // A PLACE HAS TO EARN THE DRAW PATH, and one upload is not enough to earn it.
    //
    // `scripts/frida/arrow.js` waits for sixty grounded passes at a single offset before it
    // believes one, and its confirmation line is `60 of 60 consecutive frames had a character in
    // them`. The reason is not caution for its own sake: over a kilobyte of constant buffer at
    // tens of thousands of uploads a second, some window somewhere will be shaped like a camera
    // AND put the character in front of it, once, by coincidence. Sixty consecutive cannot be a
    // coincidence, and it costs a second of a dark overlay at the start of a session.
    let passes = PASSES.fetch_add(1, Ordering::Relaxed) + 1;
    if passes < GROUNDED_TO_BELIEVE {
        return true;
    }
    if let Ok(mut slot) = FOUND.try_lock() {
        let first = slot.is_none();
        *slot = Some(candidate);
        if first {
            let transposed = TARGET_TRANSPOSED.load(Ordering::Relaxed);
            log(format_args!(
                "camera: believed at +0x{offset:x} transposed={transposed} after {passes} \
                 grounded pass(es) -- re-read from there every frame"
            ));
        }
    }
    true
}

/// Scan an upload for the matrix and, on a hit, remember where it was so [`refresh`] can take
/// over from the next frame onwards.
fn acquire(at: usize, len: Extent, how: &str, player: [f32; 3]) {
    // RE-SCANNING WHILE HOLDING ONE, which this refused to do and which cost the whole overlay.
    //
    // The refusal was `if HAVE { return }`: once an offset was latched, no upload was ever
    // scanned again until `arm`'s staleness check gave up a second later and cleared
    // `HAVE`. So the moment the camera arrived at a DIFFERENT offset -- a map change, a pass
    // that lays its constants out another way, the buffer rotation this file already knows DARK
    // SOULS II does -- the capture had a whole second of blindness before it would even look.
    //
    // Measured 2026-09-24: three acquisitions and three losses across one session, with the
    // overlay spending nearly all of the run with no captured matrix at all. The offset is no
    // more a property of the camera than the resource is; this file learned that about the
    // buffer and did not carry it across to the offset.
    //
    // So a latch is held only while it keeps paying out. `refresh` succeeding stamps
    // `CONFIRMED_FRAME`; if that stamp is more than [`RESCAN_AFTER_FRAMES`] old, this upload is
    // scanned like any other and a better offset simply replaces the stale one. The fast path is
    // untouched -- while the camera keeps arriving where it was found, `refresh` answers first
    // and this returns on the frame comparison alone.
    if HAVE.load(Ordering::Relaxed) {
        let frame = FRAME.load(Ordering::Relaxed);
        let confirmed = CONFIRMED_FRAME.load(Ordering::Relaxed);
        if confirmed > frame || frame - confirmed < RESCAN_AFTER_FRAMES {
            return;
        }
    }
    if !ARMED.load(Ordering::Relaxed) {
        return;
    }
    // SIXTY-FOUR BYTES WAS THE BUG, and it was the whole of it.
    //
    // `UpdateSubresource` on a buffer states no extent -- `srcRowPitch` is ignored for buffers
    // and this game passes zero -- so `len` arrives as `None` on every upload the camera travels
    // in. The fallback was `MIN_BUFFER`, sixty-four bytes, which makes the scan loop's condition
    // `offset + 64 <= 64`: exactly one window, at offset zero. This game's view-projection is at
    // offset `0x60`. The scan was structurally incapable of reaching it, and had been for every
    // run that reported zero acquisitions while the log insisted the detour was firing.
    //
    // `scripts/frida/arrow.js`, which is where this matrix was found in the first place, reads
    // `SCAN_BYTES = 512` for precisely this case, and says why: a constant buffer holding a
    // camera is small, and five hundred and twelve bytes covers the usual 256-byte register
    // window twice over without reading into another allocation.
    let Some((offset, transposed, _)) = scan(at, len.unwrap_or(SCAN_BYTES), player) else {
        return;
    };
    // A NEW PLACE STARTS AT ZERO AND THE DRAW PATH IS TOLD NOTHING.
    //
    // This stored the matrix straight into `FOUND`, so the very first structural match drew. On
    // 2026-09-24 that was the bare projection at `+0x0` and the overlay put a green line across
    // the sky with it. What a scan produces now is a place to WATCH: `refresh` reads it on every
    // later upload, counts the grounded passes, and hands it to the draw path only once there
    // have been [`GROUNDED_TO_BELIEVE`] of them.
    TARGET_OFFSET.store(offset, Ordering::Relaxed);
    TARGET_TRANSPOSED.store(transposed, Ordering::Relaxed);
    PASSES.store(0, Ordering::Relaxed);
    CONFIRMED_FRAME.store(FRAME.load(Ordering::Relaxed), Ordering::Relaxed);
    HAVE.store(true, Ordering::Relaxed);
    // Budgeted for the same reason as the re-latch line: acquisition can happen more than once.
    if ACQUIRE_REPORTS
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
            left.checked_sub(1)
        })
        .is_ok()
    {
        log(format_args!(
            "camera: watching {how} at +0x{offset:x} transposed={transposed} -- needs \
             {GROUNDED_TO_BELIEVE} grounded pass(es) before anything is drawn with it"
        ));
    }
}

/// `ID3D11DeviceContext::Map`, as the detour must declare it.
type MapFn = unsafe extern "system" fn(
    *mut c_void,
    *mut c_void,
    u32,
    i32,
    u32,
    *mut MappedSubresource,
) -> windows::core::HRESULT;

/// `ID3D11DeviceContext::Unmap`.
type UnmapFn = unsafe extern "system" fn(*mut c_void, *mut c_void, u32);

/// `D3D11_MAPPED_SUBRESOURCE`, declared here so the detour does not depend on the generated
/// binding's field order for a struct Direct3D fills in.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct MappedSubresource {
    data: *mut c_void,
    row_pitch: u32,
    depth_pitch: u32,
}

/// Our `Map`: remember what was handed out, then get out of the way.
///
/// # Safety
///
/// Installed by MinHook over `ID3D11DeviceContext::Map`, whose ABI this matches.
unsafe extern "system" fn map(
    context: *mut c_void,
    resource: *mut c_void,
    subresource: u32,
    map_type: i32,
    flags: u32,
    mapped: *mut MappedSubresource,
) -> windows::core::HRESULT {
    let original = ORIGINAL_MAP.load(Ordering::Acquire);
    if original == 0 {
        return windows::core::HRESULT(0);
    }
    // SAFETY: the trampoline MinHook produced, called with this detour's own arguments.
    let result = unsafe {
        let real: MapFn = core::mem::transmute(original);
        real(context, resource, subresource, map_type, flags, mapped)
    };
    if mapped.is_null() || result.is_err() {
        return result;
    }
    // SAFETY: Direct3D filled `mapped` on success, which `result` has just confirmed.
    let filled = unsafe { *mapped };
    let len = filled.row_pitch as usize;
    if filled.data.is_null() || !(MIN_BUFFER..=MAX_BUFFER).contains(&len) {
        MAPPED_AT.store(0, Ordering::Relaxed);
        return result;
    }
    MAPPED_AT.store(filled.data as usize, Ordering::Relaxed);
    MAPPED_LEN.store(len, Ordering::Relaxed);
    result
}

/// Our `Unmap`: read what was written, then get out of the way.
///
/// # Safety
///
/// Installed by MinHook over `ID3D11DeviceContext::Unmap`, whose ABI this matches.
unsafe extern "system" fn unmap(context: *mut c_void, resource: *mut c_void, subresource: u32) {
    let at = MAPPED_AT.swap(0, Ordering::Relaxed);
    if at != 0 {
        UPLOADS.fetch_add(1, Ordering::Relaxed);
        let len = Some(MAPPED_LEN.load(Ordering::Relaxed));
        if let Some(player) = standing()
            && !refresh(at, len, player)
        {
            acquire(at, len, "a Map/Unmap upload", player);
        }
    }
    let original = ORIGINAL_UNMAP.load(Ordering::Acquire);
    if original == 0 {
        return;
    }
    // SAFETY: the trampoline MinHook produced, called with this detour's own arguments.
    unsafe {
        let real: UnmapFn = core::mem::transmute(original);
        real(context, resource, subresource);
    }
}

/// `ID3D11DeviceContext::UpdateSubresource`.
type UpdateSubresourceFn = unsafe extern "system" fn(
    *mut c_void,
    *mut c_void,
    u32,
    *const c_void,
    *const c_void,
    u32,
    u32,
);

/// Trampoline back to the real `UpdateSubresource`.
static ORIGINAL_UPDATE: AtomicUsize = AtomicUsize::new(0);

/// Our `UpdateSubresource`: read what is about to be written, then get out of the way.
///
/// Unlike the `Map`/`Unmap` pair this needs no bookkeeping -- the caller hands over a pointer to
/// the finished bytes, so the data is right there.
///
/// # Safety
///
/// Installed by MinHook over `ID3D11DeviceContext::UpdateSubresource`, whose ABI this matches.
unsafe extern "system" fn update_subresource(
    context: *mut c_void,
    resource: *mut c_void,
    subresource: u32,
    box_: *const c_void,
    source: *const c_void,
    row_pitch: u32,
    depth_pitch: u32,
) {
    if !source.is_null() {
        UPLOADS.fetch_add(1, Ordering::Relaxed);
        // `row_pitch` is zero for a buffer, where the whole update is one contiguous run of an
        // unstated length. It is not a size and it never was: `srcRowPitch` is ignored for buffer
        // resources and this game passes zero on every one of them. The first Frida attempt at
        // this read 4,102,421 calls and scanned 0 buffers, every one rejected on that argument.
        let len = if row_pitch == 0 {
            None
        } else {
            Some((row_pitch as usize).min(MAX_BUFFER))
        };
        if len.is_none_or(|len| len >= MIN_BUFFER)
            && let Some(player) = standing()
            && !refresh(source as usize, len, player)
        {
            acquire(source as usize, len, "an UpdateSubresource", player);
        }
    }
    let original = ORIGINAL_UPDATE.load(Ordering::Acquire);
    if original == 0 {
        return;
    }
    // SAFETY: the trampoline MinHook produced, called with this detour's own arguments.
    unsafe {
        let real: UpdateSubresourceFn = core::mem::transmute(original);
        real(
            context,
            resource,
            subresource,
            box_,
            source,
            row_pitch,
            depth_pitch,
        );
    }
}

/// Install the two detours on `context`'s vtable. `false` means no capture, already logged.
///
/// # Safety
///
/// Installs native code detours. Call once, with a live immediate context.
pub(crate) unsafe fn install(context: &ID3D11DeviceContext) -> bool {
    if ORIGINAL_MAP.load(Ordering::Acquire) != 0 {
        return true;
    }
    // SAFETY: a live COM object: its first field is a pointer to its vtable.
    let (map_target, unmap_target, update_target) = unsafe {
        let raw = context.as_raw().cast::<*const usize>();
        (
            *(*raw).add(MAP_SLOT),
            *(*raw).add(UNMAP_SLOT),
            *(*raw).add(UPDATE_SUBRESOURCE_SLOT),
        )
    };
    // SAFETY: a vtable entry of the interface above; the detour matches its ABI. Installed
    // separately from the pair below so a failure on one upload path does not disable the other.
    if let Ok(hook) = unsafe {
        MhHook::new(
            update_target as *mut c_void,
            update_subresource as *mut c_void,
        )
    } {
        ORIGINAL_UPDATE.store(hook.trampoline() as usize, Ordering::Release);
        // SAFETY: enabling the hook created immediately above.
        unsafe {
            let _ = hook.queue_enable();
        }
    }
    // SAFETY: both are vtable entries of the interface above, and the detours match their ABIs.
    let hooks = unsafe {
        (
            MhHook::new(map_target as *mut c_void, map as *mut c_void),
            MhHook::new(unmap_target as *mut c_void, unmap as *mut c_void),
        )
    };
    let (Ok(map_hook), Ok(unmap_hook)) = hooks else {
        log(format_args!(
            "camera: could not detour Map/Unmap -- no capture this session"
        ));
        return false;
    };
    ORIGINAL_MAP.store(map_hook.trampoline() as usize, Ordering::Release);
    ORIGINAL_UNMAP.store(unmap_hook.trampoline() as usize, Ordering::Release);
    // SAFETY: enabling the two hooks created immediately above.
    unsafe {
        if map_hook.queue_enable().is_err() || unmap_hook.queue_enable().is_err() {
            return false;
        }
        if MH_ApplyQueued() != ds2_hook::MH_STATUS::MH_OK {
            return false;
        }
    }
    log(format_args!(
        "camera: watching Map/Unmap and UpdateSubresource for the view-projection"
    ));
    true
}
