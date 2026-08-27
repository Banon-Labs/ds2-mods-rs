//! Tier A: fresh-per-process file logger + game-directory resolver.
//!
//! Parameterized by target path, so a caller keeps its own named wrapper (`append_trace`,
//! `append_debug`, whatever it is) over these primitives and log paths and prefixes stay owned
//! by the caller rather than by the substrate. Ported from `../er-mods-rs`'s
//! `er-game-base::log`.
//!
//! # A log describes exactly ONE process run
//!
//! Standing rule, inherited from the sibling repo (2026-08-04): no DLL, shell or harness may
//! append to a log ACROSS runs. Every log file is truncated by the first write of the process
//! that owns it; keeping an older run means copying the file aside yourself, not letting it
//! accumulate.
//!
//! The concrete failure that set the rule: a mod DLL opened its log with a plain `append(true)`
//! on a fixed name next to the game executable. Twelve separate launches piled into one 565 KB
//! file, so a count taken over it ("37 confirms") read as ONE run doing something 37 times when
//! it was really twelve runs -- and per-run state could only be recovered by hand-splitting on
//! the module-base banner. Worse, lines from builds that no longer existed sat
//! indistinguishably next to lines from the build under test.
//!
//! [`begin_fresh_run`] is the one-shot that enforces it, and [`open_fresh_run_append`] is the
//! only sanctioned way to open a log for append.
//!
//! **In `../er-mods-rs` that rule is executable**: `scripts/check-fresh-run-logs.py` fails the
//! build on any `.append(...)` opener outside this module. No such check runs in this repo yet,
//! so here it is a convention and nothing more -- which means it holds until someone forgets.
//! Tracked as `ds2-mods-rs-gvw`.
//!
//! # Nothing here knows what game it is logging
//!
//! [`game_directory_path`] resolves the directory of whatever executable is running; it does not
//! name one. The identity line is a caller-supplied string ([`set_identity_line`]) rather than
//! something this crate composes, because composing it means knowing which build of what -- and
//! that is the kind of knowledge this crate exists not to have.

use std::ffi::OsString;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Suffix the PREVIOUS run's file is renamed to when this process freshens a log.
///
/// Exactly ONE generation is kept, and it is deliberately NOT `.prev.log`: a reader
/// or harness globbing `*.log` must not pick the stale generation up as if it were
/// live. This is not a rotation system -- run N-2 is gone.
///
/// The invariant is that `<name>.prev` holds the run IMMEDIATELY before the live file,
/// or does not exist. A harness that deletes the log before launching (`rm -f
/// "$GAME_DIR"/ds2-*.log`) leaves nothing to rotate; the older `.prev` is dropped in that case
/// rather than left sitting next to a fresh log looking one run old when it is three. Keeping a
/// run means copying it somewhere of your own.
pub const PREVIOUS_RUN_SUFFIX: &str = ".prev";

/// Log paths this process has already freshened. One entry per log file (a handful
/// at most), touched only on the first write to each path.
static FRESHENED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// The line written at the top of every log this process freshens, if a caller set one.
///
/// A `OnceLock` and not a `Mutex<Option<..>>` because a process has exactly one identity: the
/// first setter wins, later ones are ignored, and every log the process writes therefore opens
/// with the SAME line. A settable-repeatedly identity would let two logs from one run disagree
/// about which binary wrote them, which is the failure this whole mechanism exists to stop.
static IDENTITY_LINE: OnceLock<String> = OnceLock::new();

std::thread_local! {
    /// True while THIS thread is inside [`begin_fresh_run`].
    static FRESHENING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Directory the game exe lives in — everything writes artifacts relative to it.
pub fn game_directory_path() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from))
}

