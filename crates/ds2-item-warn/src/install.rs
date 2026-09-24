//! The log sink, the prologue-checked patch, and the all-or-nothing install of the pair.

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;

/// A log sink, installed by the loader so this crate writes into the same file as everything else
/// rather than opening one of its own. Stored as a `usize` because a `fn` pointer is not an
/// `Atomic` type; only ever set from [`set_logger`].
static LOGGER: AtomicUsize = AtomicUsize::new(0);

/// Signature of the sink. Matches the loader's own logging entry point.
pub type LogFn = fn(std::fmt::Arguments<'_>);

/// Install the sink. Call before [`install`], or the lines that say why an install refused are the
/// ones that are lost.
pub fn set_logger(logger: LogFn) {
    LOGGER.store(logger as usize, Ordering::Release);
}

pub(crate) fn log(args: std::fmt::Arguments<'_>) {
    let raw = LOGGER.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: `raw` is only ever a `LogFn` stored by `set_logger` above.
        let logger: LogFn = unsafe { std::mem::transmute::<usize, LogFn>(raw) };
        logger(args);
    }
}

/// The loaded module base, published once before either detour can read it.
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);

/// Where the game is loaded, or `0` before [`install`] has resolved it.
pub(crate) fn module_base() -> usize {
    MODULE_BASE.load(Ordering::Acquire)
}

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Both detours are in. Anything less is `false` and the shipped cell is untouched.
    pub installed: bool,
}

/// Patch one site, after re-reading the bytes that are about to be overwritten.
///
/// The refusal is the point. An RVA is a number, and on a build these were not read from each of
/// these addresses is some other function that would accept the patch and produce a log line
/// claiming an install. What it actually found is printed beside what it wanted.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. `base` must be the live module base and
/// MinHook must already be initialised.
unsafe fn hook_site(
    base: usize,
    rva: u32,
    prologue: &[u8],
    detour: *mut c_void,
    trampoline: &AtomicUsize,
    what: &str,
) -> bool {
    let site = base + rva as usize;
    // SAFETY: `site` is inside the loaded image's `.text` -- a function start recorded in
    // `ds2-rva` resolved against the live base -- so `prologue.len()` bytes are readable there.
    let found = unsafe { std::slice::from_raw_parts(site as *const u8, prologue.len()) };
    if found != prologue {
        log(format_args!(
            "{LOG_PREFIX} {what} NOT installed stage=prologue rva=0x{rva:08x} va=0x{site:016x} \
             expected={prologue:02x?} found={found:02x?}"
        ));
        return false;
    }
    // SAFETY: the bytes at the site are the recorded ones, so this is the function it claims to be.
    let hook = match unsafe { MhHook::new(site as *mut c_void, detour) } {
        Ok(hook) => hook,
        Err(status) => {
            log(format_args!(
                "{LOG_PREFIX} {what} NOT installed stage=MH_CreateHook rva=0x{rva:08x} \
                 va=0x{site:016x} status={status:?} -- another feature may already own this \
                 prologue"
            ));
            return false;
        }
    };
    trampoline.store(hook.trampoline() as usize, Ordering::Release);
    // SAFETY: the hook was just created for this exact address.
    let status = unsafe { MH_EnableHook(site as *mut c_void) };
    if status != MH_STATUS::MH_OK {
        log(format_args!(
            "{LOG_PREFIX} {what} NOT installed stage=MH_EnableHook rva=0x{rva:08x} \
             va=0x{site:016x} status={status:?}"
        ));
        trampoline.store(0, Ordering::Release);
        return false;
    }
    log(format_args!(
        "{LOG_PREFIX} {what} hooked rva=0x{rva:08x} va=0x{site:016x}"
    ));
    true
}

/// Detour the container builder and the item-cell bind. Call from the post-Arxan callback, never
/// `DllMain`.
///
/// **The pair goes in together.** The badge element and the switch that shows it are two halves of
/// one feature and neither is worth anything alone -- worse, a run with only the element would
/// draw nothing and a run with only the switch would write to an unresolved accessor, and both
/// look from the outside exactly like a requirement check that decided "met". So the container
/// hook goes first, and if the bind hook then refuses, the container hook is left installed but
/// DISARMED: it declines to substitute anything and the game gets the definition it asked for.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Must run after `neuter_arxan` (or after
/// `schedule_after_arxan`), which in practice means the loader's Arxan callback.
pub unsafe fn install() -> Outcome {
    let base = match ds2_game_base::mem::game_module_base() {
        Ok(base) => base,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} install-failed stage=module-base error={error}"
            ));
            return Outcome { installed: false };
        }
    };

    // MinHook is statically linked into this DLL, so nothing else shares this instance and
    // ALREADY_INITIALIZED can only mean this ran twice. Treat it as success.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=MH_Initialize status={status:?}"
        ));
        return Outcome { installed: false };
    }

    // Published before any detour that reads it, and never changed again.
    MODULE_BASE.store(base, Ordering::Release);

    // SAFETY: both rvas are function starts recorded in `ds2-rva` with the bytes they must begin
    // with, `hook_site` re-reads those bytes and refuses otherwise, and MinHook is initialised.
    let container = unsafe {
        hook_site(
            base,
            ds2_rva::FLO_BUILD_CONTAINER,
            &ds2_rva::FLO_BUILD_CONTAINER_PROLOGUE,
            crate::mark::detour as *mut c_void,
            &crate::mark::TRAMPOLINE,
            "container-builder",
        )
    };
    if !container {
        return Outcome { installed: false };
    }
    // SAFETY: as above.
    let bind = unsafe {
        hook_site(
            base,
            ds2_rva::FE_ITEM_CELL_BIND,
            &ds2_rva::FE_ITEM_CELL_BIND_PROLOGUE,
            crate::requirement::detour as *mut c_void,
            &crate::requirement::TRAMPOLINE,
            "item-cell-bind",
        )
    };
    if !bind {
        // The container hook cannot be removed safely from here -- another thread may be inside
        // its trampoline -- so it is disarmed instead. `mark::armed` is what every substitution
        // asks first, so from this point it is a pass-through that logs nothing.
        crate::mark::disarm();
        log(format_args!(
            "{LOG_PREFIX} NOT INSTALLED -- the container hook is disarmed and every definition \
             passes through untouched; item cells are the ones the game shipped"
        ));
        return Outcome { installed: false };
    }
    crate::mark::arm();
    log(format_args!(
        "{LOG_PREFIX} installed -- weapons whose requirements the player fails get a red badge at \
         icon-local ({:.2}, {:.2}); UNVERIFIED AT RUNTIME",
        ds2_rva::FE_ITEM_ICON_BOX[0] + ds2_rva::FE_ITEM_WARN_INSET,
        ds2_rva::FE_ITEM_ICON_BOX[3] - ds2_rva::FE_ITEM_WARN_SIZE[1] - ds2_rva::FE_ITEM_WARN_INSET,
    ));
    Outcome { installed: true }
}
