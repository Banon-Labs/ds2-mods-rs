//! `dinput8.dll` -- the DARK SOULS II loader shell. Three jobs, and one experiment that is off
//! by default.
//!
//! This is the smallest thing that can prove the loading path works. On its own it does no
//! hooking, reads no game memory and holds no DS2 address: it exists to answer one question --
//! *does a proxied import actually get our code into this process, early enough to matter?* --
//! and to leave a log line on disk that answers it without anyone having to take an agent's
//! word for it. [`arxan_probe`] is bolted on behind a config file and is inert unless that file
//! asks for it.
//!
//! # Why a proxy DLL at all
//!
//! `me3` cannot launch this game (`me3 profile create --game` accepts `darksouls3`, `sekiro`,
//! `eldenring`, `armoredcore6`, `nightreign` and nothing else), so the `[[natives]]` mechanism
//! every `../er-mods-rs` crate assumes does not exist here. A mod gets in by proxying one of
//! `DarkSoulsII.exe`'s own imports. `DINPUT8.dll` is the smallest honest surface in its import
//! table: a single named export, resolved by name, and the least load-bearing thing in the
//! list -- a broken forward costs controller input, which is obvious and harmless, rather than
//! the renderer. See `docs/LOADING.md` for the full survey.
//!
//! # The three jobs, in this order
//!
//! 1. [`neuter_arxan`](dearxan::disabler::neuter_arxan) -- one call, before anything else.
//! 2. Log what it reported, through [`ds2_game_base::log`].
//! 3. Forward [`DirectInput8Create`] to the real system `dinput8.dll` so input still works.
//!
//! # Why the Arxan patch belongs HERE and not in a later hook shell
//!
//! A statically-imported DLL's `DllMain(DLL_PROCESS_ATTACH)` runs during import resolution,
//! **before the executable's entry point**. That is exactly the position `neuter_arxan`
//! documents as the good one: called there, and with Arxan having hooked the MSVC CRT entry
//! sequence, dearxan patches the Arxan entry stubs before `__security_init_cookie` runs, so
//! their checks never execute. Its own docs warn that the after-the-fact path performs
//! best-effort synchronisation that "is not perfect and may lead to race conditions", and
//! **strongly** recommend a loader that creates the process suspended. Proxying an import gets
//! the same guarantee without a suspended-process launcher; `LoadLibrary` into a live process
//! does not, because it arrives after the entry stubs have already run.
//!
//! # The fourth job, and it is off unless you ask for it
//!
//! [`arxan_probe`] folds the **M1 experiment** into this DLL: one MinHook detour on one chosen
//! function, watched byte-for-byte to see whether Arxan reverts it. It does nothing at all
//! unless `<Game>/ds2-mods.toml` says `enabled = true` under `[arxan_probe]`, so an ordinary run
//! of this loader is byte-for-byte the loader described above.
//!
//! It is configured by a FILE and not by the environment, and that is a correction rather than a
//! preference. `steam -applaunch` hands the request to an already-running Steam client over IPC
//! and the game inherits *that client's* environment, so the probe's variables arrived unset in a
//! real run while `WINEDLLOVERRIDES` -- which lives in the per-app Steam launch options, a
//! different channel -- arrived fine. See the banner comment in [`arxan_probe`].
//!
//! It lives HERE rather than in a second DLL on purpose. The proxy is currently the only thing
//! that gets into this process at all, and inventing a DLL-chaining mechanism to host the probe
//! would add a second untested variable to an experiment whose entire purpose is to isolate one.
//! When the probe has reported, the chaining mechanism can be built against a known-good hook.
//!
//! # Under Proton
//!
//! Dropping this beside the exe is not enough -- Wine prefers its own builtin. The run needs
//! `WINEDLLOVERRIDES="dinput8=n,b"` ("native first, then builtin"), which is what
//! `scripts/ds2-run.py` asks for. Without it this DLL is never loaded and the log below never
//! appears; the launcher gates on the log line precisely so that case cannot be mistaken for a
//! successful run.
//!
//! `WINEDLLOVERRIDES` is the one setting that genuinely has to travel through Steam, because it
//! is read by Wine before this DLL exists to read anything. It works because it is set in the
//! per-app Steam launch options rather than in the launcher's environment. Everything the DLL
//! itself decides now comes out of `<Game>/ds2-mods.toml` -- see
//! [`arxan_probe::ProbeConfig::load`].

