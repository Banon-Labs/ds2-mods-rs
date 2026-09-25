//! The three detours and their installation.

use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicUsize, Ordering};

use ds2_game_base::mem::{game_module_base, safe_read_u8, safe_read_usize};
use ds2_hook::{MH_ApplyQueued, MH_Initialize, MH_STATUS, MhHook};

use crate::{LOG_PREFIX, Tune, bar_centre_x, bar_pivot, digit_count, text_pivot};

const _: () = assert!(crate::BAR_WIDTH == ds2_rva::HP_GAUGE_BAR_WIDTH);
const _: () = assert!(crate::NUMBER_MAX == ds2_rva::HP_GAUGE_NUMBER_MAX);

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

/// The tuning this run uses. Set once by the loader before [`install`]; [`Tune::default`] if not.
static TUNE: OnceLock<Tune> = OnceLock::new();

/// Use these values instead of the defaults. Call before [`install`]; a second call is ignored.
pub fn set_tune(tune: Tune) {
    let _ = TUNE.set(tune);
}

fn tune() -> &'static Tune {
    TUNE.get_or_init(Tune::default)
}

/// `void(gauge, u32 slot, float4* pos, float scale, float2* offset)`. The scale is the fourth
/// argument and a float, so the Win64 ABI carries it in XMM3 -- declaring it `f32` here is what
/// makes the detour receive and forward it there.
type TransformFn = unsafe extern "system" fn(*mut u8, u32, *const f32, f32, *const f32);

/// `void(gauge, scene, i32 value)`.
type SetNumberFn = unsafe extern "system" fn(*mut u8, usize, i32);

/// `FeLayoutScene::findById`, all eleven arguments. See [`ds2_rva::FE_SCENE_FIND_BY_ID`].
type FindByIdFn =
    unsafe extern "system" fn(usize, u32, u32, u32, u64, u64, u64, u64, u64, u64, u64) -> usize;

static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static BAR_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static TEXT_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static SET_NUMBER_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// Each slot's bar centre x, as `f32` bits. NaN until that slot's bar has been placed.
///
/// The update places a slot's bar and then its number, in that order, so the number always reads
/// the centre its own bar was given moments earlier.
static BAR_CENTRE: [AtomicU32; ds2_rva::HP_GAUGE_SLOTS] =
    [const { AtomicU32::new(0x7fc0_0000) }; ds2_rva::HP_GAUGE_SLOTS];

/// How many entries the digit table has.
///
/// Keyed by scene rather than slot because the setter is not handed a slot. Twenty scenes exist
/// for the gauge's life, so 32 never fills; if it somehow did, the newest scene takes a hashed
/// entry and the worst outcome is a number centred as though it had a different width.
const DIGIT_TABLE: usize = 32;
/// Scene pointer per digit-table entry; zero is free.
static DIGIT_SCENE: [AtomicUsize; DIGIT_TABLE] = [const { AtomicUsize::new(0) }; DIGIT_TABLE];
/// Digits last written for that entry's scene.
static DIGIT_COUNT: [AtomicU8; DIGIT_TABLE] = [const { AtomicU8::new(0) }; DIGIT_TABLE];

fn remember_digits(scene: usize, digits: u8) {
    for (slot, count) in DIGIT_SCENE.iter().zip(&DIGIT_COUNT) {
        match slot.compare_exchange(0, scene, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => {
                count.store(digits, Ordering::Release);
                return;
            }
            Err(existing) if existing == scene => {
                count.store(digits, Ordering::Release);
                return;
            }
            Err(_) => {}
        }
    }
    let index = (scene >> 4) % DIGIT_TABLE;
    DIGIT_SCENE[index].store(scene, Ordering::Release);
    DIGIT_COUNT[index].store(digits, Ordering::Release);
}

fn digits_for(scene: usize) -> u8 {
    DIGIT_SCENE
        .iter()
        .zip(&DIGIT_COUNT)
        .find(|(slot, _)| slot.load(Ordering::Acquire) == scene)
        .map_or(0, |(_, count)| count.load(Ordering::Acquire))
}

