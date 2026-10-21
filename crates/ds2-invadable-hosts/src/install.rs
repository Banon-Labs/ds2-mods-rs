//! The log sink and the install the loader calls.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::LOG_PREFIX;

/// A log sink, installed by the loader so this crate writes into the same file as everything else.
/// Stored as a `usize` because a `fn` pointer is not an `Atomic` type.
static LOGGER: AtomicUsize = AtomicUsize::new(0);

/// Signature of the sink. Matches the loader's own logging entry point.
pub type LogFn = fn(std::fmt::Arguments<'_>);

/// Point this crate's logging at the loader's log file. Call before [`install`].
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

/// Register the panel with `ds2-overlay`.
///
/// `ds2-overlay` must already be started and the input harness installed. `config` is
/// `ds2-mods.toml` and `text` what it said at attach; the keys are re-read from it while the game
/// runs.
///
/// Nothing touches Steam here. The matchmaking interface is wrapped, and counting starts, the first
/// time the player opens the panel.
pub fn install(config: PathBuf, text: &str) -> bool {
    let installed = crate::panel::install(config, text);
    log(format_args!(
        "{LOG_PREFIX} panel {}",
        if installed {
            "installed; Steam is left alone until the panel is first opened"
        } else {
            "NOT installed"
        }
    ));
    installed
}