// The whole crate is Windows-only by construction: it is a PE export surface and a Win32
// import forward. On a host build this leaves an empty cdylib rather than a link error, which
// is what keeps `scripts/check.sh --host-tests` runnable for the crates that are host-testable.
#![cfg(windows)]

use core::ffi::c_void;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Once, OnceLock};

use dearxan::disabler::result::DearxanResult;
use dearxan::disabler::{neuter_arxan, schedule_after_arxan};

pub mod arxan_probe;
pub mod crash_logging;
pub mod dialog_skip;
pub mod intro_skip;
pub mod title_skip;

/// `fdwReason` value for the loader's process-attach notification.
const DLL_PROCESS_ATTACH: u32 = 1;
/// `fdwReason` value for the loader's process-detach notification. Fires on an orderly
/// `ExitProcess`; does NOT fire on `TerminateProcess`. That asymmetry is the whole reason the
/// probe writes a line here -- see [`arxan_probe::detach_line`].
const DLL_PROCESS_DETACH: u32 = 0;
/// `DllMain` returns a Win32 `BOOL`; non-zero means "the DLL initialised successfully".
/// Returning zero here would fail the *executable's* import resolution and the game would not
/// start at all, so this path never reports failure -- a loader that could not do its job still
/// has to let the game boot.
const DLL_MAIN_SUCCESS: i32 = 1;

/// `E_FAIL`, the generic COM failure `HRESULT`. Returned by [`DirectInput8Create`] when the
/// real system DLL could not be reached, which is the honest answer: DirectInput is
/// unavailable. It is not `S_OK`, because claiming success and handing back an uninitialised
/// `ppvOut` would turn a missing forward into a null-dereference somewhere inside the game.
const E_FAIL: i32 = 0x8000_4005_u32 as i32;

/// The file name of the log this DLL writes, next to `DarkSoulsII.exe`.
///
/// `scripts/ds2-run.py` polls for [`ARXAN_LINE_PREFIX`] in this file and refuses to print a
/// success block without it, so this name is part of that contract -- changing it here without
/// changing it there turns every run into a false "did not load".
const LOG_FILE_NAME: &str = "ds2-loader.log";

/// Written once at `DLL_PROCESS_ATTACH`. Proves the DLL was loaded at all, which is a strictly
/// weaker claim than [`ARXAN_LINE_PREFIX`] and is kept separate for exactly that reason: a run
/// with this line and not the other one loaded fine and dearxan never reported, which is a
/// completely different failure from "the override did not take and we were never loaded".
/// Every line this DLL writes opens with this, so a grep of the game directory's logs can tell
/// our lines from the game's and from any other mod's. `arxan_probe::echo_lines` already carries
/// it; the lines composed in this file have to add it themselves.
const LOG_PREFIX: &str = "ds2-loader:";

const ATTACH_LINE_PREFIX: &str = "ds2-loader: attach";

/// Written from dearxan's callback. **This is the runtime test's evidence.**
const ARXAN_LINE_PREFIX: &str = "ds2-loader: arxan";

unsafe extern "system" {
    fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    fn GetModuleFileNameW(module: *mut c_void, filename: *mut u16, size: u32) -> u32;
    fn LoadLibraryW(filename: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, proc_name: *const u8) -> *mut c_void;
    fn GetLastError() -> u32;
}

