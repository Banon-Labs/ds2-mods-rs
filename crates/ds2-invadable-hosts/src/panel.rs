//! The Invadable hosts panel: one row per Seamless invasion bracket, each the number of hosts this
//! player could invade there now, drawn in the game's own palette (`ds2_overlay::style`,
//! docs/DS2-UI-DESIGN.md) through `ds2-overlay`, the one `Present` hook in this process.
//!
//! # What it shows
//!
//! The title and a Close; the view's heading; one line that says what is counted and how fresh it
//! is; the grid, each cell shaded against the fullest, this character's own bracket outlined; and
//! the line that offers the other view. Hosts now is the view the panel opens on. The view key (`R`
//! by default, `view_key`) switches the grid to new hosts per minute and back, and so does a click
//! on that line. Hovering a cell names its bracket and its number in words.
//!
//! # Input
//!
//! While it is open the panel takes the game's input through `ds2-input-harness`: `hold` blanks the
//! pad, the DirectInput mouse and keyboard and the Win32 mouse after each poll (so the camera holds
//! still and no key reaches the game) and the harness's cursor hook frees the pointer. The overlay
//! feeds imgui the corrected cursor. The panel key or Escape closes it, and the hold is released
//! only once every key the panel reads is up, so the key that closed it never reaches the game.
//! Keys count only while the game's window has the keyboard.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use ds2_hotkey_config::keys::vk_name;
use ds2_hotkey_config::reload::{FileChange, HotFile};
use ds2_overlay::style;
use hudhook::imgui::{MouseButton, Ui};
use hudhook::windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use hudhook::windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId,
};

use crate::census::{BRACKET_COUNT, Cell, bracket_label, watched_text};
use crate::driver::{self, PanelView};
use crate::install::log;
use crate::keys::{self, Edge, Keys};
use crate::{HEARTBEAT_EVERY_MS, LOG_PREFIX};

const VK_ESCAPE: u32 = 0x1b;
const VK_LBUTTON: u32 = 0x01;

/// The panel's width at 1080 lines; it scales with the back buffer.
const PANEL_WIDTH: f32 = 560.0;
/// The display height the layout is written for.
const REFERENCE_HEIGHT: f32 = 1080.0;
const PAD: f32 = 16.0;
/// A grid row's height at 1080 lines, and the gap between rows.
const ROW_HEIGHT: f32 = 30.0;
const ROW_GAP: f32 = 4.0;
/// The level label column's width at 1080 lines.
const LABEL_COLUMN: f32 = 150.0;
/// How much of the bronze a full cell is shaded with.
const CELL_SHADE_ALPHA: f32 = 0.55;

const TITLE: &str = "Invadable hosts";

static OPEN: AtomicBool = AtomicBool::new(false);
/// Closed, but still holding input until every key the panel reads is up.
static CLOSING: AtomicBool = AtomicBool::new(false);
/// The grid shows new hosts per minute rather than hosts now. Reset to hosts now on every open.
static RATE_VIEW: AtomicBool = AtomicBool::new(false);
/// Set by a panic in the panel: it stays hidden for the rest of the session.
static HIDDEN: AtomicBool = AtomicBool::new(false);
static OPENS: AtomicU64 = AtomicU64::new(0);
static DRAWS: AtomicU64 = AtomicU64::new(0);
static VIEW_SWITCHES: AtomicU64 = AtomicU64::new(0);
static CLICKS: AtomicU64 = AtomicU64::new(0);

struct KeyState {
    keys: Keys,
    menu: Edge,
    view: Edge,
    escape: Edge,
    file: Option<HotFile>,
    next_heartbeat_ms: u64,
}