/// Declare the line that opens every log this process freshens. First call wins.
///
/// # THE FIRST LINE OF A LOG SHOULD SAY WHICH BINARY WROTE IT
///
/// A log arrives from a tester with a symptom in it and the first question is always the same:
/// which build is that? In `../er-mods-rs` a log had to be dated by string-matching its own
/// format literals against the repo, because its opening line was `loaded module_base=0x…` and
/// nothing else -- no commit, no build time, no file name. That is archaeology in place of a
/// field the writer already had.
///
/// # Why this is a parameter and not a function this crate provides
///
/// The sibling repo composes the line here, out of a git sha baked in by `build.rs`, the DLL's
/// own PE timestamp and its module file name. That composition lives in `er-game-base::build_id`
/// and was deliberately NOT ported: it also carries release-tag/roster comparison and overlay
/// state that is product-specific. So the substrate holds the SLOT and the caller fills it,
/// which also means a caller free to say something truer than a sha if it has it.
///
/// Returns `true` if this call set the line, `false` if one was already set (in which case the
/// existing line stands and the argument is discarded).
pub fn set_identity_line(line: impl Into<String>) -> bool {
    IDENTITY_LINE.set(line.into()).is_ok()
}

/// The identity line in force for this process, or `None` if no caller set one.
///
/// Stable once set, so a test or a reader can compare a log's first line against it.
pub fn identity_line() -> Option<&'static str> {
    IDENTITY_LINE.get().map(String::as_str)
}

/// One-shot per (process, path): rotate the previous run's file aside and truncate,
/// so the file that follows describes this process run and nothing else.
///
/// Idempotent. The FIRST call for a path does the work; every later call in the same
/// process is a short lookup, which is what makes "truncate once, append thereafter"
/// different from "truncate on every write" (the latter would lose the run's own
/// earlier lines).
///
/// # Why a re-entrancy guard on a logger
///
/// Both file operations below reach the OS through `kernel32!CreateFileW`, which a mod DLL in
/// this workspace may DETOUR -- and a detour logs. So a rotate can arrive straight back here on
/// the same thread. Re-entering would deadlock on `FRESHENED`; the thread-local latch turns the
/// nested call into a no-op instead. A line about opening a log is worth nothing, and the nested
/// writer simply appends to the file this call is about to truncate.
///
/// # Failure is latched on purpose
///
/// A directory that refuses the truncating open below refuses an appending open too,
/// so retrying on the next line buys nothing but a syscall per line.
pub fn begin_fresh_run(path: &Path) {
    let entered = FRESHENING
        .try_with(|freshening| !freshening.replace(true))
        .unwrap_or(false);
    if !entered {
        return;
    }
    struct ReleaseOnDrop;
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            let _ = FRESHENING.try_with(|freshening| freshening.set(false));
        }
    }
    let _release = ReleaseOnDrop;

    {
        let mut freshened = FRESHENED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if freshened.iter().any(|seen| seen == path) {
            return;
        }
        freshened.push(path.to_path_buf());
    }

    let mut previous: OsString = path.as_os_str().to_os_string();
    previous.push(PREVIOUS_RUN_SUFFIX);
    let previous = PathBuf::from(previous);
    // Unconditional, so `<name>.prev` is never older than one run: Windows `rename` refuses
    // an existing destination anyway, and when the live file is absent (a harness cleared it
    // pre-launch) there is nothing to preserve and the stale generation should not survive.
    let _ = fs::remove_file(&previous);
    if path.exists() {
        let _ = fs::rename(path, &previous);
    }
    if let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
    {
        // The identity line goes here, in the one-shot every sanctioned opener already routes
        // through, rather than being left to each DLL to log at boot -- that is a rule that
        // holds until someone adds the twentieth shell and forgets.
        //
        // Failure is ignored for the same reason every other write here ignores it: a read-only
        // game directory must degrade to fewer lines, never to a panic on the game thread.
        if let Some(identity) = identity_line() {
            let _ = writeln!(file, "{identity}");
        }
    }
}

/// THE sanctioned way to open a log for append in this repo.
///
/// Freshens the file on this process's first call for `path`, then hands back an
/// appending handle. Callers may keep the handle for the life of the process (hot
/// paths should) or drop it per line (low-frequency callers).
pub fn open_fresh_run_append(path: &Path) -> Option<fs::File> {
    begin_fresh_run(path);
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()
}

/// Append one line to `path`, creating it if absent and truncating it once per
/// process. Opens/appends/closes per call (simple, low-frequency callers). For hot
/// paths prefer a caller-owned persistent handle over [`open_fresh_run_append`].
pub fn append_line(path: &std::path::Path, args: std::fmt::Arguments<'_>) {
    if let Some(mut file) = open_fresh_run_append(path) {
        let _ = writeln!(file, "{args}");
    }
}

