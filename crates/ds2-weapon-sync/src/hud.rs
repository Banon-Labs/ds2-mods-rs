//! The signs on screen while a feature is on, each one of `ds2-overlay`'s imgui panels: crossed
//! swords for weapon sync, a helm for armour sync, each on its own tile and shown only while its
//! own feature is on.
//!
//! # Why nothing here hooks or renders
//!
//! `ds2-overlay` owns the one `Present` detour and the one hudhook render loop the process can
//! have. Its detour is read off the game's own swap chain, found through `ds2-rva`'s
//! `GRAPHICS_DEVICE` hops, so no device or swap chain is ever created -- the throwaway swap chain
//! the invasion arrows used to build is what crashed under the DS2 Lighting Engine's `dxgi.dll`
//! proxy. This module only registers a draw function and a visibility check per feature. While a
//! feature is off its check says no, and when no other panel is up `ds2-overlay` renders no imgui
//! frame at all, so the frame the game drew goes out untouched.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use hudhook::imgui::Ui;

use crate::glyph::{self, Glyph};
use crate::install::{enabled, feature, log, logger};
use crate::policy::Kind;

/// The tile behind the sign: dark enough to read it over a bright sky.
const TILE: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
/// The tile's rim and the sign, in the gold the net-effects bar uses for its cursor.
const GOLD: [f32; 4] = [1.0, 0.85, 0.35, 1.0];
/// A dark outline under every stroke, so the gold keeps its edge against the tile's corners.
const SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.85];
/// How much wider the outline is than the stroke it sits under, in pixels.
const SHADOW_EXTRA: f32 = 2.0;

/// One feature's sign: what it is called in the log, how it is laid out, and its counters.
struct Sign {
    kind: Kind,
    name: &'static str,
    layout: fn([f32; 2]) -> Glyph,
    /// Frames on which it was drawn.
    draws: AtomicU64,
    /// Set when the feature is turned on, cleared by the next draw, so the log says the sign came
    /// back on screen and not only that the key was pressed.
    announce_next_draw: AtomicBool,
}

static SWORDS: Sign = Sign {
    kind: Kind::Weapon,
    name: "swords",
    layout: glyph::layout,
    draws: AtomicU64::new(0),
    announce_next_draw: AtomicBool::new(true),
};

static HELM: Sign = Sign {
    kind: Kind::Armor,
    name: "helm",
    layout: glyph::helm_layout,
    draws: AtomicU64::new(0),
    announce_next_draw: AtomicBool::new(true),
};

fn sign(kind: Kind) -> &'static Sign {
    match kind {
        Kind::Weapon => &SWORDS,
        Kind::Armor => &HELM,
    }
}

/// A feature's key just turned it `on` or off.
pub(crate) fn toggled(kind: Kind, on: bool) {
    let s = sign(kind);
    if on {
        s.announce_next_draw.store(true, Ordering::Release);
    } else {
        log(format_args!(
            "{} hud: {} hidden -- nothing is drawn while it is off",
            feature(kind).prefix,
            s.name
        ));
    }
}

/// The sign sits beside the game's HUD, so it is up only while the game has its own HUD up
/// ([`ds2_overlay::game_hud_visible`]): not on the title screen, while loading, or with a menu
/// open.
fn visible_swords() -> bool {
    enabled(Kind::Weapon) && ds2_overlay::game_hud_visible()
}

fn visible_helm() -> bool {
    enabled(Kind::Armor) && ds2_overlay::game_hud_visible()
}

fn draw_swords(ui: &Ui) {
    draw(&SWORDS, ui);
}

fn draw_helm(ui: &Ui) {
    draw(&HELM, ui);
}

/// `ds2-overlay` calls every panel's draw whenever any one is visible, so this checks for itself.
fn draw(s: &Sign, ui: &Ui) {
    if !enabled(s.kind) {
        return;
    }
    let prefix = feature(s.kind).prefix;
    if !ds2_overlay::game_hud_visible() {
        // Say so again when it comes back, so a run's log shows each return to the HUD.
        if !s.announce_next_draw.swap(true, Ordering::AcqRel) {
            log(format_args!(
                "{prefix} hud: {} held back -- the game's HUD is not on screen",
                s.name
            ));
        }
        return;
    }
    let display = ui.io().display_size;
    let glyph = (s.layout)(display);
    let draws = s.draws.fetch_add(1, Ordering::Relaxed) + 1;
    if s.announce_next_draw.swap(false, Ordering::AcqRel) {
        log(format_args!(
            "{prefix} hud: {} on screen (draw #{draws}) display={:.0}x{:.0} tile=({:.0},{:.0})-({:.0},{:.0})",
            s.name,
            display[0],
            display[1],
            glyph.min[0],
            glyph.min[1],
            glyph.max[0],
            glyph.max[1]
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

/// Register a feature's sign as a `ds2-overlay` panel and make sure its `Present` watcher is
/// running. The feature works without it; a `false` is logged and changes nothing else.
pub(crate) fn install(kind: Kind) -> bool {
    let prefix = feature(kind).prefix;
    let s = sign(kind);
    if let Some(sink) = logger() {
        ds2_overlay::set_logger(sink);
    }
    let (draw, visible): (fn(&Ui), fn() -> bool) = match kind {
        Kind::Weapon => (draw_swords, visible_swords),
        Kind::Armor => (draw_helm, visible_helm),
    };
    if !ds2_overlay::panels::add_panel(draw, visible, None) {
        log(format_args!(
            "{prefix} hud: every ds2-overlay panel slot is taken -- no {} on screen",
            s.name
        ));
        return false;
    }
    if !ds2_overlay::install() {
        log(format_args!(
            "{prefix} hud: ds2-overlay could not start -- no {} on screen",
            s.name
        ));
        return false;
    }
    log(format_args!(
        "{prefix} hud: the on/off sign ({}) is a ds2-overlay panel of its own (top right, shown \
         while on)",
        s.name
    ));
    true
}