/// The entry point the Windows loader calls. Runs before `DarkSoulsII.exe`'s own entry point.
///
/// # Safety
///
/// Called by the Windows loader with the loader lock held. Do not call directly.
#[unsafe(no_mangle)]
#[allow(
    non_snake_case,
    reason = "the loader looks up this exact spelling; `dll_main` is not an entry point"
)]
pub unsafe extern "system" fn DllMain(
    module: *mut c_void,
    reason: u32,
    _reserved: *mut c_void,
) -> i32 {
    if reason == DLL_PROCESS_ATTACH {
        // `DLL_PROCESS_ATTACH` fires once per process, but `neuter_arxan`'s internals panic if
        // their one-shot is re-entered, and a panic unwinding out of `DllMain` would take the
        // game's startup with it. The latch costs one relaxed atomic and removes the question.
        static ATTACHED: Once = Once::new();
        ATTACHED.call_once(|| unsafe { attach(module) });
    }
    if reason == DLL_PROCESS_DETACH
        && let Some(line) = arxan_probe::detach_line()
    {
        detach_log_line(&line);
    }
    DLL_MAIN_SUCCESS
}

/// Write the probe's closing line, **deliberately not through [`log_line`]**.
///
/// `ds2_game_base::log::open_fresh_run_append` takes a process-wide mutex on its way to the
/// rotate bookkeeping. By the time `DLL_PROCESS_DETACH` runs on a terminating process, every
/// other thread has already been killed wherever it happened to be -- including, with small but
/// non-zero probability, inside that mutex. Taking it here would hang the game on exit, and a
/// game that hangs when you quit it is a far worse outcome than a missing final log line.
///
/// The file has necessarily been freshened already (the probe wrote its install lines through
/// the normal path), so a plain append is correct as well as safe.
fn detach_log_line(line: &str) {
    let Some(path) = ds2_game_base::log::game_directory_path().map(|dir| dir.join(LOG_FILE_NAME))
    else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "{line}");
        let _ = file.sync_all();
    }
}

