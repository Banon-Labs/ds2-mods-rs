//! Installing the offline layers, and reporting what each one actually did.

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{LOG_PREFIX, flag, winsock};

/// A log sink, installed by the loader so this crate writes into the same file as everything else.
/// Stored as a `usize` because a `fn` pointer is not an `Atomic` type.
static LOGGER: AtomicUsize = AtomicUsize::new(0);

/// Signature of the sink. Matches the loader's own logging entry point.
pub type LogFn = fn(std::fmt::Arguments<'_>);

/// Point this crate's logging at the loader's log file. Call before [`install`].
pub fn set_logger(logger: LogFn) {
    LOGGER.store(logger as usize, Ordering::Release);
    winsock::set_logger(logger);
    // `patch_3byte_stub` reports its own ABORTS through this seam -- a first byte that is not the
    // one `ds2-rva` recorded, or a refused `VirtualProtect`. Those lines are how a version drift
    // announces itself instead of passing silently, so the seam is installed here rather than
    // left to the loader: a caller who forgot would lose exactly the diagnostics that matter.
    ds2_hook::set_hook_logger(logger);
}

pub(crate) fn log(args: std::fmt::Arguments<'_>) {
    let raw = LOGGER.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: `raw` is only ever a `LogFn` stored by `set_logger`.
        let logger: LogFn = unsafe { std::mem::transmute::<usize, LogFn>(raw) };
        logger(args);
    }
}

/// Which layers to install. Each is its own switch for the reason every switch in this repo is its
/// own switch: a run that misbehaves has to be attributable to one change by editing one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    /// Make `NetService::setOnline` a `ret`, so the flag keeps the zero its constructor wrote.
    pub pin_flag: bool,
    /// Make `NetService::isOnline` return zero to all 34 of its readers.
    pub report_offline: bool,
    /// Front the game's outbound `WS2_32` imports and refuse non-loopback destinations.
    pub block_sockets: bool,
}

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// `NetService::setOnline` is now inert.
    pub setter_pinned: bool,
    /// `NetService::isOnline` now returns zero.
    pub getter_forced: bool,
    /// Import slots fronted, of those looked for.
    pub sockets_patched: usize,
    /// Import slots looked for. Zero when `block_sockets` was false.
    pub sockets_attempted: usize,
    /// The online flag as read straight out of the live object, bypassing the getter -- or `None`
    /// when `GameManagerImp` did not exist yet, which is the usual case at install time.
    pub flag_at_install: Option<u8>,
}

impl Outcome {
    /// Whether anything at all was installed. A caller reporting to a launcher wants one boolean
    /// and the detail underneath it.
    pub fn any(&self) -> bool {
        self.setter_pinned || self.getter_forced || self.sockets_patched > 0
    }
}

/// Install the offline layers.
///
/// Nothing here is conditional on anything else succeeding, and that is deliberate. The two layers
/// answer different questions -- what the game believes, and what leaves the machine -- so a
/// failure in one is not a reason to skip the other. The [`Outcome`] says plainly which landed.
///
/// # Safety
///
/// Patches executable memory and the import table of the loaded game image. Must run after
/// `neuter_arxan` (or `schedule_after_arxan`) and before the title flow reaches the network
/// substates -- in practice, from the loader's post-Arxan callback, which is on the entry-point
/// thread with `DllMain` already returned and no frame yet drawn.
pub unsafe fn install(request: Request) -> Outcome {
    let base = match ds2_game_base::mem::game_module_base() {
        Ok(base) => base,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} install-failed stage=module-base error={error}"
            ));
            return Outcome::default();
        }
    };

    // SAFETY: `base` is the live module base and both targets are leaf functions recorded in
    // `ds2-rva`; `patch_3byte_stub` re-reads the first byte and refuses to write if it is not the
    // one recorded there, and `flag::apply` reads the result back before reporting success.
    let flags = unsafe { flag::apply(base, request.pin_flag, request.report_offline) };

    let sockets = if request.block_sockets {
        // SAFETY: same position, and this writes `.idata` pointers rather than code.
        unsafe { winsock::install(base) }
    } else {
        winsock::Outcome::default()
    };

    // Read the flag through the object rather than through the getter. After `report_offline` the
    // getter is a lie by construction, so asking it would prove nothing; this is the byte the
    // getter WOULD have read. At this point in the boot `GameManagerImp` is normally still null,
    // and `None` is reported as such rather than as a zero.
    //
    // SAFETY: every hop is a guarded read that returns `None` instead of faulting.
    let flag_at_install = unsafe { flag::read_flag(base) };

    let outcome = Outcome {
        setter_pinned: flags.setter_pinned,
        getter_forced: flags.getter_forced,
        sockets_patched: sockets.patched,
        sockets_attempted: if request.block_sockets {
            sockets.attempted
        } else {
            0
        },
        flag_at_install,
    };

    log(format_args!(
        "{LOG_PREFIX} install pin_flag={} report_offline={} sockets={}/{} flag={} \
         found_ws2_32={}",
        outcome.setter_pinned,
        outcome.getter_forced,
        outcome.sockets_patched,
        outcome.sockets_attempted,
        match outcome.flag_at_install {
            Some(value) => value.to_string(),
            None => "<not-constructed-yet>".to_string(),
        },
        sockets.found_ws2_32,
    ));
    outcome
}
