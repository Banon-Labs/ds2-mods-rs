//! Every imgui panel, drawn inside one hudhook frame.
//!
//! hudhook holds one render loop per process and subclasses the game window once for its input.
//! So there is one loop, [`Panels`], and it calls each registered panel's draw function in the
//! order they were added. A panel that draws nothing this frame simply returns.
//!
//! # Input
//!
//! hudhook's window-procedure subclass feeds mouse and keyboard messages to imgui, which is what
//! makes hover and click work on a panel. A panel that is modal -- the save picker -- also says so
//! through its `wants_input` function, and while any panel does, [`Panels::message_filter`]
//! holds keyboard, mouse and raw-input messages back from the game's own window procedure. That
//! stops the pause menu reacting to a click meant for the panel. It does not stop the game's
//! DirectInput and XInput polls, which do not go through window messages; a modal panel has to
//! handle those itself.

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use hudhook::imgui::{Context, FontConfig, FontSource, Io, Ui};
use hudhook::windows::Win32::Graphics::Dxgi::IDXGISwapChain;
use hudhook::windows::core::Interface;
use hudhook::{ImguiRenderLoop, MessageFilter, RenderContext};

use crate::log::log;

/// Signature of a panel's draw function. Called once per frame with the frame's `Ui`.
pub type DrawFn = fn(&Ui);

/// Signature of a panel's modality check: `true` while it wants every keyboard and mouse message
/// kept from the game window.
pub type WantsInputFn = fn() -> bool;

/// How many panels can be registered.
pub const PANEL_SLOTS: usize = 4;

/// The draw functions, as plain addresses. `0` is an empty slot.
static DRAWS: [AtomicUsize; PANEL_SLOTS] = [const { AtomicUsize::new(0) }; PANEL_SLOTS];

/// Signature of a panel's visibility check: `true` while it has something to draw.
pub type VisibleFn = fn() -> bool;

/// The visibility checks, index-matched to [`DRAWS`].
static VISIBLE: [AtomicUsize; PANEL_SLOTS] = [const { AtomicUsize::new(0) }; PANEL_SLOTS];

/// Whether any registered panel has something to draw this frame.
fn any_visible() -> bool {
    VISIBLE.iter().any(|slot| {
        let raw = slot.load(Ordering::Acquire);
        // SAFETY: a nonzero slot only ever holds a `VisibleFn` stored by `add_panel`.
        raw != 0 && unsafe { core::mem::transmute::<usize, VisibleFn>(raw) }()
    })
}

/// The modality checks, index-matched to [`DRAWS`]. `0` means never modal.
static WANTS_INPUT: [AtomicUsize; PANEL_SLOTS] = [const { AtomicUsize::new(0) }; PANEL_SLOTS];

/// Set once the render loop has been handed to hudhook.
static LOOP_SET: AtomicBool = AtomicBool::new(false);

/// `render_shared` failures, for the log's back-off.
static ERRORS: AtomicU64 = AtomicU64::new(0);

/// imgui's default font rasterises at 13 px; read across a room, it wants 25% more.
pub const FONT_SIZE_PX: f32 = 13.0 * 1.25;

/// Register a panel. `true` when it is registered now, including when `draw` already was;
/// `false` when every slot holds some other panel.
///
/// `visible` answers whether the panel has anything on screen this frame. No imgui frame is
/// rendered at all unless one panel says yes, because rendering one touches the back buffer: with
/// the DS2LE `PathTracing` `dxgi.dll` proxy installed, an empty imgui frame every `Present` froze the
/// picture on the last frame drawn while the game went on underneath (seen 2026-09-27: the terms
/// screen still showing over a live title menu).
///
/// `wants_input` is `None` for a panel that never needs the game kept away from the mouse and
/// keyboard, such as a status bar.
pub fn add_panel(draw: DrawFn, visible: VisibleFn, wants_input: Option<WantsInputFn>) -> bool {
    let raw = draw as usize;
    for (index, slot) in DRAWS.iter().enumerate() {
        match slot.compare_exchange(0, raw, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => {
                VISIBLE[index].store(visible as usize, Ordering::Release);
                WANTS_INPUT[index].store(wants_input.map_or(0, |f| f as usize), Ordering::Release);
                return true;
            }
            Err(existing) if existing == raw => return true,
            Err(_) => {}
        }
    }
    false
}

/// Whether any registered panel is modal right now.
#[must_use]
pub fn any_wants_input() -> bool {
    WANTS_INPUT.iter().any(|slot| {
        let raw = slot.load(Ordering::Acquire);
        // SAFETY: a nonzero slot only ever holds a `WantsInputFn` stored by `add_panel`.
        raw != 0 && unsafe { core::mem::transmute::<usize, WantsInputFn>(raw) }()
    })
}

/// The one render loop hudhook holds.
struct Panels;

impl ImguiRenderLoop for Panels {
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
            "panels: hudhook render loop initialized (font {FONT_SIZE_PX}px)"
        ));
    }

    fn render(&mut self, ui: &mut Ui) {
        for slot in &DRAWS {
            let raw = slot.load(Ordering::Acquire);
            if raw == 0 {
                continue;
            }
            // SAFETY: a nonzero slot only ever holds a `DrawFn` stored by `add_panel`.
            let draw: DrawFn = unsafe { core::mem::transmute::<usize, DrawFn>(raw) };
            draw(ui);
        }
    }

    fn message_filter(&self, _io: &Io) -> MessageFilter {
        if any_wants_input() {
            MessageFilter::InputAll
        } else {
            MessageFilter::empty()
        }
    }
}

/// Draw one imgui frame with every panel onto the swap chain being presented. Called by the
/// `Present` detour after the drawers. Does nothing until a panel is registered, so a session
/// with no panel never builds an imgui pipeline or subclasses the window.
pub(crate) fn render(swap_chain: *mut c_void) {
    if swap_chain.is_null() || !any_visible() {
        return;
    }
    if !LOOP_SET.swap(true, Ordering::AcqRel) {
        // SAFETY: the only use of hudhook's dx11 statics in this process; `ImguiDx11Hooks` is
        // never built.
        if !unsafe { hudhook::hooks::dx11::set_shared_render_loop(Panels) } {
            log(format_args!(
                "panels: hudhook already had a render loop -- panels may not draw"
            ));
        }
    }
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
                "panels: render failed ({errors} so far): {error:?} -- the frame is presented \
                 without them"
            ));
        }
    }
}
