//! Crash logging inside DARK SOULS II, and the deliberate fault that proves it works.
//!
//! # Why this is linked in rather than loaded as its own DLL
//!
//! `crates/ds2-crash-logging` builds a standalone `ds2_crash_logging.dll` with its own `DllMain`,
//! and `ds2-mods-rs-4tm` is written as though that file gets into the game. It has no way in.
//! `scripts/ds2-run.py` stages exactly one file -- `dinput8.dll` -- because that is the name
//! `DarkSoulsII.exe` imports, and nothing imports `ds2_crash_logging.dll`. The only way to load a
//! second DLL from here would be `LoadLibrary` out of this DLL's `DllMain`, under the loader lock,
//! which is the textbook DLL-init deadlock this crate's own `system_dinput8_path` comment already
//! refuses to commit (see [`crate`] and its `DirectInput8Create` forward).
//!
//! So the LIBRARY is linked instead. `ds2-crash-logging-core` is the whole of the logic; the
//! standalone DLL is a `DllMain` wrapper around exactly the [`install`] call made below. Same
//! code, same files, one fewer thing to get into the process.
//!
//! [`install`]: ds2_crash_logging_core::install
//!
//! # Why the install happens before `neuter_arxan`
//!
//! The riskiest thing this DLL does is `neuter_arxan`: it applies code patches derived from static
//! analysis, and the loader's own SAFETY note says a stub misidentified as Arxan would be patched
//! wrongly and the program would be UB. That is the single most likely crash in the whole startup
//! path. A crash logger installed *after* it cannot report it. So it goes first -- it is cheap
//! (two Win32 calls and a file write, no `LoadLibrary`, no thread) and it costs nothing to have it
//! watching during the one operation most likely to need watching.
//!
//! # The deliberate fault
//!
//! A crash logger that has never seen a crash is untested, and the only honest way to test one is
//! to cause a crash. `fault_after_ms` does that: a thread that sleeps and then raises
//! `0xc0000005`. It is **off by default and startup-only**, and it kills the game when it fires --
//! that is the point, since the fatal path (unhandled filter, minidump tier) is exactly the part a
//! first-chance exception cannot exercise.
//!
//! `RaiseException` rather than a null dereference, copying
//! `crates/ds2-crash-logging/examples/load_and_crash.rs`: the fault is delivered at a known
//! instruction and this module carries no UB of its own. The return address lands inside
//! `dinput8.dll`, so the module/RVA resolution `ds2-mods-rs-4tm` asks about is exercised against a
//! module whose base this process can independently confirm.
//!
//! The fault is armed from the post-Arxan callback, NOT from `DllMain`. Spawning a thread under
//! the loader lock is its own hazard, and the callback already runs at the entry point after
//! `DllMain` has returned -- the same place [`crate::install_probe`] arms the Arxan probe.

use std::path::PathBuf;
use std::time::Duration;

use ds2_hotkey_config::kv::KeyValues;

/// This module's section in `<Game>/ds2-mods.toml`.
pub const CONFIG_SECTION: &str = "crash_logging";

/// Install the crash logger at all. Default **on** -- unlike the Arxan probe, this is not an
/// experiment, and a crash logger that has to be switched on is off on the run that needed it.
pub const KEY_ENABLED: &str = "enabled";

/// Milliseconds after the entry point to deliberately fault. `0` means never, and is the default.
pub const KEY_FAULT_AFTER_MS: &str = "fault_after_ms";

/// Milliseconds after the entry point to RE-ASSERT the top-level unhandled-exception filter.
///
/// Default 5000, and it is on by default because without it this crate never sees a fatal
/// exception in DARK SOULS II at all -- see [`CrashConfig::reinstall_filter_after_ms`].
pub const KEY_REINSTALL_FILTER_AFTER_MS: &str = "reinstall_filter_after_ms";

/// Raised by the deliberate fault: `EXCEPTION_ACCESS_VIOLATION`.
const FAULT_CODE: u32 = 0xc000_0005;

unsafe extern "system" {
    fn RaiseException(code: u32, flags: u32, arg_count: u32, args: *const usize) -> !;
}