static KEYS: Mutex<Option<KeyState>> = Mutex::new(None);
/// The panel's view, rebuilt by the frame hook while the panel is open.
static VIEW: Mutex<Option<PanelView>> = Mutex::new(None);
/// The keys in force, for the drawing: (menu, view).
static KEY_NAMES: Mutex<(String, String)> = Mutex::new((String::new(), String::new()));

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Register the panel and its frame hook with `ds2-overlay`, which must already be started.
/// `config` is `ds2-mods.toml`, whose text carries the keys and is re-read while the game runs.
pub(crate) fn install(config: PathBuf, text: &str) -> bool {
    let mut keys = Keys::default();
    let update = keys.apply(text);
    if let Some(line) = keys::describe_update(&update) {
        log(format_args!("{LOG_PREFIX} config keys: {line}"));
    }
    let mut file = HotFile::new(config);
    file.adopt(text);
    *lock(&KEY_NAMES) = (vk_name(keys.menu.code()), vk_name(keys.view.code()));
    log(format_args!(
        "{LOG_PREFIX} config [{}] enabled=true {}",
        keys::CONFIG_SECTION,
        keys.describe()
    ));
    *lock(&KEYS) = Some(KeyState {
        keys,
        menu: Edge::default(),
        view: Edge::default(),
        escape: Edge::default(),
        file: Some(file),
        next_heartbeat_ms: 0,
    });
    if !ds2_overlay::panels::add_panel(draw, is_visible, Some(wants_input)) {
        log(format_args!(
            "{LOG_PREFIX} every ds2-overlay panel slot is taken -- no Invadable hosts panel this run"
        ));
        return false;
    }
    if !ds2_overlay::frame_hook::add_frame_hook(on_frame) {
        log(format_args!(
            "{LOG_PREFIX} every ds2-overlay frame-hook slot is taken -- the panel key does nothing"
        ));
        return false;
    }
    true
}

