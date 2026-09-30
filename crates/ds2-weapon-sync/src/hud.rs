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
use std::sync::Mutex;
use std::time::Instant;

use ds2_overlay::status_strip;
use ds2_overlay::style;
use hudhook::imgui::Ui;

use crate::glyph::{self, Glyph};
use crate::install::{enabled, feature, log, logger};
use crate::policy::Kind;

/// The tile behind the sign, the rim and strokes, and the outline under them: the status strip's
/// colours, so every feature sign in the corner reads as one set.
const TILE: [f32; 4] = status_strip::TILE;
const INK: [f32; 4] = status_strip::INK;
const SHADOW: [f32; 4] = status_strip::SHADOW;
/// How much wider the outline is than the stroke it sits under, in pixels.
const SHADOW_EXTRA: f32 = 2.0;

/// A panel's draw function, as `ds2-overlay` takes it.
type DrawFn = fn(&Ui);
/// A panel's visibility check.
type VisibleFn = fn() -> bool;

/// One feature's sign: what it is called in the log, how it is laid out, and its counters.
struct Sign {
    kind: Kind,
    name: &'static str,
    /// What the player reads under the tile for `status_strip::NAME_SECONDS` after switching it on.
    title: &'static str,
    layout: fn([f32; 2]) -> Glyph,
    /// Frames on which it was drawn.
    draws: AtomicU64,
    /// Set when the feature is turned on, cleared by the next draw, so the log says the sign came
    /// back on screen and not only that the key was pressed.
    announce_next_draw: AtomicBool,
    /// Set when the feature is turned on; the next draw starts the name's clock from it. Kept
    /// apart from `announce_next_draw`, which also re-arms on every return from a menu.
    name_pending: AtomicBool,
    /// When the name started showing.
    name_since: Mutex<Option<Instant>>,
}

static SWORDS: Sign = Sign {
    kind: Kind::Weapon,
    name: "swords",
    title: "Weapon Sync",
    layout: glyph::layout,
    draws: AtomicU64::new(0),
    announce_next_draw: AtomicBool::new(true),
    name_pending: AtomicBool::new(true),
    name_since: Mutex::new(None),
};

static HELM: Sign = Sign {
    kind: Kind::Armor,
    name: "helm",
    title: "Armor Sync",
    layout: glyph::helm_layout,
    draws: AtomicU64::new(0),
    announce_next_draw: AtomicBool::new(true),
    name_pending: AtomicBool::new(true),
    name_since: Mutex::new(None),
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
        s.name_pending.store(true, Ordering::Release);
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
            s.name, display[0], display[1], glyph.min[0], glyph.min[1], glyph.max[0], glyph.max[1]
        ));
    }
    paint(ui, &glyph);
    paint_name(s, ui, display);
}

/// The feature's name under its tile, for `status_strip::NAME_SECONDS` after it was switched on.
fn paint_name(s: &Sign, ui: &Ui, display: [f32; 2]) {
    let Ok(mut since) = s.name_since.lock() else {
        return;
    };
    if s.name_pending.swap(false, Ordering::AcqRel) {
        *since = Some(Instant::now());
    }
    let Some(started) = *since else {
        return;
    };
    let seconds = started.elapsed().as_secs_f32();
    if !status_strip::shows_name(seconds) {
        *since = None;
        return;
    }
    let slot = match s.kind {
        Kind::Weapon => status_strip::Slot::WeaponSync,
        Kind::Armor => status_strip::Slot::ArmorSync,
    };
    let tile = status_strip::tile(display, slot);
    let at = status_strip::name_origin(&tile, ui.calc_text_size(s.title)[0]);
    let list = ui.get_foreground_draw_list();
    list.add_text([at[0] + 1.0, at[1] + 1.0], SHADOW, s.title);
    list.add_text(at, style::TEXT, s.title);
}

fn paint(ui: &Ui, glyph: &Glyph) {
    let list = ui.get_foreground_draw_list();
    list.add_rect(glyph.min, glyph.max, TILE)
        .filled(true)
        .rounding(glyph.rounding)
        .build();
    list.add_rect(glyph.min, glyph.max, INK)
        .rounding(glyph.rounding)
        .thickness(style::FRAME_PX)
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
        list.add_line(stroke.from, stroke.to, INK)
            .thickness(stroke.width)
            .build();
    }
    for dot in &glyph.dots {
        list.add_circle(dot.centre, dot.radius, INK)
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
    let (draw, visible): (DrawFn, VisibleFn) = match kind {
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