/// First-sighting flags, so the log says each thing happened once rather than every frame.
static SEEN_BAR: AtomicBool = AtomicBool::new(false);
static SEEN_TEXT: AtomicBool = AtomicBool::new(false);
static SEEN_NUMBER: AtomicBool = AtomicBool::new(false);
static SEEN_FULL_MATRIX: AtomicBool = AtomicBool::new(false);
static SEEN_REFUSAL: AtomicBool = AtomicBool::new(false);

fn first(flag: &AtomicBool) -> bool {
    !flag.swap(true, Ordering::AcqRel)
}

/// A 16-byte-aligned float4, the shape the game hands the transforms.
#[repr(C, align(16))]
struct Pivot([f32; 4]);

/// Copy the caller's pivot. The game reads `pos[0]` and `pos[1]` only, but the whole float4 is
/// carried so the copy is a faithful stand-in.
///
/// # Safety
///
/// `pos` is the non-null pointer the game passed to the transform, which the original reads
/// unguarded.
unsafe fn read_pos(pos: *const f32) -> Pivot {
    // SAFETY: per the contract, the same four floats the original would read.
    Pivot(unsafe {
        [
            pos.read(),
            pos.add(1).read(),
            pos.add(2).read(),
            pos.add(3).read(),
        ]
    })
}

unsafe extern "system" fn bar_detour(
    gauge: *mut u8,
    slot: u32,
    pos: *const f32,
    scale: f32,
    offset: *const f32,
) {
    let trampoline = BAR_TRAMPOLINE.load(Ordering::Acquire);
    // SAFETY: MinHook published this trampoline for this site, with this signature, before the
    // site was enabled.
    let original: TransformFn = unsafe { std::mem::transmute::<usize, TransformFn>(trampoline) };
    let tune = tune();
    if pos.is_null() {
        // SAFETY: the game's own arguments, forwarded untouched.
        unsafe { original(gauge, slot, pos, scale, offset) };
        return;
    }
    // SAFETY: `pos` is non-null and is the pointer the original is about to read.
    let mut moved = unsafe { read_pos(pos) };
    let [x, y] = bar_pivot([moved.0[0], moved.0[1]], scale, tune);
    moved.0[0] = x;
    moved.0[1] = y;
    if let Some(cell) = BAR_CENTRE.get(slot as usize) {
        cell.store(bar_centre_x(x, scale, tune).to_bits(), Ordering::Release);
    }
    if first(&SEEN_BAR) {
        log(format_args!(
            "{LOG_PREFIX} first bar slot={slot} game-scale={scale} applied={}",
            scale * tune.bar_scale
        ));
    }
    // SAFETY: the game's own gauge, slot and offset; `moved` is a live aligned float4 standing in
    // for the caller's pivot for the duration of the call.
    unsafe {
        original(
            gauge,
            slot,
            moved.0.as_ptr(),
            scale * tune.bar_scale,
            offset,
        )
    };
}