fn is_visible() -> bool {
    OPEN.load(Ordering::Acquire) && !HIDDEN.load(Ordering::Acquire)
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

fn open() {
    ds2_input_harness::hold(true);
    ds2_overlay::panels::use_overlay_mouse_for_imgui(true);
    RATE_VIEW.store(false, Ordering::Release);
    CLOSING.store(false, Ordering::Release);
    OPEN.store(true, Ordering::Release);
    OPENS.fetch_add(1, Ordering::Relaxed);
    driver::arm();
    log(format_args!("{LOG_PREFIX} panel opened"));
}

fn close() {
    if !OPEN.swap(false, Ordering::AcqRel) {
        return;
    }
    CLOSING.store(true, Ordering::Release);
    log(format_args!("{LOG_PREFIX} panel closed"));
}

fn switch_view() {
    let rate = !RATE_VIEW.load(Ordering::Acquire);
    RATE_VIEW.store(rate, Ordering::Release);
    VIEW_SWITCHES.fetch_add(1, Ordering::Relaxed);
    log(format_args!(
        "{LOG_PREFIX} view -> {}",
        if rate {
            "new hosts per minute"
        } else {
            "hosts now"
        }
    ));
}

/// After a panic in the panel's drawing: hide it for the rest of the session and give the game its
/// input back. The census keeps counting.
fn hide_for_session() {
    if !HIDDEN.swap(true, Ordering::AcqRel) {
        log(format_args!(
            "{LOG_PREFIX} a panic while drawing the panel; it stays hidden until the game restarts"
        ));
    }
    close();
}

/// Once per `Present`: the keys, the config re-read, the census, the view and the heartbeat.
fn on_frame() {
    driver::tick();
    let now = driver::now_ms();
    let mut guard = lock(&KEYS);
    let Some(state) = guard.as_mut() else {
        return;
    };
    if let Some(FileChange::Text(text)) = state.file.as_mut().and_then(HotFile::poll) {
        let update = state.keys.apply(&text);
        if let Some(line) = keys::describe_update(&update) {
            log(format_args!("{LOG_PREFIX} config reloaded: {line}"));
        }
        if update.changed() {
            state.menu.reset();
            state.view.reset();
            *lock(&KEY_NAMES) = (
                vk_name(state.keys.menu.code()),
                vk_name(state.keys.view.code()),
            );
            log(format_args!(
                "{LOG_PREFIX} config [{}] enabled=true {}",
                keys::CONFIG_SECTION,
                state.keys.describe()
            ));
        }
    }
    let focus = game_has_focus();
    let menu_key = state.keys.menu.code();
    let view_key = state.keys.view.code();
    let menu = state.menu.pressed(menu_key, focus && key_down(menu_key));
    let view = state.view.pressed(view_key, focus && key_down(view_key));
    let escape = state
        .escape
        .pressed(VK_ESCAPE, focus && key_down(VK_ESCAPE));
    // The mouse button too: a click on Close or on the prompt must not land in the game either.
    let any_held = state.menu.held()
        || state.view.held()
        || state.escape.held()
        || (focus && key_down(VK_LBUTTON));
    if OPEN.load(Ordering::Acquire) {
        if menu || escape {
            close();
        } else if view {
            switch_view();
        }
    } else if CLOSING.load(Ordering::Acquire) {
        if !any_held {
            CLOSING.store(false, Ordering::Release);
            ds2_overlay::panels::use_overlay_mouse_for_imgui(false);
            ds2_input_harness::hold(false);
        }
    } else if menu && !HIDDEN.load(Ordering::Acquire) {
        open();
    }
    if OPEN.load(Ordering::Acquire) {
        *lock(&VIEW) = Some(driver::view());
    }
    if OPENS.load(Ordering::Relaxed) > 0 && now >= state.next_heartbeat_ms {
        state.next_heartbeat_ms = now + HEARTBEAT_EVERY_MS;
        heartbeat();
    }
}

fn heartbeat() {
    let (sweeps, failed, sent, held) = driver::tallies();
    let (filters, requests, publishes) = crate::steam::seen_counts();
    let [pad, mouse, keyboard, win_mouse] = ds2_input_harness::poll_counts();
    log(format_args!(
        "{LOG_PREFIX} heartbeat panel[open={} opens={} drawn={} switches={} clicks={} hidden={}] \
         census[sweeps={sweeps} failed={failed} sent={sent} held_for_seamless={held}] \
         seamless[filters={filters} requests={requests} publishes={publishes}] \
         input[held={} blocking={} polls pad={pad} mouse={mouse} keyboard={keyboard} \
         win_mouse={win_mouse}]",
        OPEN.load(Ordering::Relaxed),
        OPENS.load(Ordering::Relaxed),
        DRAWS.load(Ordering::Relaxed),
        VIEW_SWITCHES.load(Ordering::Relaxed),
        CLICKS.load(Ordering::Relaxed),
        HIDDEN.load(Ordering::Relaxed),
        ds2_input_harness::is_held(),
        ds2_input_harness::is_blocking(),
    ));
}

/// The panel's draw function, called by `ds2-overlay` once per frame.
fn draw(ui: &Ui) {
    if !is_visible() {
        return;
    }
    if catch_unwind(AssertUnwindSafe(|| draw_panel(ui))).is_err() {
        hide_for_session();
    } else {
        DRAWS.fetch_add(1, Ordering::Relaxed);
    }
}

fn status_line(view: &PanelView, rate: bool) -> String {
    let progress = match view.progress {
        Some(bracket) => format!(" Counting bracket {} of {BRACKET_COUNT}.", bracket + 1),
        None => String::new(),
    };
    if rate {
        return format!(
            "New open hosts per minute, averaged over {} watched.{progress}",
            watched_text(view.watched_minutes)
        );
    }
    format!(
        "{} open hosts on your regulation, from the last full count.{progress}",
        view.total
    )
}

fn draw_panel(ui: &Ui) {
    let view = lock(&VIEW).clone().unwrap_or_default();
    let (menu_key, view_key) = lock(&KEY_NAMES).clone();
    let rate = RATE_VIEW.load(Ordering::Acquire);
    let display = ui.io().display_size;
    let scale = (display[1] / REFERENCE_HEIGHT).max(0.5);
    let line = ui.current_font_size();
    let width = (PANEL_WIDTH * scale)
        .max(line * 24.0)
        .min(display[0] - 32.0);
    let row_height = (ROW_HEIGHT * scale).max(line + 6.0).round();
    let row_gap = (ROW_GAP * scale).round();
    let pad = (PAD * scale).round();
    let title_line = ds2_overlay::panels::title_height(ui);
    let inner_width = width - pad * 2.0;

    // Words first, so the height is known before anything is drawn.
    let status = status_line(&view, rate);
    let mut notes: Vec<(String, [f32; 4])> = Vec::new();
    if let Some(error) = &view.error {
        notes.push((format!("Not counting: {error}"), style::WARN_TEXT));
    } else {
        notes.push((status, style::TEXT_DIM));
        if !view.pool_known {
            notes.push((
                format!(
                    "Seamless has not shown its regulation yet this session, so counting uses \
                     the one 0.0.5 hosts publish ({}). Hosting or searching once confirms it.",
                    ds2_seamless::lobby::REGULATION_VALUE_MEASURED
                ),
                style::WARN_TEXT,
            ));
        }
        if view.waiting_for_seamless {
            notes.push((
                "A Seamless search is running, so counting waits for it: a count sent into a \
                 search would cost Seamless its own answer."
                    .to_owned(),
                style::TEXT_DIM,
            ));
        }
    }
    let wrapped: Vec<(Vec<String>, [f32; 4])> = notes
        .iter()
        .map(|(text, colour)| (wrap(ui, text, inner_width), *colour))
        .collect();
    let note_lines: usize = wrapped.iter().map(|(lines, _)| lines.len()).sum();
    let cells = if rate {
        view.rate.as_deref()
    } else {
        view.hosts.as_deref()
    };
    let grid_height = match cells {
        Some(_) => BRACKET_COUNT as f32 * (row_height + row_gap),
        None => line,
    };
    let height = pad * 2.0
        + title_line
        + 6.0
        + line
        + 8.0
        + note_lines as f32 * line
        + 10.0
        + grid_height
        + 12.0
        + (line + 4.0) * 2.0;
    let left = ((display[0] - width) * 0.5).round();
    let top = (display[1] * 0.12)
        .round()
        .min((display[1] - height).max(0.0));

    let list = ui.get_foreground_draw_list();
    list.add_rect([left, top], [left + width, top + height], style::PANEL_BG)
        .filled(true)
        .rounding(style::ROUNDING)
        .build();
    ds2_overlay::panels::frame(&list, [left, top], [left + width, top + height], display[1]);

    let mouse = ds2_overlay::panels::mouse().unwrap_or(ui.io().mouse_pos);
    let inside = |min: [f32; 2], max: [f32; 2]| {
        mouse[0] >= min[0] && mouse[0] < max[0] && mouse[1] >= min[1] && mouse[1] < max[1]
    };
    let clicked = ui.is_mouse_clicked(MouseButton::Left);
    let inner_left = left + pad;
    let inner_right = left + width - pad;
    let mut y = top + pad;

    ds2_overlay::panels::title(ui, &list, [inner_left, y], style::TEXT, TITLE);
    let close_label = "Close";
    let close_min = [inner_right - ui.calc_text_size(close_label)[0], y];
    let close_max = [inner_right, y + line];
    let close_hover = inside(close_min, close_max);
    list.add_text(
        close_min,
        if close_hover {
            style::TEXT
        } else {
            style::TEXT_DIM
        },
        close_label,
    );
    if close_hover && clicked {
        CLICKS.fetch_add(1, Ordering::Relaxed);
        close();
    }
    y += title_line + 6.0;
    list.add_text(
        [inner_left, y],
        style::BRONZE,
        if rate {
            "New hosts per minute"
        } else {
            "Hosts now"
        },
    );
    y += line + 8.0;
    for (lines, colour) in &wrapped {
        for text in lines {
            list.add_text([inner_left, y], *colour, text);
            y += line;
        }
    }
    y += 10.0;

    let mut tooltip: Option<Vec<String>> = None;
    match cells {
        Some(cells) => {
            for (bracket, cell) in cells.iter().enumerate().take(BRACKET_COUNT) {
                let row_top = y + bracket as f32 * (row_height + row_gap);
                let rect = RowRect {
                    min: [inner_left, row_top],
                    max: [inner_right, row_top + row_height],
                    label_width: (LABEL_COLUMN * scale).round(),
                };
                let row = RowCell {
                    bracket,
                    cell,
                    own: view.own_bracket == Some(bracket),
                    own_from_seamless: view.own_from_seamless,
                    rate,
                };
                if let Some(words) = draw_row(ui, &list, &rect, &row, inside) {
                    tooltip = Some(words);
                }
            }
        }
        None => {
            let text = if view.error.is_some() {
                ""
            } else if rate {
                "No rate yet: it needs two full counts, about a minute apart."
            } else {
                "The first count is still running."
            };
            list.add_text([inner_left, y], style::TEXT_DIM, text);
        }
    }
    y += grid_height + 12.0;

    // "Press [R] to see new hosts per minute": the line is a button as well as a hint, so a player
    // with the mouse in hand need not reach for the key.
    let press = "Press ";
    let press_width = ui.calc_text_size(press)[0];
    list.add_text([inner_left, y], style::TEXT_DIM, press);
    let verb = if rate {
        "to see hosts now"
    } else {
        "to see new hosts per minute"
    };
    ds2_overlay::panels::hint_bar(
        ui,
        &list,
        [inner_left + press_width, y],
        inner_right,
        &[(view_key.as_str(), verb)],
    );
    let prompt_width =
        press_width + ui.calc_text_size(&view_key)[0] + ui.calc_text_size(verb)[0] + line;
    let prompt_min = [inner_left, y - 2.0];
    let prompt_max = [inner_left + prompt_width, y + line + 2.0];
    if inside(prompt_min, prompt_max) {
        list.add_line(
            [prompt_min[0], prompt_max[1]],
            [prompt_max[0], prompt_max[1]],
            style::BRONZE_DIM,
        )
        .build();
        tooltip.get_or_insert_with(|| vec![format!("Click, or press {view_key}.")]);
        if clicked {
            CLICKS.fetch_add(1, Ordering::Relaxed);
            switch_view();
        }
    }
    y += line + 4.0;
    ds2_overlay::panels::hint_bar(
        ui,
        &list,
        [inner_left, y],
        inner_right,
        &[(menu_key.as_str(), "or"), ("Esc", "Close")],
    );

    if let Some(lines) = tooltip {
        draw_tooltip(ui, &list, mouse, display, &lines);
    }
}

/// Where one grid row goes: its corners and the width of its level label.
struct RowRect {
    min: [f32; 2],
    max: [f32; 2],
    label_width: f32,
}

/// What one grid row says.
struct RowCell<'a> {
    bracket: usize,
    cell: &'a Cell,
    own: bool,
    /// The own bracket came from Seamless's own search or publish, not the level table.
    own_from_seamless: bool,
    rate: bool,
}

