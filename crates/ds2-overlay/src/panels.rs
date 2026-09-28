//! Every imgui panel, drawn inside one hudhook frame.
//!
//! hudhook holds one render loop per process and subclasses the game window once for its input.
//! So there is one loop, `Panels`, and it calls each registered panel's draw function in the
//! order they were added. A panel that draws nothing this frame simply returns.
//!
//! # Input
//!
//! hudhook's window-procedure subclass feeds mouse and keyboard messages to imgui, which is what
//! makes hover and click work on a panel. A panel that is modal -- the save picker -- also says so
//! through its `wants_input` function, and while any panel does, `Panels::message_filter`
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
///
/// Eight: three were taken before the build recommender added a fourth, and a table that is full
/// on the day a panel is added leaves the next one failing at registration.
pub const PANEL_SLOTS: usize = 8;

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

/// Whether the filter the last rendered frame handed hudhook blocks the game's input.
static FILTER_BLOCKING: AtomicBool = AtomicBool::new(false);

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

/// Whether imgui's own mouse position is replaced by [`mouse`]. Off by default, so the panels that
/// hit-test [`mouse`] themselves see imgui exactly as before.
static IMGUI_MOUSE: AtomicBool = AtomicBool::new(false);

/// Make imgui's widgets use the corrected cursor, for a panel drawn with imgui's own sliders and
/// buttons. The panel turns it on when it opens and off when it closes.
pub fn use_overlay_mouse_for_imgui(on: bool) {
    IMGUI_MOUSE.store(on, Ordering::Release);
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

    /// Hand imgui the overlay's cursor while a panel built from imgui widgets asks for it.
    ///
    /// hudhook queues the cursor Wine reports, which is short by screen over window on a stretched
    /// fullscreen window (see [`mouse`]); a panel drawing imgui widgets rather than hit-testing
    /// [`mouse`] itself would have every slider and button offset. Adding the corrected position
    /// last makes it the one imgui uses, and turning the trickle queue off applies it on the same
    /// frame as a button press queued before it, so the press lands where the pointer is.
    fn before_render<'a>(
        &'a mut self,
        ctx: &mut Context,
        _render_context: &'a mut dyn RenderContext,
    ) {
        let on = IMGUI_MOUSE.load(Ordering::Acquire);
        let io = ctx.io_mut();
        io.config_input_trickle_event_queue = !on;
        if on && let Some(position) = mouse() {
            io.add_mouse_pos_event(position);
        }
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
        let blocking = any_wants_input();
        FILTER_BLOCKING.store(blocking, Ordering::Release);
        if blocking {
            MessageFilter::InputAll
        } else {
            MessageFilter::empty()
        }
    }
}

/// The cursor in back-buffer pixels, as `f32` bits, `x` high and `y` low; `u64::MAX` when the
/// cursor could not be read.
static MOUSE: AtomicU64 = AtomicU64::new(u64::MAX);

/// Set once the stretch decision has been logged.
static STRETCH_LOGGED: AtomicBool = AtomicBool::new(false);

/// Set once the window and back-buffer sizes have been logged.
static SIZES_LOGGED: AtomicBool = AtomicBool::new(false);

/// The cursor in the pixels panels draw in, or `None` when it could not be read this frame.
///
/// Use this, not `ui.io().mouse_pos`, for hit tests. hudhook feeds imgui the cursor Wine reports,
/// and with the game's window stretched over a larger screen that is short by screen over window:
/// the player saw the highlighted row at 0.59 of the pointer's position (2026-09-27).
/// This maps the cursor into the window's client area, undoes a stretched fullscreen window (see
/// `measure_mouse`), and scales by back buffer over client.
#[must_use]
pub fn mouse() -> Option<[f32; 2]> {
    let raw = MOUSE.load(Ordering::Acquire);
    (raw != u64::MAX).then(|| {
        [
            f32::from_bits((raw >> 32) as u32),
            f32::from_bits(raw as u32),
        ]
    })
}