unsafe extern "system" fn text_detour(
    gauge: *mut u8,
    slot: u32,
    pos: *const f32,
    scale: f32,
    offset: *const f32,
) {
    let trampoline = TEXT_TRAMPOLINE.load(Ordering::Acquire);
    // SAFETY: MinHook published this trampoline for this site, with this signature, before the
    // site was enabled.
    let original: TransformFn = unsafe { std::mem::transmute::<usize, TransformFn>(trampoline) };
    let tune = tune();
    let index = slot as usize;
    if pos.is_null() || gauge.is_null() || index >= ds2_rva::HP_GAUGE_SLOTS {
        // SAFETY: the game's own arguments, forwarded untouched. The original refuses the slot.
        unsafe { original(gauge, slot, pos, scale, offset) };
        return;
    }
    let scene_at = gauge as usize + ds2_rva::HP_GAUGE_SCENES_OFFSET + index * 8;
    // SAFETY: fault-tolerant read; the original reads this same slot unguarded.
    let scene = unsafe { safe_read_usize(scene_at) }.unwrap_or(0);
    let centre = f32::from_bits(BAR_CENTRE[index].load(Ordering::Acquire));
    let centre = centre.is_finite().then_some(centre);
    let digits = if scene == 0 { 0 } else { digits_for(scene) };

    // SAFETY: `pos` is non-null and is the pointer the original is about to read.
    let mut moved = unsafe { read_pos(pos) };
    let [x, y] = text_pivot([moved.0[0], moved.0[1]], centre, digits, scale, tune);
    moved.0[0] = x;
    moved.0[1] = y;
    let applied = scale * tune.text_scale;
    if first(&SEEN_TEXT) {
        log(format_args!(
            "{LOG_PREFIX} first number slot={slot} game-scale={scale} applied={applied} \
             digits={digits} centred={}",
            centre.is_some()
        ));
    }
    // SAFETY: as in `bar_detour`.
    unsafe { original(gauge, slot, moved.0.as_ptr(), applied, offset) };

    if scene != 0 {
        // SAFETY: `scene` is the non-null pointer the original just searched with.
        unsafe { apply_full_matrix(scene) };
    }
}

/// Make the number's element apply the whole matrix the original just stored.
///
/// # Safety
///
/// `scene` is a live gauge scene. Every read of the element is fault-tolerant; the two byte writes
/// happen only on an element whose vtable is `FeComponentObject`'s and whose matrix is allocated.
unsafe fn apply_full_matrix(scene: usize) {
    let base = MODULE_BASE.load(Ordering::Acquire);
    // SAFETY: an RVA in this module, resolved against its live base, with the signature
    // `ds2_rva::FE_SCENE_FIND_BY_ID` records.
    let find: FindByIdFn = unsafe {
        std::mem::transmute::<usize, FindByIdFn>(base + ds2_rva::FE_SCENE_FIND_BY_ID as usize)
    };
    let id = ds2_rva::HP_GAUGE_TEXT_ELEMENT;
    // SAFETY: the same call, with the same arguments, the original made a moment ago.
    let element = unsafe { find(scene, id, id, 0, 0, 0, 0, 0, 0, 0, 0) };
    if element == 0 {
        return;
    }
    // SAFETY: fault-tolerant read.
    let vtable = unsafe { safe_read_usize(element) };
    // SAFETY: fault-tolerant read.
    let matrix = unsafe { safe_read_usize(element + ds2_rva::FE_COMPONENT_OBJECT_MATRIX_OFFSET) };
    let expected = base + ds2_rva::FE_COMPONENT_OBJECT_VTABLE as usize;
    if vtable != Some(expected) || matches!(matrix, None | Some(0)) {
        if first(&SEEN_REFUSAL) {
            log(format_args!(
                "{LOG_PREFIX} number left at the game's size: element=0x{element:x} \
                 vtable={vtable:x?} expected=0x{expected:x} matrix={matrix:x?}"
            ));
        }
        return;
    }
    let mask_at = element + ds2_rva::FE_COMPONENT_OBJECT_APPLY_MASK_OFFSET;
    let flag_at = element + ds2_rva::FE_COMPONENT_OBJECT_MATRIX_FLAG_OFFSET;
    // SAFETY: fault-tolerant read, for the log only.
    let mask_before = unsafe { safe_read_u8(mask_at) };
    // SAFETY: the element is an `FeComponentObject` (vtable checked above) with a stored matrix,
    // and these are the two bytes its own `setMatrix` writes when given flag 0.
    unsafe {
        (mask_at as *mut u8).write(ds2_rva::FE_COMPONENT_OBJECT_APPLY_MASK_FULL);
        (flag_at as *mut u8).write(0);
    }
    if first(&SEEN_FULL_MATRIX) {
        log(format_args!(
            "{LOG_PREFIX} number scale applied: mask {mask_before:?} -> {}",
            ds2_rva::FE_COMPONENT_OBJECT_APPLY_MASK_FULL
        ));
    }
}