/// # Safety
///
/// Calls [`neuter_arxan`], or -- in the probe's skip arm -- [`schedule_after_arxan`]. See the
/// safety notes on the calls themselves.
unsafe fn attach(module: *mut c_void) {
    // FIRST, before any logging: the identity line is written by the log module's one-shot
    // rotate, so it has to be in place before the first line is appended or the log opens
    // without it. `set_identity_line`'s whole purpose is to answer "which build wrote this?"
    // for a log that arrives with a symptom in it.
    ds2_game_base::log::set_identity_line(identity_line(module));

    // CRASH LOGGING BEFORE ANYTHING THAT CAN CRASH, and specifically before `neuter_arxan`. That
    // call applies code patches derived from static analysis, and the SAFETY note on it below says
    // plainly that a stub misidentified as Arxan would be patched wrongly and the program would be
    // UB -- the single most likely crash in this whole startup path. A logger installed after it
    // could not report the crash it most exists to report. It is cheap enough to afford here: two
    // Win32 calls and a file write, no `LoadLibrary`, no thread. See `crash_logging`.
    let (crash_config, crash_problems) = crash_logging::CrashConfig::load();
    if crash_config.enabled {
        crash_logging::install(module);
    }

    // Read the config file ONCE, here, and log what it resolved to. Everything below branches on
    // this value, so a run that was configured differently from how anyone believed says so in
    // its own opening lines instead of looking like a probe that failed to report.
    let config = arxan_probe::ProbeConfig::load();
    arxan_probe::set_probe_logger(log_line);

    // ECHO WHAT WAS READ, BEFORE ACTING ON IT. The path, whether the file was there at all, every
    // key verbatim as written, and every line that could not be used. The environment version
    // printed the raw variable values on the attach line so a typo was visible; a file can fail
    // in one more way than a variable can -- it can be absent, or be somewhere else -- so there
    // is one more thing to print. See `ProbeConfig::echo_lines`.
    for line in config.echo_lines() {
        log_line(format_args!("{line}"));
    }
    // Same contract as the probe's echo: every unusable line named, then what was resolved --
    // before anything acts on it. A run configured differently from how anyone believed says so in
    // its own opening lines rather than looking like a logger that failed to report.
    for problem in &crash_problems {
        log_line(format_args!("{LOG_PREFIX} config unusable: {problem}"));
    }
    log_line(format_args!(
        "{LOG_PREFIX} config resolved {}",
        crash_config.describe()
    ));

    // JOB 2 (first half): say we are here. If dearxan's callback never fires, this line is the
    // difference between "loaded, and dearxan went quiet" and "never loaded at all".
    log_line(format_args!(
        "{ATTACH_LINE_PREFIX} awaiting-arxan-callback {}",
        config.describe(),
    ));

    // `None` when the probe is off, which is the default and is the whole of the difference
    // between this build and the loader without it.
    let arm = config.arm;
    let probe = config.enabled.then_some(config);
    match arm {
        // JOB 1, and the default path. SAFETY: dearxan applies code patches derived from static
        // analysis of the loaded image, so a stub misidentified as Arxan would be patched wrongly
        // and the program would be UB. That risk is inherent to the crate and is why the function
        // is `unsafe`; nothing at this call site can reduce it. What this call site CAN control is
        // the timing hazard the docs single out, and it does: this is a statically-imported DLL's
        // `DLL_PROCESS_ATTACH`, which runs during import resolution, before the executable's entry
        // point -- the position dearxan asks for, and the reason no suspended-process launcher is
        // needed. It is called exactly once (the `Once` above), and the callback is
        // `Send + 'static` because dearxan may run it on the entry-point thread or, if it could
        // not synchronise, on one of its own.
        arxan_probe::Arm::NeuterArxan => unsafe {
            neuter_arxan(move |result: DearxanResult| {
                // JOB 2 (second half). Not in `DllMain`: this callback runs at the entry point,
                // after `DllMain` has returned. The result carries both facts the runtime test
                // needs -- whether Arxan was there, and whether we got to speak before the entry
                // point ran rather than racing it from a side thread.
                match result {
                    Ok(status) => log_line(format_args!(
                        "{ARXAN_LINE_PREFIX} status=ok detected={} blocking_entrypoint={}",
                        status.is_arxan_detected, status.is_executing_entrypoint,
                    )),
                    // No `Status` on the error path, so `detected`/`blocking_entrypoint` are
                    // genuinely unknown here and are not printed as guesses.
                    Err(error) => log_line(format_args!(
                        "{ARXAN_LINE_PREFIX} status=error error={error}"
                    )),
                }
                install_probe(probe);
                install_intro_skip();
                install_dialog_skip();
                install_title_skip();
                arm_fault(crash_config);
            });
        },

        // THE A/B ARM. `neuter_arxan` is not called, so Arxan's 48 stubs are left running and can
        // do whatever they do to our detour. What IS called is the scheduling half of it:
        // `neuter_arxan` is "patch the stubs before the Arxan entry stub" plus "report after it",
        // and this is the second of those two on its own. That is what keeps the arms comparable
        // -- same thread, same point in the CRT entry sequence, same SteamStub handling, exactly
        // one variable changed. Installing from `DllMain` instead would have changed two.
        //
        // SAFETY: the same inherent risk as above minus the stub patching. `schedule_after_arxan`
        // may still patch SteamStub headers, which is the part both arms deliberately share.
        arxan_probe::Arm::SkipNeuterArxan => unsafe {
            schedule_after_arxan(move |detected: bool, blocking_entrypoint: bool| {
                // Deliberately the SAME prefix a neutered run writes. `scripts/ds2-run.py` gates
                // on it to mean "the loader reached the Arxan decision point and is reporting what
                // it did there", and `status=skipped` is a truthful answer to that question.
                // A different prefix would make the skip arm indistinguishable from a run in which
                // the DLL was never loaded.
                log_line(format_args!(
                    "{ARXAN_LINE_PREFIX} status=skipped detected={detected} \
                     blocking_entrypoint={blocking_entrypoint}"
                ));
                install_probe(probe);
                install_intro_skip();
                install_dialog_skip();
                install_title_skip();
                arm_fault(crash_config);
            });
        },
    }
}

