//! The log seam, the same shape every other feature crate in this repo uses.
//!
//! A sink is product-specific -- a path, a rotate policy, a lifetime this crate knows nothing
//! about -- so the loader installs one and this crate calls through a function pointer. Lines
//! emitted before a sink exists are dropped rather than buffered, so [`set_logger`] belongs
//! before [`crate::install`] and not after it.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Prefix on every line this crate writes. `scripts/ds2-run.py` greps for it, so it is part of a
/// contract rather than a decoration.
pub const LOG_PREFIX: &str = "ds2-input-harness:";

/// Signature of the sink. Matches the loader's own logging entry point.
pub type LogFn = fn(std::fmt::Arguments<'_>);

/// Stored as a `usize` because a `fn` pointer is not an `Atomic` type. Only ever written by
/// [`set_logger`].
static LOGGER: AtomicUsize = AtomicUsize::new(0);

/// Point this crate's logging at the loader's log file. Call before [`crate::install`].
pub fn set_logger(logger: LogFn) {
    LOGGER.store(logger as usize, Ordering::Release);
}

/// Write one line, if a sink has been installed.
pub(crate) fn log(args: std::fmt::Arguments<'_>) {
    let raw = LOGGER.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: `raw` is only ever a `LogFn` stored by `set_logger` above.
        let logger: LogFn = unsafe { std::mem::transmute::<usize, LogFn>(raw) };
        logger(args);
    }
}

/// `log!` with the prefix already on it, so no call site can forget it.
macro_rules! harness_log {
    ($($arg:tt)*) => {
        $crate::log::log(format_args!("{} {}", $crate::log::LOG_PREFIX, format_args!($($arg)*)))
    };
}

pub(crate) use harness_log;