unsafe extern "system" fn set_number_detour(gauge: *mut u8, scene: usize, value: i32) {
    let digits = digit_count(value);
    if scene != 0 {
        remember_digits(scene, digits);
    }
    if first(&SEEN_NUMBER) {
        log(format_args!(
            "{LOG_PREFIX} first number written value={value} digits={digits}"
        ));
    }
    let trampoline = SET_NUMBER_TRAMPOLINE.load(Ordering::Acquire);
    // SAFETY: MinHook published this trampoline for this site, with this signature, before the
    // site was enabled.
    let original: SetNumberFn = unsafe { std::mem::transmute::<usize, SetNumberFn>(trampoline) };
    // SAFETY: the game's own arguments, forwarded untouched.
    unsafe { original(gauge, scene, value) };
}

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// All three detours are live.
    pub installed: bool,
}

/// One hook target.
struct Site {
    name: &'static str,
    rva: u32,
    detour: *mut c_void,
    trampoline: &'static AtomicUsize,
}

/// Detour the number setter, the bar transform and the text transform.
///
/// All three or none. Each is created and queued, and one `MH_ApplyQueued` enables them together,
/// so a failure before that call leaves the gauge exactly as the game draws it.
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
    MODULE_BASE.store(base, Ordering::Release);

    // SAFETY: takes no arguments; a second call reports ALREADY_INITIALIZED, accepted below.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=MH_Initialize status={status:?}"
        ));
        return failed;
    }

    let sites = [
        Site {
            name: "set-number",
            rva: ds2_rva::HP_GAUGE_SET_NUMBER,
            detour: set_number_detour as *mut c_void,
            trampoline: &SET_NUMBER_TRAMPOLINE,
        },
        Site {
            name: "bar",
            rva: ds2_rva::HP_GAUGE_BAR_TRANSFORM,
            detour: bar_detour as *mut c_void,
            trampoline: &BAR_TRAMPOLINE,
        },
        Site {
            name: "text",
            rva: ds2_rva::HP_GAUGE_TEXT_TRANSFORM,
            detour: text_detour as *mut c_void,
            trampoline: &TEXT_TRAMPOLINE,
        },
    ];

    for site in &sites {
        let va = base + site.rva as usize;
        // SAFETY: a function start recorded in `ds2-rva` that is not an Arxan redirect, and a
        // `'static` detour of the signature that function implements.
        let hook = match unsafe { MhHook::new(va as *mut c_void, site.detour) } {
            Ok(hook) => hook,
            Err(status) => {
                log(format_args!(
                    "{LOG_PREFIX} hook-failed site={} va=0x{va:x} stage=MH_CreateHook \
                     status={status:?}",
                    site.name
                ));
                return failed;
            }
        };
        // Published before anything is enabled, so no detour can read a zero trampoline.
        site.trampoline
            .store(hook.trampoline() as usize, Ordering::Release);
        // SAFETY: `hook` is the target MinHook just registered.
        if let Err(status) = unsafe { hook.queue_enable() } {
            log(format_args!(
                "{LOG_PREFIX} hook-failed site={} va=0x{va:x} stage=MH_QueueEnableHook \
                 status={status:?}",
                site.name
            ));
            return failed;
        }
    }
    // SAFETY: every queued target was registered above with a published trampoline.
    let status = unsafe { MH_ApplyQueued() };
    if status != MH_STATUS::MH_OK {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=MH_ApplyQueued status={status:?}"
        ));
        return failed;
    }
    for site in &sites {
        log(format_args!(
            "{LOG_PREFIX} hooked site={} rva=0x{:08x} va=0x{:x}",
            site.name,
            site.rva,
            base + site.rva as usize
        ));
    }
    log(format_args!("{LOG_PREFIX} installed tune={:?}", tune()));
    Outcome { installed: true }
}
