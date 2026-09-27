//! The selector bar on screen: hudhook's Dear `ImGui`, drawn inside `ds2-invasion-path`'s
//! `Present` detour.
//!
//! # Why hudhook does not hook anything here
//!
//! Upstream hudhook installs its own MinHook detours on `IDXGISwapChain::Present` and
//! `ResizeBuffers`. `ds2-invasion-path` already detours `Present`, this DLL links one MinHook, and
//! MinHook answers a second hook on the same address with `MH_ERROR_ALREADY_CREATED` -- which
//! upstream's `ImguiDx11Hooks::new` turns into a panic inside the game. So the vendored copy
//! (`vendor/hudhook`) gained `set_shared_render_loop` and `render_shared`, and this module puts
//! [`on_present`] in `ds2_invasion_path::frame_hook`'s overlay slot: one `Present` detour, which
//! draws the route lines and then this bar. `ResizeBuffers` needs no hook: hudhook's render target
//! view lives for one call, and the display size is re-read from the swap chain every frame.
//!
//! # What it draws
//!
//! Whatever [`publish`] last handed it: the lines `crate::selector::view` builds on the game
//! thread's frame consumer, which runs earlier in the same `Present`.

use core::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use hudhook::imgui::{Context, FontConfig, FontSource, Ui};
use hudhook::windows::Win32::Graphics::Dxgi::IDXGISwapChain;
use hudhook::windows::core::Interface;
use hudhook::{ImguiRenderLoop, RenderContext};

use crate::LOG_PREFIX;
use crate::install::log;
use crate::selector::{Line, LineKind};

/// The lines to draw. Written by the frame consumer, read by the render loop; both run on the
/// thread calling `Present`, so the lock is never contended.
static VIEW: Mutex<Vec<Line>> = Mutex::new(Vec::new());

/// `render` calls: the render loop ran.
static DRAWS: AtomicU64 = AtomicU64::new(0);

/// Frames on which the bar had something to draw.
static VISIBLE_DRAWS: AtomicU64 = AtomicU64::new(0);

/// How many lines the last `render` drew; `usize::MAX` before the first.
static LAST_LINE_COUNT: AtomicUsize = AtomicUsize::new(usize::MAX);

/// `render_shared` failures.
static ERRORS: AtomicU64 = AtomicU64::new(0);

/// imgui's default font rasterises at 13 px; read across a room, it wants 25% more, as ER's bar
/// does.
const FONT_SIZE_PX: f32 = 13.0 * 1.25;

const PANEL: [f32; 4] = [0.0, 0.0, 0.0, 0.68];
const CURSOR_BAND: [f32; 4] = [1.0, 1.0, 1.0, 0.14];
const TITLE: [f32; 4] = [0.95, 0.90, 0.78, 1.0];
const ROW: [f32; 4] = [0.96, 0.94, 0.88, 1.0];
const CURSOR: [f32; 4] = [1.0, 0.85, 0.35, 1.0];
const HINT: [f32; 4] = [0.70, 0.70, 0.66, 1.0];
const PADDING: f32 = 8.0;

/// Hand the next frame's lines to the render loop.
pub(crate) fn publish(lines: Vec<Line>) {
    if let Ok(mut view) = VIEW.lock() {
        *view = lines;
    }
}

struct SelectorOverlay;

impl ImguiRenderLoop for SelectorOverlay {
    fn initialize<'a>(&'a mut self, ctx: &mut Context, _render_context: &'a mut dyn RenderContext) {
        ctx.fonts().add_font(&[FontSource::DefaultFontData {
            config: Some(FontConfig {
                size_pixels: FONT_SIZE_PX,
                ..FontConfig::default()
            }),
        }]);
        // Keep imgui's `imgui.ini` out of the game directory.
        ctx.set_ini_filename(None);
        log(format_args!(
            "{LOG_PREFIX} overlay: hudhook render loop initialized (font {FONT_SIZE_PX}px)"
        ));
    }

    fn render(&mut self, ui: &mut Ui) {
        let draws = DRAWS.fetch_add(1, Ordering::Relaxed) + 1;
        let display = ui.io().display_size;
        if draws == 1 {
            log(format_args!(
                "{LOG_PREFIX} overlay: hudhook first render display={:.0}x{:.0}",
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

/// The overlay slot's function: draw one imgui frame onto the swap chain being presented.
fn on_present(swap_chain: *mut c_void) {
    // SAFETY: the raw `IDXGISwapChain*` the `Present` detour was called with, live for this call.
    // Borrowed, not owned: no reference is taken or released.
    let Some(chain) = (unsafe { IDXGISwapChain::from_raw_borrowed(&swap_chain) }) else {
        return;
    };
    // SAFETY: this is the thread presenting `chain`, inside its `Present` detour, which is the
    // contract of `render_shared`.
    if let Err(error) = unsafe { hudhook::hooks::dx11::render_shared(chain) } {
        let errors = ERRORS.fetch_add(1, Ordering::Relaxed) + 1;
        if errors <= 3 || errors.is_power_of_two() {
            log(format_args!(
                "{LOG_PREFIX} overlay: render failed ({errors} so far): {error:?} -- the frame is \
                 presented without the bar"
            ));
        }
    }
}

/// Give hudhook the render loop and take `ds2-invasion-path`'s overlay slot. `false` is logged.
pub(crate) fn install() -> bool {
    // SAFETY: the only use of hudhook's dx11 statics in this DLL; `ImguiDx11Hooks` is never built.
    if !unsafe { hudhook::hooks::dx11::set_shared_render_loop(SelectorOverlay) } {
        log(format_args!(
            "{LOG_PREFIX} overlay: hudhook already has a render loop -- no selector bar"
        ));
        return false;
    }
    if !ds2_invasion_path::frame_hook::set_present_overlay(on_present) {
        log(format_args!(
            "{LOG_PREFIX} overlay: the Present overlay slot is taken -- no selector bar"
        ));
        return false;
    }
    log(format_args!(
        "{LOG_PREFIX} overlay: hudhook dx11 renders inside ds2-invasion-path's Present detour \
         (shared, no second Present hook)"
    ));
    true
}
