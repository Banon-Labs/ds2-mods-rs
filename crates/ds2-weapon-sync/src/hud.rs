//! The crossed swords on screen while weapon sync is on: one of `ds2-overlay`'s imgui panels.
//!
//! # Why nothing here hooks or renders
//!
//! `ds2-overlay` owns the one `Present` detour and the one hudhook render loop the process can
//! have. Its detour is read off the game's own swap chain, found through `ds2-rva`'s
//! `GRAPHICS_DEVICE` hops, so no device or swap chain is ever created -- the throwaway swap chain
//! the invasion arrows used to build is what crashed under the DS2 Lighting Engine's `dxgi.dll`
//! proxy. This module only registers a draw function and a visibility check. While the feature is
//! off the check says no, and when no other panel is up `ds2-overlay` renders no imgui frame at
//! all, so the frame the game drew goes out untouched.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use hudhook::imgui::Ui;

use crate::LOG_PREFIX;
use crate::glyph::{self, Glyph};
use crate::install::{enabled, log, logger};

/// The tile behind the swords: dark enough to read them over a bright sky.
const TILE: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
/// The tile's rim and the swords, in the gold the net-effects bar uses for its cursor.
const GOLD: [f32; 4] = [1.0, 0.85, 0.35, 1.0];
/// A dark outline under every stroke, so the gold keeps its edge against the tile's corners.
const SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.85];
/// How much wider the outline is than the stroke it sits under, in pixels.
const SHADOW_EXTRA: f32 = 2.0;

/// Frames on which the swords were drawn.
static DRAWS: AtomicU64 = AtomicU64::new(0);

/// Set when the feature is turned on, cleared by the next draw, so the log says the swords came
/// back on screen and not only that the key was pressed.
static ANNOUNCE_NEXT_DRAW: AtomicBool = AtomicBool::new(true);

/// The key just turned the feature `on` or off.
pub(crate) fn toggled(on: bool) {
    if on {
        ANNOUNCE_NEXT_DRAW.store(true, Ordering::Release);
    } else {
        log(format_args!(
            "{LOG_PREFIX} hud: swords hidden -- nothing is drawn while weapon sync is off"
        ));
    }
}

/// `ds2-overlay` asks this before rendering a frame. The swords sit beside the game's HUD, so they
/// are up only while the game has its own HUD up ([`ds2_overlay::game_hud_visible`]): not on the
/// title screen, while loading, or with a menu open.
fn visible() -> bool {
    enabled() && ds2_overlay::game_hud_visible()
}

/// The panel's draw function. `ds2-overlay` calls every panel's whenever any one is visible, so
/// this checks for itself.
fn draw(ui: &Ui) {
    if !enabled() {
        return;
    }
    if !ds2_overlay::game_hud_visible() {
        // Say so again when they come back, so a run's log shows each return to the HUD.
        if !ANNOUNCE_NEXT_DRAW.swap(true, Ordering::AcqRel) {
            log(format_args!(
                "{LOG_PREFIX} hud: swords held back -- the game's HUD is not on screen"
            ));
        }
        return;
    }
    let display = ui.io().display_size;
    let glyph = glyph::layout(display);
    let draws = DRAWS.fetch_add(1, Ordering::Relaxed) + 1;
    if ANNOUNCE_NEXT_DRAW.swap(false, Ordering::AcqRel) {
        log(format_args!(
            "{LOG_PREFIX} hud: swords on screen (draw #{draws}) display={:.0}x{:.0} tile=({:.0},{:.0})-({:.0},{:.0})",
            display[0], display[1], glyph.min[0], glyph.min[1], glyph.max[0], glyph.max[1]
        ));
    }
    paint(ui, &glyph);
}

fn paint(ui: &Ui, glyph: &Glyph) {
    let list = ui.get_foreground_draw_list();
    list.add_rect(glyph.min, glyph.max, TILE)
        .filled(true)
        .rounding(glyph.rounding)
        .build();
    list.add_rect(glyph.min, glyph.max, GOLD)
        .rounding(glyph.rounding)
        .thickness(1.5)
        .build();
    for stroke in &glyph.strokes {
        list.add_line(stroke.from, stroke.to, SHADOW)
            .thickness(stroke.width + SHADOW_EXTRA)
            .build();
    }
    for dot in &glyph.dots {
        list.add_circle(dot.centre, dot.radius + SHADOW_EXTRA / 2.0, SHADOW)
            .filled(true)
            .build();
    }
    for stroke in &glyph.strokes {
        list.add_line(stroke.from, stroke.to, GOLD)
            .thickness(stroke.width)
            .build();
    }
    for dot in &glyph.dots {
        list.add_circle(dot.centre, dot.radius, GOLD)
            .filled(true)
            .build();
    }
}

/// Register the swords as a `ds2-overlay` panel and make sure its `Present` watcher is running.
/// The feature works without them; a `false` is logged and changes nothing else.
pub(crate) fn install() -> bool {
    if let Some(sink) = logger() {
        ds2_overlay::set_logger(sink);
    }
    if !ds2_overlay::panels::add_panel(draw, visible, None) {
        log(format_args!(
            "{LOG_PREFIX} hud: every ds2-overlay panel slot is taken -- no swords on screen"
        ));
        return false;
    }
    if !ds2_overlay::install() {
        log(format_args!(
            "{LOG_PREFIX} hud: ds2-overlay could not start -- no swords on screen"
        ));
        return false;
    }
    log(format_args!(
        "{LOG_PREFIX} hud: the on/off swords are a ds2-overlay panel (top right, shown while on)"
    ));
    true
}