/// Install the probe, if this run is one. Called from inside whichever Arxan callback ran, so it
/// is on the entry-point thread with `DllMain` already returned.
///
/// Takes the whole config rather than just the arm: the poller thread it starts goes on watching
/// the same file for the two knobs that can move mid-run, and it needs to know what they were at
/// install to report an edit to the two that cannot.
fn install_probe(config: Option<arxan_probe::ProbeConfig>) {
    if let Some(config) = config {
        // SAFETY: the target is a `.pdata` function start recorded in `ds2-rva`, resolved against
        // the live module base, and the probe re-reads the prologue and refuses to patch anything
        // if it is not the five bytes recorded there. The detour is a naked tail-jump, so it
        // imposes no ABI on the function it fronts.
        unsafe { arxan_probe::install(&config) };
    }
}

/// Install the boot-screen skip, if `<Game>/ds2-mods.toml` asked for it.
///
/// Called from the same post-Arxan position as [`install_probe`], and for the same two reasons:
/// `DllMain` has returned so patching executable memory is not happening under the loader lock,
/// and Arxan's entry stubs have already been dealt with one way or the other. It must also be
/// EARLY enough that the title flow has not reached these substates yet, which the entry-point
/// callback is -- the game has not drawn a frame at this point.
fn install_intro_skip() {
    let config = intro_skip::IntroSkipConfig::load();
    log_line(format_args!("{}", config.describe()));
    if !config.enabled {
        return;
    }
    ds2_intro_skip::set_logger(log_line);
    // SAFETY: the three targets are vtable-slot function starts recorded in `ds2-rva`, resolved
    // against the live module base, and each was checked with `scripts/ds2-arxan-chain.py` to be
    // a real prologue rather than an Arxan redirect. The detours declare the signature every
    // override actually implements: `void enter(this)` with `this` in RCX.
    let outcome = unsafe { ds2_intro_skip::install() };
    if outcome.installed != outcome.attempted {
        log_line(format_args!(
            "{} PARTIAL {}/{} screens hooked -- the rest will still appear",
            ds2_intro_skip::LOG_PREFIX,
            outcome.installed,
            outcome.attempted
        ));
    }
}

/// Install the message-box skip, if `<Game>/ds2-mods.toml` asked for it.
///
/// Called immediately after [`install_intro_skip`] and from the same post-Arxan position, for the
/// same reasons. The order between the two is not load-bearing -- they hook different functions
/// and share no state -- but it matches the order the player meets them in, which is the order
/// the log then reads in.
fn install_dialog_skip() {
    let config = dialog_skip::DialogSkipConfig::load();
    log_line(format_args!("{}", config.describe()));
    if !config.enabled {
        return;
    }
    ds2_dialog_skip::set_logger(log_line);
    // SAFETY: the target is `FeSubStateCommonWindowBase::v3`, a `.pdata` function start recorded
    // in `ds2-rva` and resolved against the live module base. `scripts/ds2-arxan-chain.py` shows
    // it is not Arxan-redirected, and its first instruction is a whole five bytes so MinHook never
    // has to split one. The detour declares the signature every substate update implements:
    // `void update(this, float delta)`, `this` in RCX and the delta in XMM1.
    let outcome = unsafe { ds2_dialog_skip::install() };
    if !outcome.installed {
        log_line(format_args!(
            "{} NOT INSTALLED -- the title message boxes will still need a button press",
            ds2_dialog_skip::LOG_PREFIX
        ));
    }
}