/// What `<Game>/ds2-mods.toml` said about crash logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrashConfig {
    /// Install the vectored handler and top-level filter.
    pub enabled: bool,
    /// Deliberately fault this many milliseconds after the entry point; `0` disables it.
    pub fault_after_ms: u64,
    /// Re-assert the top-level unhandled-exception filter this many ms in; `0` disables it.
    ///
    /// ON BY DEFAULT, because the game takes the slot away from us otherwise. Measured statically
    /// from the shipped binary (`ds2-mods-rs-w0m`): `SetUnhandledExceptionFilter` has exactly one
    /// call site, `0x140c43293`, inside a function registered in the CRT initializer table at
    /// `0x1410ac2c8` -- so it runs from `_initterm` at CRT startup, AFTER every `DllMain`. It ends
    /// `CALL SetUnhandledExceptionFilter; XOR EAX,EAX`, discarding the previous filter rather than
    /// chaining it. Installing from `DllMain` therefore guarantees being overwritten and forgotten.
    ///
    /// 5000ms is a deliberately loose bound on "CRT startup is over", not a measurement. It is far
    /// past `_initterm` and far short of anything a player would reach.
    pub reinstall_filter_after_ms: u64,
}

impl Default for CrashConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fault_after_ms: 0,
            reinstall_filter_after_ms: 5_000,
        }
    }
}

impl CrashConfig {
    /// Read `<Game>/ds2-mods.toml`.
    ///
    /// Reads and parses the file a second time -- [`crate::arxan_probe::ProbeConfig::load`] has
    /// already read it by the time this runs. That is deliberate: sharing one parse would mean
    /// threading a `KeyValues` through a module that was written, documented and merged as a
    /// self-contained unit, and the cost being avoided is one read of a file under a kilobyte,
    /// once, at startup. If a third section ever wants it, hoist the parse then.
    ///
    /// A missing file is not a problem -- it means the defaults, which are "log crashes, never
    /// fault on purpose". Every unusable line comes back in the returned `Vec` rather than being
    /// silently defaulted, because a typo in a key is indistinguishable from an absent key at the
    /// point where it matters -- the caller logs them before acting on the config.
    pub fn load() -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let Some(path) = config_file_path() else {
            return (Self::default(), problems);
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            // Absent file: defaults, and no complaint. `ds2-run.py` writes this file on every
            // launch, so its absence means someone launched the game some other way.
            return (Self::default(), problems);
        };
        let parsed = KeyValues::parse(&text);
        let defaults = Self::default();
        let config = Self {
            enabled: read_bool(&parsed, KEY_ENABLED, defaults.enabled, &mut problems),
            fault_after_ms: read_u64(
                &parsed,
                KEY_FAULT_AFTER_MS,
                defaults.fault_after_ms,
                &mut problems,
            ),
            reinstall_filter_after_ms: read_u64(
                &parsed,
                KEY_REINSTALL_FILTER_AFTER_MS,
                defaults.reinstall_filter_after_ms,
                &mut problems,
            ),
        };
        (config, problems)
    }

    /// One line, for the attach log, saying what was resolved before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "crash_logging={} reinstall_filter_after_ms={} fault_after_ms={}{}",
            if self.enabled { "on" } else { "off" },
            self.reinstall_filter_after_ms,
            self.fault_after_ms,
            if self.fault_after_ms > 0 {
                "  *** THIS RUN WILL DELIBERATELY CRASH ***"
            } else {
                ""
            }
        )
    }
}

/// `<Game>/ds2-mods.toml`, beside the running executable.
fn config_file_path() -> Option<PathBuf> {
    ds2_game_base::log::game_directory_path()
        .map(|dir| dir.join(crate::arxan_probe::CONFIG_FILE_NAME))
}

