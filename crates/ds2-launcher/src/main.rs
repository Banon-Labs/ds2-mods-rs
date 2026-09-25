//! A launcher that starts DARK SOULS II suspended and injects the DLLs a config file names.
//!
//! # Why this exists rather than `ds2sc_launcher.exe`
//!
//! Seamless Co-op cannot be loaded from inside the game process. That was measured across four
//! slots -- after the Arxan callback, inline in `ds2-loader`'s attach, on a dedicated thread with
//! the entry point held, and a control -- and every in-process `LoadLibraryW` mapped the DLL and
//! left it inert: no settings read, no hooks installed, the game still opening its vanilla save
//! container. The control ran the mod's own launcher and it initialised. What the mod does not
//! survive is a process whose main thread has already run `LdrInitializeThunk`, which a
//! statically imported DLL cannot avoid, so the load has to happen from outside before the game
//! starts.
//!
//! Running the mod's launcher works and costs two things. It can only ever load that one mod,
//! and it is someone else's binary with two gates that stop the boot on a dialog rather than on
//! a log line. This launcher does the same job for a list.
//!
//! # What it reads
//!
//! `<Game>/ds2-mods.toml`, the same file `ds2-loader` reads once it is in the process. See
//! [`plan`] for the section and the ordering rules.
//!
//! # What it is not
//!
//! Not a replacement for `dinput8.dll`. This workspace's own loader is a static import of
//! `DarkSoulsII.exe` and needs no injecting; this launcher only adds the mods that cannot get in
//! that way.

mod plan;

#[cfg(windows)]
mod inject;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use plan::{LOG_PREFIX, Plan};

/// The config file, beside the game executable. Mirrored in `scripts/ds2-run.py`.
const CONFIG_FILE_NAME: &str = "ds2-mods.toml";

/// Exit code for a refusal: something the config asked for is not on disk.
const EXIT_REFUSED: u8 = 1;

/// Exit code for a failure after the process existed.
const EXIT_FAILED: u8 = 2;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let dry_run = arguments.iter().any(|argument| argument == "--dry-run");
    let selftest = arguments.iter().any(|argument| argument == "--selftest");

    if selftest {
        return selftest_main();
    }

    let game_dir = game_directory();
    let config_path = game_dir.join(CONFIG_FILE_NAME);
    let text = std::fs::read_to_string(&config_path).unwrap_or_default();
    if text.is_empty() {
        println!(
            "{LOG_PREFIX} no config at {} -- vanilla plus whatever `dinput8.dll` does",
            config_path.display()
        );
    }

    let plan = Plan::from_text(&text, &game_dir);
    println!("{LOG_PREFIX} exe {}", plan.exe.display());
    if plan.injections.is_empty() {
        println!("{LOG_PREFIX} nothing to inject");
    }
    for (index, injection) in plan.injections.iter().enumerate() {
        println!(
            "{LOG_PREFIX} inject {} of {}: {} -> {} (from `{}`)",
            index + 1,
            plan.injections.len(),
            injection.spelled,
            injection.resolved.display(),
            injection.source.describe()
        );
    }

    let refusals = plan.refusals();
    if !refusals.is_empty() {
        for refusal in &refusals {
            println!("{LOG_PREFIX} REFUSING: {}", refusal.describe());
        }
        println!(
            "{LOG_PREFIX} nothing was started. A session gets the whole list or does not exist, \
             so fix the above and run again."
        );
        return ExitCode::from(EXIT_REFUSED);
    }

    if dry_run {
        println!("{LOG_PREFIX} --dry-run: the plan above is runnable; nothing was started");
        return ExitCode::SUCCESS;
    }

    run(&plan, &game_dir)
}

/// Start the plan, on a platform that can.
#[cfg(windows)]
fn run(plan: &Plan, game_dir: &Path) -> ExitCode {
    match inject::launch(plan, game_dir) {
        Ok(launched) => {
            println!(
                "{LOG_PREFIX} started pid {} with {} DLL(s) in it, every one of them confirmed \
                 loaded by its own injected thread",
                launched.process_id,
                plan.injections.len()
            );
            ExitCode::SUCCESS
        }
        Err(failure) => {
            println!("{LOG_PREFIX} FAILED: {failure}");
            println!("{LOG_PREFIX} the process was terminated -- nothing half-modded is running");
            ExitCode::from(EXIT_FAILED)
        }
    }
}

/// The same entry point on a host build, which can plan but cannot launch.
///
/// The crate builds on Linux so `plan` can be unit-tested there; this arm exists so that build
/// links, and says plainly that it is not the product.
#[cfg(not(windows))]
fn run(_plan: &Plan, _game_dir: &Path) -> ExitCode {
    println!(
        "{LOG_PREFIX} this is a host build and cannot start a Windows process. The plan above is \
         what the Windows build would do; `--dry-run` says so without this warning."
    );
    ExitCode::from(EXIT_FAILED)
}

/// Where the game is: the directory holding this executable.
///
/// Taken from the executable's own path rather than the working directory, because a launcher
/// started from a shortcut, from Steam, or through a Proton chain does not reliably inherit the
/// working directory it is sitting in -- and resolving a mod list against the wrong directory
/// would refuse every entry with a message that looked like the files were missing.
fn game_directory() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Prove the parts of this binary that can be proven without a game.
///
/// Runs the planner against config text held here rather than read off disk, and prints what it
/// decided. `scripts/check.sh` can call this on any platform.
fn selftest_main() -> ExitCode {
    let game = Path::new("/games/ds2");
    let text = "[launcher]\ndlls = [\"SeamlessCoop/ds2sc.dll\", \"Other/other.dll\"]\n\
                [seamless]\nenabled = true\n";
    let plan = Plan::from_text(text, game);

    let mut failures = Vec::new();
    if plan.injections.len() != 2 {
        failures.push(format!(
            "expected 2 injections after deduplication, got {}",
            plan.injections.len()
        ));
    }
    if plan.injections.first().map(|first| first.source) != Some(plan::Source::Seamless) {
        failures.push("seamless should lead the list".to_owned());
    }
    if plan.exe != game.join(plan::DEFAULT_EXE) {
        failures.push(format!("wrong default exe: {}", plan.exe.display()));
    }
    // The planner must refuse a plan whose files are not there, or the all-or-nothing contract
    // in `plan::Refusal` is not being enforced by anything.
    if plan.refusals().is_empty() {
        failures.push("a plan over a directory that does not exist must be refused".to_owned());
    }

    for failure in &failures {
        println!("{LOG_PREFIX} selftest: {failure}");
    }
    if failures.is_empty() {
        println!("{LOG_PREFIX} selftest: ok (planning, ordering, deduplication, refusal)");
        ExitCode::SUCCESS
    } else {
        ExitCode::from(EXIT_FAILED)
    }
}