/// Install the two remaining title-flow skips, if `<Game>/ds2-mods.toml` asked for them.
///
/// Called from the same post-Arxan position as its two siblings. Last of the three because it is
/// the one whose effect the player sees last: the boot screens go, then the notice boxes, then the
/// press-any-button gate and the "please wait" windows between there and the menu.
fn install_title_skip() {
    let config = title_skip::TitleSkipConfig::load();
    log_line(format_args!("{}", config.describe()));
    if !config.press_any_button
        && !config.process_windows
        && !config.title_animation
        && !config.title_sequence_gate
    {
        return;
    }
    ds2_dialog_skip::set_logger(log_line);
    // SAFETY: both targets are `.pdata` function starts recorded in `ds2-rva`, resolved against the
    // live module base, and neither is Arxan-redirected. The press gate is replaced outright and
    // takes an argument it never reads; the process-window detour fronts `enter` and calls the
    // original FIRST, because that call is what starts the work the window is covering.
    let outcome = unsafe {
        ds2_dialog_skip::install_title(ds2_dialog_skip::TitleRequest {
            press_any_button: config.press_any_button,
            process_windows: config.process_windows,
            hide_process_windows: config.hide_process_windows,
            title_animation: config.title_animation,
            title_sequence_gate: config.title_sequence_gate,
        })
    };
    if config.press_any_button && !outcome.press_any_button {
        log_line(format_args!(
            "{} press-any-button NOT INSTALLED -- the title will still wait for a button",
            ds2_dialog_skip::LOG_PREFIX
        ));
    }
    if config.process_windows && !outcome.process_windows {
        log_line(format_args!(
            "{} process-window NOT INSTALLED -- the wait windows keep their minimum display time",
            ds2_dialog_skip::LOG_PREFIX
        ));
    }
    if config.hide_process_windows && !outcome.hide_process_windows {
        log_line(format_args!(
            "{} show-process-window NOT INSTALLED -- the wait windows will still be drawn",
            ds2_dialog_skip::LOG_PREFIX
        ));
    }
    if config.title_sequence_gate && !outcome.title_sequence_gate {
        log_line(format_args!(
            "{} title-sequence NOT INSTALLED -- the title logo animation will still be waited out",
            ds2_dialog_skip::LOG_PREFIX
        ));
    }
    if config.title_animation && !outcome.title_animation {
        log_line(format_args!(
            "{} title-animation NOT INSTALLED -- the title screen keeps its activation flourish",
            ds2_dialog_skip::LOG_PREFIX
        ));
    }
}

/// Arm the deliberate crash test, if `<Game>/ds2-mods.toml` asked for one.
///
/// Called from the post-Arxan callback and NEVER from `DllMain`, for the same reason
/// [`install_probe`] is: this runs at the entry point after `DllMain` has returned, so spawning a
/// thread here is not spawning it under the loader lock.
///
/// When it fires the game dies, on purpose. That is the only way to exercise the fatal path --
/// the top-level filter and the minidump tier -- which a first-chance exception cannot reach.
fn arm_fault(config: crash_logging::CrashConfig) {
    // The re-assert first: it is the one that matters on an ordinary run, and on a crash-test run
    // it has to be scheduled before the fault it exists to make visible.
    if let Some(line) = crash_logging::schedule_filter_reinstall(config) {
        log_line(format_args!("{LOG_PREFIX} {line}"));
    }
    if let Some(line) = crash_logging::arm_deliberate_fault(config) {
        log_line(format_args!("{LOG_PREFIX} {line}"));
    }
}