/// One grid row: the level label, then the cell. Answers the tooltip's lines when hovered.
fn draw_row(
    ui: &Ui,
    list: &hudhook::imgui::DrawListMut<'_>,
    rect: &RowRect,
    row: &RowCell<'_>,
    inside: impl Fn([f32; 2], [f32; 2]) -> bool,
) -> Option<Vec<String>> {
    let RowRect {
        min,
        max,
        label_width,
    } = *rect;
    let RowCell {
        bracket,
        cell,
        own,
        own_from_seamless,
        rate,
    } = *row;
    let line = ui.current_font_size();
    let label = format!("SL {}", bracket_label(bracket));
    let text_y = min[1] + ((max[1] - min[1] - line) * 0.5).round();
    list.add_text(
        [min[0], text_y],
        if own { style::TEXT } else { style::TEXT_DIM },
        &label,
    );
    let cell_min = [min[0] + label_width, min[1]];
    let cell_max = max;
    list.add_rect(cell_min, cell_max, style::INK_1)
        .filled(true)
        .rounding(style::ROUNDING)
        .build();
    if cell.weight > 0.0 {
        // The shade grows from the left like a bar, so a glance compares rows by length.
        let fill = cell_min[0] + (cell_max[0] - cell_min[0]) * cell.weight.clamp(0.0, 1.0);
        list.add_rect(
            cell_min,
            [fill, cell_max[1]],
            style::with_alpha(style::BRONZE_DIM, CELL_SHADE_ALPHA),
        )
        .filled(true)
        .rounding(style::ROUNDING)
        .build();
    }
    list.add_rect(
        cell_min,
        cell_max,
        if own { style::BRONZE } else { style::INK_2 },
    )
    .rounding(style::ROUNDING)
    .thickness(if own { 2.0 } else { style::FRAME_PX })
    .build();
    let words = ui.calc_text_size(&cell.text);
    list.add_text(
        [
            cell_max[0] - words[0] - 8.0,
            min[1] + ((max[1] - min[1] - words[1]) * 0.5).round(),
        ],
        if cell.weight > 0.0 {
            style::TEXT
        } else {
            style::TEXT_DISABLED
        },
        &cell.text,
    );
    if !inside(min, max) {
        return None;
    }
    let number = if rate {
        format!("{} new open hosts a minute.", cell.text)
    } else if cell.text.starts_with('\u{2265}') {
        format!(
            "At least {} open hosts now: Steam returns 50 lobbies a query at most.",
            cell.text.trim_start_matches('\u{2265}')
        )
    } else {
        format!("{} open hosts now.", cell.text)
    };
    let mut lines = vec![
        format!(
            "Soul level {} (Seamless bracket {bracket}).",
            bracket_label(bracket)
        ),
        number,
    ];
    if own {
        lines.push(if own_from_seamless {
            "Your bracket, as your own Seamless search asked for it.".to_owned()
        } else {
            "Your bracket, by your soul level: the only one your invasion search asks for."
                .to_owned()
        });
    }
    Some(lines)
}