/// Truncate-then-open `path` for a clean per-process log, invoking `header` to
/// write a banner line once. Returns the open handle so the caller can retain a
/// persistent `Mutex<Option<File>>` and avoid per-call open/close syscalls.
///
/// Routes through [`begin_fresh_run`] so the previous run's file is rotated aside
/// rather than destroyed, matching every other writer.
pub fn open_truncated_with_header(
    path: &std::path::Path,
    header: impl FnOnce(&mut fs::File),
) -> Option<fs::File> {
    begin_fresh_run(path);
    // APPEND, not truncate. `begin_fresh_run` has already emptied the file and written the
    // identity line into it; opening with `.truncate(true)` here would delete that line and
    // leave exactly the logs that most need identifying -- the ones with a persistent handle
    // and a banner -- as the only ones without it.
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()?;
    header(&mut file);
    Some(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every test in this module sets the identity line and then reads back whichever value
    /// won, rather than asserting a literal. `IDENTITY_LINE` is process-wide and cargo runs
    /// these threads in parallel, so a literal assertion would be a race that passes on a
    /// quiet machine; comparing against [`identity_line`] is deterministic because the value
    /// cannot change once set.
    fn expected_identity_prefix() -> String {
        set_identity_line("build test-identity");
        format!("{}\n", identity_line().expect("a setter won"))
    }

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ds2-game-base-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    /// A second write in the SAME process must not lose the first: truncation is
    /// one-shot, not per-write. This is the bug the rule's shape is chosen to avoid.
    #[test]
    fn first_write_truncates_and_later_writes_append() {
        let identity = expected_identity_prefix();
        let dir = scratch_dir("fresh");
        let path = dir.join("one-run.log");
        let _ = fs::write(&path, "STALE FROM AN EARLIER RUN\n");

        append_line(&path, format_args!("first"));
        append_line(&path, format_args!("second"));

        let body = fs::read_to_string(&path).expect("log written");
        assert_eq!(
            body,
            format!("{identity}first\nsecond\n"),
            "expected the identity line then both of this run's lines, and nothing older"
        );

        let previous = fs::read_to_string(dir.join("one-run.log.prev")).expect("rotated aside");
        assert_eq!(previous, "STALE FROM AN EARLIER RUN\n");
        let _ = fs::remove_dir_all(&dir);
    }

    /// `<name>.prev` must hold the run immediately before the live file, or nothing. A
    /// harness that clears the log pre-launch leaves nothing to rotate, and a `.prev` left
    /// over from an older run would read as "the run before this one" when it is not.
    #[test]
    fn a_cleared_log_does_not_leave_an_older_generation_behind() {
        let identity = expected_identity_prefix();
        let dir = scratch_dir("cleared");
        let path = dir.join("cleared.log");
        // Live file absent (harness `rm -f`'d it), stale generation still on disk.
        let _ = fs::write(dir.join("cleared.log.prev"), "THREE RUNS AGO\n");

        append_line(&path, format_args!("only line"));

        let body = fs::read_to_string(&path).expect("log written");
        assert_eq!(body, format!("{identity}only line\n"));
        assert!(
            !dir.join("cleared.log.prev").exists(),
            "a stale generation survived next to a fresh log"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// A caller-supplied banner must land AFTER the identity line, not instead of it. The
    /// persistent-handle path is the one that historically lost it, by re-truncating.
    #[test]
    fn a_header_banner_does_not_displace_the_identity_line() {
        let identity = expected_identity_prefix();
        let dir = scratch_dir("header");
        let path = dir.join("banner.log");

        {
            let mut file = open_truncated_with_header(&path, |file| {
                let _ = writeln!(file, "banner");
            })
            .expect("log opened");
            let _ = writeln!(file, "after");
        }

        let body = fs::read_to_string(&path).expect("log written");
        assert_eq!(body, format!("{identity}banner\nafter\n"));
        let _ = fs::remove_dir_all(&dir);
    }

    /// The identity is fixed for the life of the process: a second setter must not be able to
    /// make two logs from one run disagree about who wrote them.
    #[test]
    fn the_identity_line_is_set_once_and_later_setters_are_refused() {
        let identity = expected_identity_prefix();
        assert!(
            !set_identity_line("build something-else"),
            "a second setter must be refused"
        );
        assert_eq!(format!("{}\n", identity_line().expect("set")), identity);
    }
}
