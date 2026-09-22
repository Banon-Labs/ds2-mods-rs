//! One line into the loader's log file, or nowhere.
//!
//! The same shape every other feature crate in this workspace uses: the loader owns the file and
//! hands a sink over before installing, so a crate that is linked but never installed writes
//! nothing rather than opening a second log beside the first.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Signature of the sink. Matches the loader's own logging entry point.
pub type LogFn = fn(std::fmt::Arguments<'_>);

/// The sink, as a plain address because a function pointer is not an atomic type.
static LOGGER: AtomicUsize = AtomicUsize::new(0);

/// Point this crate's logging at the loader's log file. Call before `install`.
pub fn set_logger(logger: LogFn) {
    LOGGER.store(logger as usize, Ordering::Release);
}

/// What every line from this crate is prefixed with, so a reader can grep one feature out of a
/// file a dozen of them share.
pub const LOG_PREFIX: &str = "ds2-invasion-path:";

/// Write one line, if there is anywhere to write it.
pub(crate) fn log(args: std::fmt::Arguments<'_>) {
    let raw = LOGGER.load(Ordering::Acquire);
    if raw == 0 {
        return;
    }
    // SAFETY: `raw` is only ever a `LogFn` stored by `set_logger` above.
    let logger: LogFn = unsafe { std::mem::transmute::<usize, LogFn>(raw) };
    logger(format_args!("{LOG_PREFIX} {args}"));
}
