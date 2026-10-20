//! Re-reading `<Game>/ds2-mods.toml` while the game runs, for the keys that can change live.
//!
//! Live keys: `[inventory_sort] up_enters_list`, `[continue] hollow_slot_guard`, and the three
//! bonfire rows (`[change_appearance]`, `[rename_character]`, `[reallocate_stats]` `enabled`).
//!
//! One thread polls the file by content through `ds2_hotkey_config::reload::HotFile`, about once a
//! second, and pushes what it resolved into each feature's own switch. A feature that was off at
//! boot was never hooked, so the first time its key turns on it is installed then; after that,
//! turning it off only clears its switch and the hooks stay in, passing through.
//!
//! Started after the boot batch is applied, so an install from here patches at once instead of
//! being queued behind a batch nothing will apply.

use std::sync::atomic::{AtomicBool, Ordering};

use ds2_hotkey_config::reload::{FileChange, HotFile};

use crate::{change_appearance, continue_flow, crash_logging, inventory_sort, log_line};

const LOG_PREFIX: &str = "ds2-loader: live-config";

/// Whether `ds2_inventory_sort::install` succeeded, at boot or here.
pub static INVENTORY_INSTALLED: AtomicBool = AtomicBool::new(false);
/// Whether `ds2_change_appearance::install` succeeded, at boot or here.
pub static BONFIRE_INSTALLED: AtomicBool = AtomicBool::new(false);

/// What the live keys resolved to, so a reload that changed only other keys does nothing.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Live {
    up_enters_list: bool,
    hollow_slot_guard: bool,
    rows: ds2_change_appearance::Rows,
}

impl Live {
    fn load() -> Self {
        Self {
            up_enters_list: inventory_sort::InventorySortConfig::load().up_enters_list,
            hollow_slot_guard: continue_flow::ContinueConfig::load().hollow_slot_guard,
            rows: change_appearance::ChangeAppearanceConfig::load().rows(),
        }
    }
}

/// Start the watcher thread. Call once, after the boot batch has been applied.
pub fn spawn() {
    let Some(path) = crash_logging::config_file_path() else {
        log_line(format_args!(
            "{LOG_PREFIX} not started -- no config path, so nothing to watch"
        ));
        return;
    };
    let spawned = std::thread::Builder::new()
        .name("ds2-live-config".into())
        .spawn(move || {
            let mut file = HotFile::new(path);
            // The first poll reads the file the boot installs already acted on; it sets the
            // baseline rather than counting as a change.
            let _ = file.poll();
            let mut current = Live::load();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(
                    ds2_hotkey_config::reload::DEFAULT_POLL_INTERVAL_MS,
                ));
                if !matches!(file.poll(), Some(FileChange::Text(_))) {
                    continue;
                }
                let next = Live::load();
                if next != current {
                    apply(current, next);
                    current = next;
                }
            }
        });
    match spawned {
        Ok(_) => log_line(format_args!(
            "{LOG_PREFIX} watching up_enters_list, hollow_slot_guard and the bonfire rows"
        )),
        Err(error) => log_line(format_args!(
            "{LOG_PREFIX} not started -- thread spawn failed: {error}"
        )),
    }
}

fn apply(was: Live, now: Live) {
    if now.up_enters_list != was.up_enters_list {
        set_up_enters_list(now.up_enters_list);
    }
    if now.hollow_slot_guard != was.hollow_slot_guard {
        set_hollow_slot_guard(now.hollow_slot_guard);
    }
    if now.rows != was.rows {
        set_rows(now.rows);
    }
}

fn set_up_enters_list(on: bool) {
    log_line(format_args!(
        "{LOG_PREFIX} [inventory_sort] up_enters_list={on}"
    ));
    ds2_inventory_sort::set_up_enters_list(on);
    if !on || INVENTORY_INSTALLED.load(Ordering::Acquire) {
        return;
    }
    // Off at boot, so the menu detours were never installed. The sort button stays off: its tick
    // registry is sealed by now, and `enabled` is read at boot only.
    ds2_inventory_sort::set_logger(log_line);
    let request = ds2_inventory_sort::Request {
        config_path: crash_logging::config_file_path(),
        sort_button: false,
        up_enters_list: true,
    };
    // SAFETY: the same prologue-checked targets the boot install patches; with `sort_button` off
    // the install registers no tick, so the sealed registry is not touched.
    let outcome = unsafe { ds2_inventory_sort::install(&request) };
    INVENTORY_INSTALLED.store(outcome.installed, Ordering::Release);
    if !outcome.installed {
        log_line(format_args!(
            "{} NOT INSTALLED live -- UP on the strip stays as shipped",
            ds2_inventory_sort::LOG_PREFIX
        ));
    }
}

fn set_hollow_slot_guard(on: bool) {
    log_line(format_args!(
        "{LOG_PREFIX} [continue] hollow_slot_guard={on}"
    ));
    ds2_continue::set_hollow_slot_guard(on);
    if on && !ds2_continue::hollow_slot_guard_installed() {
        // SAFETY: the walk's prologue is checked against `ds2-rva` before anything is patched.
        unsafe { ds2_continue::install_hollow_slot_guard() };
    }
}

fn set_rows(rows: ds2_change_appearance::Rows) {
    log_line(format_args!("{LOG_PREFIX} bonfire rows {rows:?}"));
    if BONFIRE_INSTALLED.load(Ordering::Acquire) {
        // SAFETY: `install` succeeded; the only patch this can make is the set-text hook.
        unsafe { ds2_change_appearance::set_rows(rows) };
        return;
    }
    if !rows.any() {
        return;
    }
    ds2_change_appearance::set_logger(log_line);
    // SAFETY: the patch site's bytes are re-read and nothing is written on a mismatch; every hook
    // target is recorded in `ds2-rva`. Same call the boot install makes.
    let outcome = unsafe { ds2_change_appearance::install(rows) };
    BONFIRE_INSTALLED.store(outcome.installed, Ordering::Release);
    if !outcome.installed {
        log_line(format_args!(
            "{} NOT INSTALLED live -- no bonfire rows",
            ds2_change_appearance::LOG_PREFIX
        ));
    }
}
