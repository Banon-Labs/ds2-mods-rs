//! The Music panel: an imgui window over the game, opened and closed with one key (`F10` unless
//! `[music_probe] key` says otherwise).
//!
//! It never calls FMOD. It reads the player's [`SNAPSHOT`], edits the playlists in
//! [`PLAYLISTS`] (saving the file on every edit), and queues [`Command`]s the sound thread runs.
//! While it is open the input harness holds the game's own devices still, and the overlay feeds
//! imgui the corrected cursor so the slider and buttons are where the pointer is.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use hudhook::imgui::{Condition, Ui};
use hudhook::windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use hudhook::windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId,
};

use crate::engine::{Command, SNAPSHOT, Snapshot, push};
use crate::install::log;
use crate::playlist::{Region, TrackKey};
use crate::store::{PLAYLISTS, save};
use crate::{DEFAULT_KEY, LOG_PREFIX, minutes_seconds};

const VK_ESCAPE: u32 = 0x1b;

/// The Music window's width, in back-buffer pixels. Its height follows its content.
const WINDOW_WIDTH: f32 = 640.0;

/// Frames the panel has drawn, for the once-a-second geometry line.
static FRAMES: AtomicU32 = AtomicU32::new(0);

/// Catalog rows drawn at most, after the filter.
const CATALOG_ROWS: usize = 500;

static OPEN: AtomicBool = AtomicBool::new(false);
/// Closed, but still holding input until the closing key is up.
static CLOSING: AtomicBool = AtomicBool::new(false);
static KEY: AtomicU32 = AtomicU32::new(0);
static KEY_WAS_DOWN: AtomicBool = AtomicBool::new(false);
static ESCAPE_WAS_DOWN: AtomicBool = AtomicBool::new(false);

struct PanelState {
    filter: String,
    /// The slider's value while it is being dragged, in seconds.
    seek_drag: Option<f32>,
}

static STATE: Mutex<PanelState> = Mutex::new(PanelState {
    filter: String::new(),
    seek_drag: None,
});

/// Register the panel and its key with `ds2-overlay`. `ds2-overlay` must already be started.
/// `false` has been logged.
pub fn install_panel(key: &str) -> bool {
    let vk = match ds2_hotkey_config::keys::parse_virtual_key(key) {
        Ok(vk) => vk,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} panel key {key:?} is not a key ({error:?}); using {DEFAULT_KEY}"
            ));
            ds2_hotkey_config::keys::parse_virtual_key(DEFAULT_KEY).unwrap_or(0x79)
        }
    };
    KEY.store(vk, Ordering::Release);
    if !ds2_overlay::panels::add_panel(draw, is_open, Some(wants_input)) {
        log(format_args!(
            "{LOG_PREFIX} every ds2-overlay panel slot is taken -- no Music panel this run"
        ));
        return false;
    }
    if !ds2_overlay::frame_hook::add_frame_hook(on_frame) {
        log(format_args!(
            "{LOG_PREFIX} every ds2-overlay frame-hook slot is taken -- the Music panel key does \
             nothing"
        ));
        return false;
    }
    log(format_args!(
        "{LOG_PREFIX} panel installed key={key} vk=0x{vk:02x}"
    ));
    true
}

fn is_open() -> bool {
    OPEN.load(Ordering::Acquire)
}

fn wants_input() -> bool {
    OPEN.load(Ordering::Acquire) || CLOSING.load(Ordering::Acquire)
}

fn key_down(vk: u32) -> bool {
    // SAFETY: `GetAsyncKeyState` takes a virtual-key code and returns a bitfield.
    (unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000) != 0
}

/// Is the game the window the keyboard is talking to?
fn game_has_focus() -> bool {
    // SAFETY: two `user32` calls; the only pointer is to an owned local.
    unsafe {
        let window = GetForegroundWindow();
        if window.is_invalid() {
            return false;
        }
        let mut process = 0u32;
        GetWindowThreadProcessId(window, Some(&mut process));
        process == std::process::id()
    }
}