fn draw_tooltip(
    ui: &Ui,
    list: &hudhook::imgui::DrawListMut<'_>,
    mouse: [f32; 2],
    display: [f32; 2],
    lines: &[String],
) {
    let line = ui.current_font_size();
    let pad = (line * 0.4).round();
    let width = lines
        .iter()
        .map(|text| ui.calc_text_size(text)[0])
        .fold(0.0, f32::max)
        + pad * 2.0;
    let height = lines.len() as f32 * line + pad * 2.0;
    let left = (mouse[0] + line).min(display[0] - width).max(0.0);
    let top = (mouse[1] + line).min(display[1] - height).max(0.0);
    list.add_rect([left, top], [left + width, top + height], style::INK_0)
        .filled(true)
        .build();
    list.add_rect([left, top], [left + width, top + height], style::BRONZE_DIM)
        .thickness(style::FRAME_PX)
        .build();
    for (index, text) in lines.iter().enumerate() {
        list.add_text(
            [left + pad, top + pad + index as f32 * line],
            style::TEXT,
            text,
        );
    }
}

/// Break `text` into lines no wider than `width`, at spaces.
fn wrap(ui: &Ui, text: &str, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split(' ') {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };
        if !current.is_empty() && ui.calc_text_size(&candidate)[0] > width {
            lines.push(std::mem::take(&mut current));
            current = word.to_owned();
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}
