//! The selector bar on screen: one of `ds2-overlay`'s imgui panels.
//!
//! # Why nothing here hooks or renders
//!
//! `ds2-overlay` owns the one `Present` detour and the one hudhook render loop the process can
//! have, and runs every registered panel inside it. This module registers [`draw_panel`], which
//! draws whatever [`publish`] last handed it: the lines `crate::selector::view` builds on the
//! game thread's frame consumer, which runs earlier in the same `Present`.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use hudhook::imgui::Ui;

use crate::LOG_PREFIX;
use crate::install::log;
use crate::selector::{Line, LineKind};

/// The lines to draw. Written by the frame consumer, read by the panel; both run on the thread
/// calling `Present`, so the lock is never contended.
static VIEW: Mutex<Vec<Line>> = Mutex::new(Vec::new());

/// Panel calls: the render loop ran.
static DRAWS: AtomicU64 = AtomicU64::new(0);

/// Frames on which the bar had something to draw.
static VISIBLE_DRAWS: AtomicU64 = AtomicU64::new(0);

/// How many lines the last draw drew; `usize::MAX` before the first.
static LAST_LINE_COUNT: AtomicUsize = AtomicUsize::new(usize::MAX);

const PANEL: [f32; 4] = [0.0, 0.0, 0.0, 0.68];
const CURSOR_BAND: [f32; 4] = [1.0, 1.0, 1.0, 0.14];
const TITLE: [f32; 4] = [0.95, 0.90, 0.78, 1.0];
const ROW: [f32; 4] = [0.96, 0.94, 0.88, 1.0];
const CURSOR: [f32; 4] = [1.0, 0.85, 0.35, 1.0];
const HINT: [f32; 4] = [0.70, 0.70, 0.66, 1.0];
const PADDING: f32 = 8.0;

/// Hand the next frame's lines to the panel.
pub(crate) fn publish(lines: Vec<Line>) {
    if let Ok(mut view) = VIEW.lock() {
        *view = lines;
    }
}

/// The panel's draw function, called by `ds2-overlay` once per frame.
fn draw_panel(ui: &Ui) {
    let draws = DRAWS.fetch_add(1, Ordering::Relaxed) + 1;
    let display = ui.io().display_size;
    if draws == 1 {
        log(format_args!(
            "{LOG_PREFIX} overlay: first panel draw display={:.0}x{:.0}",
            display[0], display[1]
        ));
    }
    let lines = VIEW.lock().map(|v| v.clone()).unwrap_or_default();
    // One line whenever what is drawn changes shape (hidden, header only, the list), naming
    // the highlighted row: the log's proof that the draw followed the keys.
    if LAST_LINE_COUNT.swap(lines.len(), Ordering::Relaxed) != lines.len() {
        let cursor = lines
            .iter()
            .find(|l| l.kind == LineKind::Cursor)
            .map_or("", |l| l.text.as_str());
        log(format_args!(
            "{LOG_PREFIX} overlay: draw #{draws} now {} lines, cursor row {cursor:?}",
            lines.len()
        ));
    }
    if lines.is_empty() {
        return;
    }
    let visible = VISIBLE_DRAWS.fetch_add(1, Ordering::Relaxed) + 1;
    if visible == 1 || visible == 600 || visible.is_multiple_of(18_000) {
        log(format_args!(
            "{LOG_PREFIX} overlay: draw-count draws={draws} visible={visible} lines={} \
             first={:?}",
            lines.len(),
            lines[0].text
        ));
    }
    draw(ui, display, &lines);
}

fn draw(ui: &Ui, display: [f32; 2], lines: &[Line]) {
    let row = ui.current_font_size() + 2.0;
    let width = lines
        .iter()
        .map(|l| ui.calc_text_size(&l.text)[0])
        .fold(0.0f32, f32::max)
        + 2.0 * PADDING;
    let height = row * lines.len() as f32 + 2.0 * PADDING;
    let x = 24.0f32;
    let y = (display[1] * 0.18).max(24.0);
    let list = ui.get_foreground_draw_list();
    list.add_rect([x, y], [x + width, y + height], PANEL)
        .filled(true)
        .rounding(4.0)
        .build();
    for (index, line) in lines.iter().enumerate() {
        let top = y + PADDING + row * index as f32;
        let color = match line.kind {
            LineKind::Title => TITLE,
            LineKind::Row => ROW,
            LineKind::Cursor => {
                list.add_rect(
                    [x + 2.0, top - 1.0],
                    [x + width - 2.0, top + row - 1.0],
                    CURSOR_BAND,
                )
                .filled(true)
                .build();
                CURSOR
            }
            LineKind::Hint => HINT,
        };
        list.add_text(
            [x + PADDING + 1.0, top + 1.0],
            [0.0, 0.0, 0.0, 1.0],
            &line.text,
        );
        list.add_text([x + PADDING, top], color, &line.text);
    }
}

/// Register the bar as a `ds2-overlay` panel and make sure the `Present` detour goes in. `false`
/// is logged.
pub(crate) fn install() -> bool {
    if !ds2_overlay::panels::add_panel(draw_panel, None) {
        log(format_args!(
            "{LOG_PREFIX} overlay: every ds2-overlay panel slot is taken -- no selector bar"
        ));
        return false;
    }
    if !ds2_overlay::install() {
        log(format_args!(
            "{LOG_PREFIX} overlay: ds2-overlay could not start -- no selector bar and no clock"
        ));
        return false;
    }
    log(format_args!(
        "{LOG_PREFIX} overlay: the selector bar is a ds2-overlay panel"
    ));
    true
}