/// Once per `Present`: the key toggles the panel, Escape closes it, and the hold is released once
/// the closing key is up so the press never reaches the game.
fn on_frame() {
    let focus = game_has_focus();
    let key = focus && key_down(KEY.load(Ordering::Acquire));
    let escape = focus && key_down(VK_ESCAPE);
    let key_pressed = key && !KEY_WAS_DOWN.swap(key, Ordering::AcqRel);
    if !key {
        KEY_WAS_DOWN.store(false, Ordering::Release);
    }
    let escape_pressed = escape && !ESCAPE_WAS_DOWN.swap(escape, Ordering::AcqRel);
    if !escape {
        ESCAPE_WAS_DOWN.store(false, Ordering::Release);
    }
    if OPEN.load(Ordering::Acquire) {
        if key_pressed || escape_pressed {
            close();
        }
    } else if CLOSING.load(Ordering::Acquire) {
        if !key && !escape {
            CLOSING.store(false, Ordering::Release);
            ds2_overlay::panels::use_overlay_mouse_for_imgui(false);
            ds2_input_harness::hold(false);
        }
    } else if key_pressed {
        open();
    }
}

fn open() {
    ds2_input_harness::hold(true);
    ds2_overlay::panels::use_overlay_mouse_for_imgui(true);
    OPEN.store(true, Ordering::Release);
    log(format_args!("{LOG_PREFIX} panel open"));
}

fn close() {
    CLOSING.store(true, Ordering::Release);
    OPEN.store(false, Ordering::Release);
    log(format_args!("{LOG_PREFIX} panel closed"));
}

fn draw(ui: &Ui) {
    if !OPEN.load(Ordering::Acquire) {
        return;
    }
    let snapshot = SNAPSHOT
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_default();
    let Ok(mut state) = STATE.lock() else {
        return;
    };
    let mut open = true;
    let display = ui.io().display_size;
    // Sized to its content, and never larger than the screen it is drawn on.
    ui.window("Music")
        .position([40.0, 40.0], Condition::FirstUseEver)
        .size_constraints([WINDOW_WIDTH, 0.0], [WINDOW_WIDTH, display[1] - 80.0])
        .always_auto_resize(true)
        .opened(&mut open)
        .build(|| {
            let frame = FRAMES.fetch_add(1, Ordering::Relaxed);
            if frame.is_multiple_of(60) {
                log(format_args!(
                    "{LOG_PREFIX} panel window pos={:?} size={:?} display={display:?}",
                    ui.window_pos(),
                    ui.window_size()
                ));
            }
            let _wrap = ui.push_text_wrap_pos();
            body(ui, &snapshot, &mut state);
        });
    if !open {
        close();
    }
}