/// Compose the line that opens the log: what this is, what version, and **which file on disk**
/// the loader actually mapped.
///
/// The module path is the load-bearing part. It distinguishes the DLL staged into the game
/// directory from any other `dinput8.dll` that could have won the search order, which is the
/// first thing to check when a run behaves like a build that is not the one under test.
///
/// It deliberately does NOT carry a git sha or a build timestamp. `../er-mods-rs` bakes those
/// in via a `build.rs` (`er-game-base::build_id`), and that was not ported -- so inventing a
/// half-version of it here would be new machinery, not a port. The build identity for a run is
/// the SHA-256 `scripts/ds2-run.py` prints for the staged file immediately before launching;
/// this line says which file, that hash says which bytes.
fn identity_line(module: *mut c_void) -> String {
    let name = env!("CARGO_PKG_NAME");
    let version = env!("CARGO_PKG_VERSION");
    match module_file_name(module) {
        Some(path) => format!("{name} {version} module={}", path.display()),
        None => format!(
            "{name} {version} module=<GetModuleFileNameW failed: {}>",
            unsafe { GetLastError() }
        ),
    }
}

/// Full path of the loaded image `module` was handed for, or `None` if the call failed.
fn module_file_name(module: *mut c_void) -> Option<PathBuf> {
    // Windows paths are not bounded by `MAX_PATH` in general. Grow until the call stops filling
    // the buffer exactly, which is the documented "truncated" signal for this API -- it returns
    // the copied length and sets `ERROR_INSUFFICIENT_BUFFER` rather than the required size.
    let mut capacity = 260usize;
    loop {
        let mut buffer = vec![0u16; capacity];
        let written = unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), capacity as u32) };
        if written == 0 {
            return None;
        }
        if (written as usize) < capacity {
            buffer.truncate(written as usize);
            return Some(PathBuf::from(String::from_utf16_lossy(&buffer)));
        }
        if capacity >= 32_768 {
            // The NT path limit. Past here the call is not going to succeed and doubling
            // forever would be an allocation loop in `DllMain`.
            return None;
        }
        capacity *= 2;
    }
}

/// Append one line to the log next to the game executable, and push it to the OS before
/// returning.
///
/// # A buffered line that dies with the process proves nothing
///
/// This is the runtime test's only evidence, and the failure modes it is evidence *for* include
/// "the game crashed seconds later". So: [`std::fs::File`] is unbuffered, each call opens,
/// writes and closes, and `sync_all` is called before the handle drops. After that the line is
/// on disk and survives the process dying where it stands.
///
/// Every failure is swallowed. A read-only game directory must cost lines in a log, never a
/// panic unwinding out of the loader on the game's startup thread.
fn log_line(args: std::fmt::Arguments<'_>) {
    let Some(path) = ds2_game_base::log::game_directory_path().map(|dir| dir.join(LOG_FILE_NAME))
    else {
        return;
    };
    // `open_fresh_run_append` is the sanctioned opener: it rotates the previous run's file to
    // `.prev` and truncates on this process's first write, so this log describes exactly one
    // process run. `scripts/ds2-run.py` reads the log with an inode+offset tail that survives
    // that rotation.
    if let Some(mut file) = ds2_game_base::log::open_fresh_run_append(&path) {
        let _ = writeln!(file, "{args}");
        let _ = file.sync_all();
    }
}

/// The one named export `DarkSoulsII.exe` imports from `DINPUT8.dll`, forwarded verbatim to the
/// real system DLL.
///
/// Signature is `dinput.h`'s:
/// `HRESULT WINAPI DirectInput8Create(HINSTANCE, DWORD, REFIID, LPVOID*, LPUNKNOWN)`. `REFIID`
/// is a `const IID*` at the ABI level. Every argument is passed straight through untouched --
/// this proxy inspects nothing and owns nothing.
///
/// # Safety
///
/// Called by the game through its import thunk. All pointer arguments are forwarded unmodified
/// to the system implementation, which is the sole owner of their contracts.
#[unsafe(no_mangle)]
#[allow(
    non_snake_case,
    reason = "the export name IS the ABI -- the game's import descriptor asks the loader for \
              this exact spelling, so renaming it unbinds the import and the game will not start"
)]
pub unsafe extern "system" fn DirectInput8Create(
    hinst: *mut c_void,
    version: u32,
    riidltf: *const c_void,
    ppv_out: *mut *mut c_void,
    punk_outer: *mut c_void,
) -> i32 {
    let Some(real) = real_direct_input8_create() else {
        return E_FAIL;
    };
    unsafe { real(hinst, version, riidltf, ppv_out, punk_outer) }
}