/// A `key = value` bool from this module's section, or `default` if absent.
///
/// Strict on purpose: anything that is not exactly `true` or `false` is a PROBLEM, not a silent
/// default. A key spelled `enabled = yes` that quietly means `true` today is a key that quietly
/// means `false` the day the parser changes.
fn read_bool(parsed: &KeyValues, key: &str, default: bool, problems: &mut Vec<String>) -> bool {
    match parsed.get(CONFIG_SECTION, key) {
        None => default,
        Some("true") => true,
        Some("false") => false,
        Some(other) => {
            problems.push(format!(
                "[{CONFIG_SECTION}] {key} = {other:?} is not `true` or `false`; using {default}"
            ));
            default
        }
    }
}

/// A `key = value` unsigned integer from this module's section, or `default` if absent.
fn read_u64(parsed: &KeyValues, key: &str, default: u64, problems: &mut Vec<String>) -> u64 {
    match parsed.get(CONFIG_SECTION, key) {
        None => default,
        Some(raw) => match raw.parse::<u64>() {
            Ok(value) => value,
            Err(_) => {
                problems.push(format!(
                    "[{CONFIG_SECTION}] {key} = {raw:?} is not a non-negative integer; using {default}"
                ));
                default
            }
        },
    }
}

/// Install the crash logger. Call from `DllMain`, before anything that could crash.
///
/// The file names are spelled out here rather than taken from a `Default`, matching
/// `crates/ds2-crash-logging/src/lib.rs`: these are the strings a player is asked to send back, so
/// the DLL that ships them says them out loud instead of inheriting them from a library that could
/// change underneath it. They are IDENTICAL to the standalone DLL's, so a log from either is read
/// the same way.
pub fn install(module: *mut core::ffi::c_void) {
    ds2_crash_logging_core::install(
        ds2_crash_logging_core::CrashLogConfig {
            log_file_name: "ds2-crash-log.txt",
            latest_file_name: "ds2-crash-latest.txt",
            breadcrumb_file_name: "ds2-crash-breadcrumb-latest.txt",
            modules_file_name: "ds2-crash-modules.txt",
            minidump_file_name: "ds2-crash-minidump.dmp",
            module_label: "ds2-loader",
        },
        module as usize,
    );
    ds2_crash_logging_core::write_breadcrumb(
        "dll-attach",
        format_args!("loader installed crash logging before neuter_arxan"),
    );
}

/// Schedule the late re-assert of the top-level unhandled-exception filter.
///
/// Call from the post-Arxan callback, never `DllMain` -- both because spawning a thread under the
/// loader lock is a hazard and because the whole point is to run AFTER the game's CRT startup,
/// which `DllMain` precedes by construction.
///
/// Returns the line to log, or `None` when disabled.
pub fn schedule_filter_reinstall(config: CrashConfig) -> Option<String> {
    if !config.enabled || config.reinstall_filter_after_ms == 0 {
        return None;
    }
    let delay = Duration::from_millis(config.reinstall_filter_after_ms);
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        let (previous, replaced_self) = ds2_crash_logging_core::reinstall_unhandled_filter();
        ds2_crash_logging_core::write_breadcrumb(
            "filter-reasserted",
            format_args!("previous=0x{previous:x} replaced_self={replaced_self}"),
        );
    });
    Some(format!(
        "unhandled-filter re-assert scheduled in {}ms (the game's CRT takes the slot at startup)",
        config.reinstall_filter_after_ms
    ))
}