fn body(ui: &Ui, snapshot: &Snapshot, state: &mut PanelState) {
    ui.text_disabled("F10 or Esc closes this window.");
    match &snapshot.now {
        Some(key) => ui.text(format!(
            "Now playing: {key}  ({})",
            if snapshot.ours {
                "playlist track"
            } else {
                "the game's own track"
            }
        )),
        None if snapshot.silent => {
            ui.text("Stopped: repeat is off and the playlist has played out.");
        }
        None => ui.text("Nothing playing."),
    }
    if snapshot.waiting {
        ui.text_disabled("The playlist starts once the game's own track is heard after the load.");
    }
    if let Some(problem) = &snapshot.problem {
        ui.text_colored(
            [1.0, 0.6, 0.4, 1.0],
            format!("Playing the region's own track: {problem}"),
        );
    }
    if let (Some(position), Some(length)) = (snapshot.position_ms, snapshot.length_ms) {
        let mut seconds = state.seek_drag.unwrap_or(position as f32 / 1000.0);
        let max = length as f32 / 1000.0;
        if ui
            .slider_config("##seek", 0.0f32, max)
            .display_format("%.0f s")
            .build(&mut seconds)
        {
            state.seek_drag = Some(seconds);
        }
        if ui.is_item_deactivated_after_edit() {
            let ms = (seconds.clamp(0.0, max) * 1000.0) as u32;
            log(format_args!("{LOG_PREFIX} panel seek to={ms}ms"));
            push(Command::Seek(ms));
            state.seek_drag = None;
        }
        ui.same_line();
        ui.text(format!(
            "{} / {}",
            minutes_seconds(position),
            minutes_seconds(length)
        ));
    }
    if ui.button("Previous") {
        push(Command::Step(false));
    }
    ui.same_line();
    if ui.button("Next") {
        push(Command::Step(true));
    }
    ui.separator();

    let Some(region_key) = snapshot.region.clone() else {
        ui.text("No region track yet: the game has not started any region music.");
        return;
    };
    ui.text(format!(
        "Region: {region_key}{}",
        snapshot
            .map
            .map_or_else(String::new, |m| format!("  (map index {m})"))
    ));
    let Ok(mut lists) = PLAYLISTS.lock() else {
        return;
    };
    let before = lists
        .get(&region_key)
        .cloned()
        .unwrap_or_else(|| Region::new(region_key.clone(), snapshot.map));
    let mut region = before.clone();
    let mut play_now = None;

    let mut repeat = region.repeat;
    if ui.checkbox("Repeat the playing track", &mut repeat) {
        region.repeat = repeat;
    }
    ui.text_disabled("Off: each track plays once, then the next; a one-track playlist stops.");
    let mut include = region.include_default;
    if ui.checkbox("Play the region's own track", &mut include) {
        region.include_default = include;
    }

    ui.text("Playlist:");
    let entries = region.entries();
    if entries.is_empty() {
        ui.text_disabled("  (empty: this region is silent)");
    }
    let mut remove: Option<TrackKey> = None;
    let mut move_up: Option<(TrackKey, bool)> = None;
    for (index, key) in entries.iter().enumerate() {
        let _id = ui.push_id_usize(index);
        let playing = snapshot.index == Some(index) && snapshot.now.as_ref() == Some(key);
        ui.text(format!(
            "{} {}. {key}{}",
            if playing { ">" } else { " " },
            index + 1,
            if *key == region.default {
                "  (region's own)"
            } else {
                ""
            }
        ));
        ui.same_line();
        if ui.small_button("Play") {
            play_now = Some(index);
        }
        if *key != region.default {
            ui.same_line();
            if ui.small_button("Up") {
                move_up = Some((key.clone(), true));
            }
            ui.same_line();
            if ui.small_button("Down") {
                move_up = Some((key.clone(), false));
            }
        }
        ui.same_line();
        if ui.small_button("Remove") {
            remove = Some(key.clone());
        }
    }
    if let Some(key) = remove {
        region.remove(&key);
    }
    if let Some((key, up)) = move_up {
        region.move_track(&key, up);
    }

    ui.separator();
    ui.text("Add a track from any map (click to add):");
    ui.input_text("Filter", &mut state.filter).build();
    ui.text_disabled(format!(
        "{} music tracks{}",
        snapshot.catalog.len(),
        if snapshot.catalog_done {
            ""
        } else {
            " so far -- still reading the game's event list"
        }
    ));
    let filter = state.filter.to_ascii_lowercase();
    ui.child_window("catalog").size([0.0, 260.0]).build(|| {
        let matches = snapshot
            .catalog
            .iter()
            .filter(|key| {
                filter.is_empty() || key.to_string().to_ascii_lowercase().contains(&filter)
            })
            .take(CATALOG_ROWS);
        for (index, key) in matches.enumerate() {
            let _id = ui.push_id_usize(index);
            let label = if entries.contains(key) {
                format!("{key}  (in the playlist)")
            } else {
                key.to_string()
            };
            if ui.selectable(label) {
                region.add(key.clone());
            }
        }
    });

    if region != before {
        let tracks: Vec<String> = region.tracks.iter().map(ToString::to_string).collect();
        log(format_args!(
            "{LOG_PREFIX} panel edit region={region_key} include_default={} repeat={} tracks=[{}]",
            region.include_default,
            region.repeat,
            tracks.join(", ")
        ));
        *lists.get_or_insert(&region_key, snapshot.map) = region;
        save(&lists);
        drop(lists);
        push(Command::RegionChanged);
    }
    if let Some(index) = play_now {
        log(format_args!("{LOG_PREFIX} panel play index={index}"));
        push(Command::Play(index));
    }
}
