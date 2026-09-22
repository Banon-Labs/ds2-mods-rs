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
//! The same discipline as `crate::camera`, adapted to a COMBINED matrix. A constant buffer holds
//! many things that are sixteen floats, so recognising one is the whole problem:
//!
//! 1. The local player -- whose position is read independently, from the game's own character
//!    accessor -- must project near the middle of the frame. A third-person camera follows the
//!    character; that is a fact about this game rather than about projection.
//! 2. A point at head height above the player must project ABOVE them, by a number of pixels
//!    that looks like a character rather than like a map.
//!
//! The second is the scale test, and it is the one that catches a matrix which merely puts
//! everything near the principal point -- the failure that cost several live runs before it was
//! named.
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

/// Smallest buffer worth scanning, in bytes. A matrix is sixty-four.
const MIN_BUFFER: usize = 64;

/// Largest buffer worth scanning, in bytes.
///
/// Constant buffers are small; a megabyte of mapped vertex data is not where a camera lives, and
/// walking it every frame inside the renderer would cost more than the overlay is worth.
const MAX_BUFFER: usize = 64 * 1024;

/// The matrix the renderer last uploaded, once one has been recognised.
static FOUND: Mutex<Option<Matrix>> = Mutex::new(None);

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

/// What the game supplies so a candidate can be judged. Written by the frame path, read by the
/// `Unmap` detour, which runs on the same thread.
static SUBJECT: Mutex<Option<Subject>> = Mutex::new(None);

/// The known point a candidate matrix is tested against.
#[derive(Clone, Copy, Debug)]
struct Subject {
    /// The local player's world position.
    player: [f32; 3],
    /// The back buffer's size in pixels.
    screen: [f32; 2],
}

/// Tell the capture what to judge candidates against.
///
/// Called from the frame path once a character and a back buffer exist. Without it nothing is
/// scanned at all, which is deliberate: before there is a player there is no way to tell a
/// camera from any other sixteen floats, and guessing is what this whole module exists to avoid.
pub(crate) fn set_subject(player: [f32; 3], screen: [f32; 2]) {
    // Goes on being told the subject for the life of the process, not only until a matrix is
    // first recognised: every later frame's upload is judged against it too.
    let frame = FRAME.fetch_add(1, Ordering::Relaxed) + 1;
    if let Ok(mut subject) = SUBJECT.try_lock() {
        *subject = Some(Subject { player, screen });
    }
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
///
/// Both tests use `subject`, whose player position came from the game's own character data and
/// never from anything in this module -- so there is nothing for a coincidence to hang on.
fn recognises(candidate: &Matrix, subject: Subject) -> bool {
    if !is_finite(candidate) {
        return false;
    }
    let camera = Camera {
        view: IDENTITY,
        view_projection: *candidate,
    };
    let Some(here) = camera.project(subject.player, subject.screen) else {
        return false;
    };
    // The character is near the middle of the frame, because the camera follows them.
    const CENTRE_TOLERANCE: f32 = 0.30;
    let offset_x = (here[0] - subject.screen[0] * 0.5).abs() / (subject.screen[0] * 0.5);
    let offset_y = (here[1] - subject.screen[1] * 0.5).abs() / (subject.screen[1] * 0.5);
    if offset_x > CENTRE_TOLERANCE || offset_y > CENTRE_TOLERANCE {
        return false;
    }
    // THE SCALE TEST. A matrix that maps the whole world to a cluster around the principal point
    // satisfies the one above trivially, and several live runs were lost to exactly that. A
    // character's head must be a character's height above their feet on screen: tens of pixels,
    // not one and not a thousand.
    const HEAD_METERS: f32 = 1.8;
    let head = [
        subject.player[0],
        subject.player[1] + HEAD_METERS,
        subject.player[2],
    ];
    let Some(above) = camera.project(head, subject.screen) else {
        return false;
    };
    let rise = here[1] - above[1];
    rise >= subject.screen[1] * 0.02 && rise <= subject.screen[1] * 0.6
}

/// Walk a mapped buffer looking for the matrix.
///
/// Sixteen-byte steps, because a constant buffer's contents are register-aligned and a matrix
/// never starts off one.
fn scan(at: usize, len: usize, subject: Subject) -> Option<(usize, bool, Matrix)> {
    if len < MIN_BUFFER {
        return None;
    }
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
        if recognises(&candidate, subject) {
            return Some((offset, false, candidate));
        }
        let flipped = crate::geometry::transpose(&candidate);
        if recognises(&flipped, subject) {
            return Some((offset, true, flipped));
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
fn refresh(at: usize, len: Extent, subject: Option<Subject>) -> bool {
    if !HAVE.load(Ordering::Relaxed) {
        return false;
    }
    let Some(subject) = subject else {
        return false;
    };
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
    if !recognises(&candidate, subject) {
        return false;
    }
    if let Ok(mut slot) = FOUND.try_lock() {
        *slot = Some(candidate);
        CONFIRMED_FRAME.store(FRAME.load(Ordering::Relaxed), Ordering::Relaxed);
    }
    true
}

/// Scan an upload for the matrix and, on a hit, remember where it was so [`refresh`] can take
/// over from the next frame onwards.
fn acquire(at: usize, len: Extent, how: &str) {
    if HAVE.load(Ordering::Relaxed) {
        return;
    }
    let Some(subject) = SUBJECT.try_lock().ok().and_then(|held| *held) else {
        return;
    };
    let Some((offset, transposed, found)) = scan(at, len.unwrap_or(MIN_BUFFER), subject) else {
        return;
    };
    let Ok(mut slot) = FOUND.try_lock() else {
        return;
    };
    *slot = Some(found);
    TARGET_OFFSET.store(offset, Ordering::Relaxed);
    TARGET_TRANSPOSED.store(transposed, Ordering::Relaxed);
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
            "camera: captured from {how} at +0x{offset:x} transposed={transposed} -- re-read from there every frame"
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
        let len = Some(MAPPED_LEN.load(Ordering::Relaxed));
        let subject = SUBJECT.try_lock().ok().and_then(|held| *held);
        if !refresh(at, len, subject) {
            acquire(at, len, "a Map/Unmap upload");
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
        // `row_pitch` is zero for a buffer, where the whole update is one contiguous run of an
        // unstated length -- sixty-four bytes is all a matrix needs and is always readable if
        // the caller passed a buffer at all.
        let len = if row_pitch == 0 {
            None
        } else {
            Some((row_pitch as usize).min(MAX_BUFFER))
        };
        if len.is_none_or(|len| len >= MIN_BUFFER) {
            let subject = SUBJECT.try_lock().ok().and_then(|held| *held);
            if !refresh(source as usize, len, subject) {
                acquire(source as usize, len, "an UpdateSubresource");
            }
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