type DirectInput8CreateFn = unsafe extern "system" fn(
    *mut c_void,
    u32,
    *const c_void,
    *mut *mut c_void,
    *mut c_void,
) -> i32;

/// Resolve the system `dinput8.dll`'s `DirectInput8Create` once, on first call.
///
/// # Why this is NOT done in `DllMain`
///
/// `LoadLibraryW` under the loader lock is the textbook DLL-init deadlock. It is also
/// unnecessary: the game calls this export from its own code, long after the loader lock is
/// released, so lazy resolution is both safe and free.
fn real_direct_input8_create() -> Option<DirectInput8CreateFn> {
    // `Option` inside the `OnceLock` on purpose: a failed resolution is latched too. If the
    // system DLL is not reachable it will not become reachable, and retrying per call would
    // mean a `LoadLibraryW` on every DirectInput call for the life of the process.
    static REAL: OnceLock<Option<usize>> = OnceLock::new();
    let address = (*REAL.get_or_init(resolve_real_direct_input8_create))?;
    // SAFETY: `address` is a non-null `GetProcAddress` result for `DirectInput8Create` in the
    // system `dinput8.dll`, whose signature is the one `DirectInput8CreateFn` spells.
    Some(unsafe { core::mem::transmute::<usize, DirectInput8CreateFn>(address) })
}

fn resolve_real_direct_input8_create() -> Option<usize> {
    // Bound to a local rather than inlined into the call: the buffer must outlive the call that
    // reads it, and a named binding says so instead of relying on temporary-lifetime rules.
    let path = system_dinput8_path()?;
    let module = unsafe { LoadLibraryW(path.as_ptr()) };
    if module.is_null() {
        return None;
    }
    // NUL-terminated, because `GetProcAddress` takes a C string and not a Rust one.
    let address = unsafe { GetProcAddress(module, c"DirectInput8Create".as_ptr().cast()) };
    (!address.is_null()).then_some(address as usize)
}

/// `<system directory>\dinput8.dll`, NUL-terminated, ready for `LoadLibraryW`.
///
/// # Absolute path, resolved at runtime
///
/// Two reasons, and both are load-bearing:
///
/// * **Absolute**, so the loader does not find *us*. This DLL is itself named `dinput8.dll` and
///   sits in the game directory, which is at the front of the default search order -- a bare
///   `LoadLibraryW("dinput8.dll")` would map the proxy again and the forward would recurse.
/// * **Resolved, not hardcoded.** `C:\windows\system32` is a guess about someone else's
///   machine. Under Proton the system directory is the one inside the wineprefix, and the
///   builtin `dinput8.dll` this forward targets is the one that lives there.
fn system_dinput8_path() -> Option<Vec<u16>> {
    // `GetSystemDirectoryW` returns the length WITHOUT the terminator on success, or the
    // required size INCLUDING it when the buffer was too small. Ask once, size from the answer.
    let mut buffer = vec![0u16; 260];
    let mut written = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    if written as usize >= buffer.len() {
        buffer = vec![0u16; written as usize];
        written = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    }
    if written == 0 || written as usize >= buffer.len() {
        return None;
    }
    buffer.truncate(written as usize);
    // The documented exception: the returned path has no trailing backslash unless the system
    // directory is a drive root, in which case it does. Appending unconditionally would produce
    // `C:\\dinput8.dll`.
    const BACKSLASH: u16 = b'\\' as u16;
    if buffer.last() != Some(&BACKSLASH) {
        buffer.push(BACKSLASH);
    }
    buffer.extend("dinput8.dll".encode_utf16());
    buffer.push(0);
    Some(buffer)
}