/// Arm the deliberate fault, if configured. Call from the post-Arxan callback, never `DllMain`.
///
/// Returns the thread's description for the log, or `None` when no fault was armed. The caller
/// logs it; this function does not, so the ordering of the loader's own lines stays in the
/// loader's hands.
pub fn arm_deliberate_fault(config: CrashConfig) -> Option<String> {
    if config.fault_after_ms == 0 {
        return None;
    }
    let delay = Duration::from_millis(config.fault_after_ms);
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        ds2_crash_logging_core::write_breadcrumb(
            "deliberate-fault",
            format_args!(
                "raising 0x{FAULT_CODE:08x} after {}ms -- this crash was ASKED FOR",
                delay.as_millis()
            ),
        );
        // SAFETY: `RaiseException` with no arguments and no continuable flag. It does not
        // dereference anything, so this raises a well-formed exception at a known instruction
        // rather than committing UB and hoping the fault lands where intended. It does not
        // return, which is the whole point: nothing handles 0xc0000005 here, so the top-level
        // filter runs, writes the fatal record and the minidump, and the process dies.
        unsafe { RaiseException(FAULT_CODE, 0, 0, std::ptr::null()) }
    });
    Some(format!(
        "deliberate fault ARMED: 0x{FAULT_CODE:08x} in {}ms",
        config.fault_after_ms
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> (CrashConfig, Vec<String>) {
        let parsed = KeyValues::parse(text);
        let mut problems = Vec::new();
        let defaults = CrashConfig::default();
        let config = CrashConfig {
            enabled: read_bool(&parsed, KEY_ENABLED, defaults.enabled, &mut problems),
            fault_after_ms: read_u64(
                &parsed,
                KEY_FAULT_AFTER_MS,
                defaults.fault_after_ms,
                &mut problems,
            ),
            reinstall_filter_after_ms: read_u64(
                &parsed,
                KEY_REINSTALL_FILTER_AFTER_MS,
                defaults.reinstall_filter_after_ms,
                &mut problems,
            ),
        };
        (config, problems)
    }

    #[test]
    fn an_absent_section_logs_crashes_and_never_faults() {
        let (config, problems) = parse("");
        assert!(config.enabled, "crash logging must default ON");
        assert_eq!(config.fault_after_ms, 0, "a fault must never be a default");
        assert!(problems.is_empty());
    }

    #[test]
    fn a_misspelled_bool_is_a_problem_rather_than_a_silent_default() {
        let (config, problems) = parse("[crash_logging]\nenabled = yes\n");
        assert!(config.enabled, "falls back to the default");
        assert_eq!(problems.len(), 1, "and says so: {problems:?}");
        assert!(problems[0].contains("enabled"));
    }

    #[test]
    fn disabling_takes_exactly_the_word_false() {
        let (config, problems) = parse("[crash_logging]\nenabled = false\n");
        assert!(!config.enabled);
        assert!(problems.is_empty());
    }

    #[test]
    fn a_negative_delay_cannot_arm_a_fault() {
        let (config, problems) = parse("[crash_logging]\nfault_after_ms = -1\n");
        assert_eq!(config.fault_after_ms, 0, "must not wrap into a huge delay");
        assert_eq!(problems.len(), 1, "and says so: {problems:?}");
    }

    #[test]
    fn the_description_shouts_when_the_run_will_crash_on_purpose() {
        let quiet = CrashConfig::default().describe();
        assert!(!quiet.contains("DELIBERATELY"), "{quiet}");

        let armed = CrashConfig {
            fault_after_ms: 15_000,
            ..CrashConfig::default()
        }
        .describe();
        assert!(armed.contains("DELIBERATELY CRASH"), "{armed}");
        assert!(armed.contains("15000"), "{armed}");
    }

    #[test]
    fn the_filter_reassert_is_on_by_default() {
        let (config, problems) = parse("");
        assert_eq!(
            config.reinstall_filter_after_ms, 5_000,
            "the game's CRT takes the filter slot; not re-asserting means never seeing a fatal"
        );
        assert!(problems.is_empty());
    }

    #[test]
    fn the_filter_reassert_can_be_turned_off_with_zero() {
        let (config, _) = parse("[crash_logging]\nreinstall_filter_after_ms = 0\n");
        assert_eq!(config.reinstall_filter_after_ms, 0);
        assert_eq!(schedule_filter_reinstall(config), None);
    }

    #[test]
    fn disabling_crash_logging_also_stops_the_reassert() {
        let config = CrashConfig {
            enabled: false,
            ..CrashConfig::default()
        };
        assert_eq!(
            schedule_filter_reinstall(config),
            None,
            "re-asserting a filter that was never installed would publish a handler with no config"
        );
    }

    #[test]
    fn a_zero_delay_arms_nothing() {
        assert_eq!(
            arm_deliberate_fault(CrashConfig {
                fault_after_ms: 0,
                ..CrashConfig::default()
            }),
            None
        );
    }
}