/// Recompute [`MOUSE`] for this frame from the swap chain's own window.
fn measure_mouse(chain: &IDXGISwapChain) {
    use hudhook::windows::Win32::Foundation::{POINT, RECT};
    use hudhook::windows::Win32::Graphics::Gdi::ScreenToClient;
    use hudhook::windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetCursorPos};

    MOUSE.store(u64::MAX, Ordering::Release);
    // SAFETY: a live swap chain, borrowed for this `Present`.
    let Ok(desc) = (unsafe { chain.GetDesc() }) else {
        return;
    };
    let window = desc.OutputWindow;
    let mut client = RECT::default();
    let mut cursor = POINT::default();
    // SAFETY: `window` is the swap chain's own output window; both out-pointers are locals.
    let read = unsafe {
        GetClientRect(window, &mut client).is_ok()
            && GetCursorPos(&mut cursor).is_ok()
            && ScreenToClient(window, &mut cursor).as_bool()
    };
    let (client_w, client_h) = (client.right - client.left, client.bottom - client.top);
    let (buffer_w, buffer_h) = (desc.BufferDesc.Width, desc.BufferDesc.Height);
    if !SIZES_LOGGED.swap(true, Ordering::AcqRel) {
        log(format_args!(
            "panels: window client {client_w}x{client_h}, back buffer {buffer_w}x{buffer_h} -- \
             the mouse is scaled by the ratio"
        ));
    }
    if !read || client_w <= 0 || client_h <= 0 {
        return;
    }
    // STRETCHED FULLSCREEN. Measured 2026-09-27: Wine reported a 3840x2160 screen and the game's
    // window as 2260x1272 at the origin, and that window filled the monitor. The panel's pointer
    // then sat at 0.59 of the real one on both axes -- 2260/3840 and 1272/2160 -- so the cursor
    // Wine hands back is short by screen over window. A window at the origin with the screen's
    // shape is taken to be stretched that way; any other window is used as measured.
    let (screen_w, screen_h) = {
        use hudhook::windows::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN,
        };
        // SAFETY: `GetSystemMetrics` reads a system value and touches no memory of ours.
        unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
    };
    let mut origin = POINT::default();
    // SAFETY: `origin` is a local; `window` is the swap chain's output window.
    let at_origin =
        unsafe { hudhook::windows::Win32::Graphics::Gdi::ClientToScreen(window, &mut origin) }
            .as_bool()
            && origin.x == 0
            && origin.y == 0;
    let same_shape =
        i64::from(screen_w) * i64::from(client_h) - i64::from(screen_h) * i64::from(client_w);
    let stretched = at_origin
        && screen_w > client_w
        && same_shape.unsigned_abs() <= u64::from(screen_w.unsigned_abs().max(1)) * 2;
    let (stretch_x, stretch_y) = if stretched {
        (
            screen_w as f32 / client_w as f32,
            screen_h as f32 / client_h as f32,
        )
    } else {
        (1.0, 1.0)
    };
    if !STRETCH_LOGGED.swap(true, Ordering::AcqRel) {
        log(format_args!(
            "panels: screen {screen_w}x{screen_h}, window client {client_w}x{client_h} at the \
             origin={at_origin} -- stretched={stretched}, cursor x{stretch_x:.3} y{stretch_y:.3}"
        ));
    }
    let x = cursor.x as f32 * stretch_x * buffer_w as f32 / client_w as f32;
    let y = cursor.y as f32 * stretch_y * buffer_h as f32 / client_h as f32;
    MOUSE.store(
        (u64::from(x.to_bits()) << 32) | u64::from(y.to_bits()),
        Ordering::Release,
    );
}

/// Draw one imgui frame with every panel onto the swap chain being presented. Called by the
/// `Present` detour after the drawers. Does nothing until a panel is registered, so a session
/// with no panel never builds an imgui pipeline or subclasses the window.
pub(crate) fn render(swap_chain: *mut c_void) {
    // hudhook's window procedure applies whatever filter the last rendered frame stored, so frames
    // keep rendering while that is still a block. Stopping at the frame the picker closed left
    // `InputAll` in place, and the game got no left click afterwards (2026-09-27).
    if swap_chain.is_null() || !(any_visible() || FILTER_BLOCKING.load(Ordering::Acquire)) {
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
    measure_mouse(chain);
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
