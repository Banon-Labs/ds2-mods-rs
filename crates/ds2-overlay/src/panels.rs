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

use hudhook::imgui::{Context, FontConfig, FontGlyphRanges, FontId, FontSource, Io, Ui};
use hudhook::windows::Win32::Graphics::Direct3D11::{D3D11_TEXTURE2D_DESC, ID3D11Texture2D};
use hudhook::windows::Win32::Graphics::Dxgi::IDXGISwapChain;
use hudhook::windows::core::Interface;
use hudhook::{ImguiRenderLoop, MessageFilter, RenderContext};

use crate::fefont::{self, FaceName};
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

/// Frames drawn with the corrected cursor, for the once-a-second diagnostic line.
static IMGUI_MOUSE_FRAMES: AtomicU64 = AtomicU64::new(0);

/// imgui's default font rasterises at 13 px; read across a room, it wants 25% more. Only used when
/// the game's own fonts cannot be read (see `install_game_fonts`).
pub const FONT_SIZE_PX: f32 = 13.0 * 1.25;

/// Where `FeFont_Big` sits in the atlas's font list, or `usize::MAX` while the game's fonts are
/// not loaded. `FeFont_Small` is font 0, the default every panel draws with.
static BIG_FONT: AtomicUsize = AtomicUsize::new(usize::MAX);

/// `FeFont_Big`, the game's title face, for a panel to push around its title. `None` when the
/// panels fell back to imgui's default font.
#[must_use]
pub fn big_font(ui: &Ui) -> Option<FontId> {
    let index = BIG_FONT.load(Ordering::Acquire);
    ui.fonts().fonts().get(index).copied()
}

/// The game's palette and square corners, pushed onto imgui's own widgets for as long as this
/// lives. For a panel built from imgui windows and widgets rather than the draw list.
pub struct GameStyle<'ui> {
    _colors: Vec<hudhook::imgui::ColorStackToken<'ui>>,
    _vars: Vec<hudhook::imgui::StyleStackToken<'ui>>,
}

/// Push [`GameStyle`]: `ds2_overlay::style`'s colours on every imgui widget colour a panel uses,
/// and no rounding anywhere.
#[must_use]
pub fn game_style(ui: &Ui) -> GameStyle<'_> {
    use crate::style::{
        ASH, BRONZE, BRONZE_DIM, INK_1, INK_2, PANEL_BG, SLATE, SLATE_EDIT, TEXT, with_alpha,
    };
    use hudhook::imgui::{StyleColor as C, StyleVar as V};
    let colors = [
        (C::WindowBg, PANEL_BG),
        (C::ChildBg, with_alpha(INK_1, 0.0)),
        (C::PopupBg, PANEL_BG),
        (C::Border, BRONZE),
        (C::TitleBg, INK_1),
        (C::TitleBgActive, INK_2),
        (C::TitleBgCollapsed, INK_1),
        (C::Text, TEXT),
        (C::TextDisabled, BRONZE),
        (C::FrameBg, SLATE),
        (C::FrameBgHovered, SLATE_EDIT),
        (C::FrameBgActive, SLATE_EDIT),
        (C::Button, INK_1),
        (C::ButtonHovered, INK_2),
        (C::ButtonActive, INK_2),
        (C::Header, INK_2),
        (C::HeaderHovered, INK_2),
        (C::HeaderActive, INK_2),
        (C::CheckMark, BRONZE),
        (C::SliderGrab, BRONZE_DIM),
        (C::SliderGrabActive, BRONZE),
        (C::ScrollbarBg, with_alpha(INK_1, 0.6)),
        (C::ScrollbarGrab, BRONZE_DIM),
        (C::ScrollbarGrabHovered, BRONZE),
        (C::ScrollbarGrabActive, BRONZE),
        (C::Separator, BRONZE_DIM),
        (C::TextSelectedBg, crate::style::TEXT_SELECTION),
        (C::ResizeGrip, with_alpha(ASH, 0.4)),
    ];
    let vars = [
        V::WindowRounding(0.0),
        V::ChildRounding(0.0),
        V::FrameRounding(0.0),
        V::PopupRounding(0.0),
        V::ScrollbarRounding(0.0),
        V::GrabRounding(0.0),
        V::WindowBorderSize(crate::style::FRAME_PX),
    ];
    GameStyle {
        _colors: colors
            .into_iter()
            .map(|(which, colour)| ui.push_style_color(which, colour))
            .collect(),
        _vars: vars.into_iter().map(|v| ui.push_style_var(v)).collect(),
    }
}

/// How tall [`title`] draws, for a panel laying out before it draws.
#[must_use]
pub fn title_height(ui: &Ui) -> f32 {
    let _big = big_font(ui).map(|id| ui.push_font(id));
    ui.current_font_size()
}

/// How wide [`title`] draws `text`.
#[must_use]
pub fn title_width(ui: &Ui, text: &str) -> f32 {
    let _big = big_font(ui).map(|id| ui.push_font(id));
    ui.calc_text_size(text)[0]
}

/// Draw a panel title at `pos` in `FeFont_Big` (the default font when the game's fonts did not
/// load). Answers the height it took, so the caller can move down by it.
pub fn title(
    ui: &Ui,
    list: &hudhook::imgui::DrawListMut<'_>,
    pos: [f32; 2],
    colour: [f32; 4],
    text: &str,
) -> f32 {
    let _big = big_font(ui).map(|id| ui.push_font(id));
    list.add_text(pos, colour, text);
    ui.current_font_size()
}

/// Draw a key-help bar from `origin`, stopping before `right`. Answers the height it took.
///
/// Each entry is a framed button name, then its verb: the player reads a button and a word, not a
/// sentence (docs/DS2-UI-DESIGN.md).
pub fn hint_bar(
    ui: &Ui,
    list: &hudhook::imgui::DrawListMut<'_>,
    origin: [f32; 2],
    right: f32,
    entries: &[(&str, &str)],
) -> f32 {
    let line = ui.current_font_size();
    let pad = (line * 0.2).round();
    let gap = (line * 0.9).round();
    let mut x = origin[0];
    for (button, verb) in entries {
        let button_w = ui.calc_text_size(button)[0] + pad * 2.0;
        let verb_w = ui.calc_text_size(verb)[0];
        if x + button_w + pad + verb_w > right {
            break;
        }
        list.add_rect(
            [x, origin[1] - 1.0],
            [x + button_w, origin[1] + line + 1.0],
            crate::style::BRONZE,
        )
        .thickness(crate::style::FRAME_PX)
        .build();
        list.add_text([x + pad, origin[1]], crate::style::BRONZE, button);
        x += button_w + pad;
        list.add_text([x, origin[1]], crate::style::TEXT, verb);
        x += verb_w + gap;
    }
    line + 2.0
}

/// The folder holding `DarkSoulsII.exe`, which is this process's executable.
fn game_dir() -> Option<std::path::PathBuf> {
    std::env::current_exe().ok()?.parent().map(Into::into)
}

/// Load `FeFont_Small` then `FeFont_Big` into imgui's atlas, so every panel draws in the game's
/// typeface at its own sizes (line heights 28 and 41). Answers how many glyphs went in, or why
/// nothing did; on failure the atlas is left as it was.
///
/// Each face is an imgui font with an empty glyph range, filled with one custom-rect glyph per
/// game glyph (imgui 1.89's `AddCustomRectFontGlyph`). The atlas is built here, and the game's
/// pixels are copied into the RGBA32 buffer that hudhook's `setup_fonts` then uploads as it is:
/// `build_rgba32_texture` does not rebuild an atlas that already has its pixels.
fn install_game_fonts(ctx: &mut Context) -> Result<usize, String> {
    use hudhook::imgui::internal::RawCast;
    use hudhook::imgui::sys;

    /// A range holding only the space, so the placeholder font imgui builds under each face
    /// rasterises one glyph; the custom glyphs replace even that one.
    static SPACE_ONLY: [u32; 3] = [0x20, 0x20, 0];

    let dir = game_dir().ok_or("no executable path")?;
    let small =
        fefont::load(&dir, FaceName::Small, fefont::latin).map_err(|e| format!("Small: {e}"))?;
    let big = fefont::load(&dir, FaceName::Big, fefont::latin).map_err(|e| format!("Big: {e}"))?;

    let fonts = ctx.fonts();
    let mut ids = Vec::with_capacity(2);
    for face in [&small, &big] {
        ids.push(fonts.add_font(&[FontSource::DefaultFontData {
            config: Some(FontConfig {
                size_pixels: f32::from(face.line_height),
                glyph_ranges: FontGlyphRanges::from_slice(&SPACE_ONLY),
                ..FontConfig::default()
            }),
        }]));
    }
    // SAFETY: `fonts` is the context's live atlas; imgui-rs's `FontAtlas` and `Font` are
    // `RawCast` views of the same `ImFontAtlas`/`ImFont`, which the calls below take by pointer.
    let atlas: *mut sys::ImFontAtlas = unsafe { fonts.raw_mut() };
    let mut rects = Vec::new();
    for (face, id) in [&small, &big].into_iter().zip(&ids) {
        let font = fonts.get_font(*id).ok_or("font vanished from the atlas")?;
        // SAFETY: as above; the pointer is only handed back to imgui.
        let raw = unsafe { font.raw() as *const sys::ImFont as *mut sys::ImFont };
        for glyph in &face.glyphs {
            // SAFETY: `atlas` and `raw` are live; imgui copies the arguments.
            let rect = unsafe {
                sys::ImFontAtlas_AddCustomRectFontGlyph(
                    atlas,
                    raw,
                    glyph.code,
                    i32::from(glyph.width()),
                    i32::from(glyph.height()),
                    glyph.pen_step() as f32,
                    sys::ImVec2 {
                        x: f32::from(glyph.pre_space),
                        y: 0.0,
                    },
                )
            };
            rects.push((rect, face, *glyph));
        }
    }
    // SAFETY: a live atlas with every font and custom rect registered.
    if !unsafe { sys::ImFontAtlas_Build(atlas) } {
        return Err("ImFontAtlas_Build failed".into());
    }
    let (mut pixels, mut width, mut height, mut bpp) = (core::ptr::null_mut(), 0, 0, 0);
    // SAFETY: the atlas is built; imgui hands back its own RGBA32 buffer.
    unsafe {
        sys::ImFontAtlas_GetTexDataAsRGBA32(atlas, &mut pixels, &mut width, &mut height, &mut bpp);
    }
    if pixels.is_null() || bpp != 4 {
        return Err(format!("atlas pixels unavailable (bpp {bpp})"));
    }
    let (width, height) = (width as usize, height as usize);
    // SAFETY: imgui owns `width * height * 4` bytes at `pixels` until the atlas is cleared.
    let atlas_px = unsafe { core::slice::from_raw_parts_mut(pixels, width * height * 4) };
    for (index, face, glyph) in &rects {
        // SAFETY: `index` came from `AddCustomRectFontGlyph` on this atlas.
        let rect = unsafe { &*sys::ImFontAtlas_GetCustomRectByIndex(atlas, *index) };
        let page = &face.pages[glyph.page];
        let (gw, gh) = (usize::from(glyph.width()), usize::from(glyph.height()));
        let (dx, dy) = (usize::from(rect.X), usize::from(rect.Y));
        if dx + gw > width || dy + gh > height {
            return Err("a packed glyph lies outside the atlas".into());
        }
        for row in 0..gh {
            let src =
                ((usize::from(glyph.rect[1]) + row) * page.width + usize::from(glyph.rect[0])) * 4;
            let dst = ((dy + row) * width + dx) * 4;
            atlas_px[dst..dst + gw * 4].copy_from_slice(&page.rgba[src..src + gw * 4]);
        }
    }
    BIG_FONT.store(1, Ordering::Release);
    Ok(rects.len())
}

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
        match install_game_fonts(ctx) {
            Ok(glyphs) => log(format_args!(
                "panels: hudhook render loop initialized (the game's FeFont_Small 28px default, \
                 FeFont_Big 41px for titles, {glyphs} glyphs)"
            )),
            Err(why) => {
                ctx.fonts().clear();
                ctx.fonts().add_font(&[FontSource::DefaultFontData {
                    config: Some(FontConfig {
                        size_pixels: FONT_SIZE_PX,
                        ..FontConfig::default()
                    }),
                }]);
                log(format_args!(
                    "panels: hudhook render loop initialized (imgui's default font {FONT_SIZE_PX}px: \
                     the game's fonts did not load: {why})"
                ));
            }
        }
        // Keep imgui's `imgui.ini` out of the game directory.
        ctx.set_ini_filename(None);
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
        // hudhook sizes imgui from the swap chain's description; the texture it draws into can be
        // larger (see `measure_mouse`), and then imgui's viewport and every window's clip end part
        // way across the screen. Imgui's display is the render target, and so is `mouse`.
        if let Some([width, height]) = target_size() {
            io.display_size = [width as f32, height as f32];
        }
        if on {
            if let Some(position) = mouse() {
                io.add_mouse_pos_event(position);
            }
            let now = IMGUI_MOUSE_FRAMES.fetch_add(1, Ordering::Relaxed);
            if now.is_multiple_of(60) {
                log(format_args!(
                    "panels: imgui display_size={:?} framebuffer_scale={:?} target={:?} mouse={:?}",
                    io.display_size,
                    io.display_framebuffer_scale,
                    target_size(),
                    mouse()
                ));
            }
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

/// Set once the window and back-buffer sizes have been logged.
static SIZES_LOGGED: AtomicBool = AtomicBool::new(false);

/// The cursor in the pixels panels draw in, or `None` when it could not be read this frame.
///
/// Use this, not `ui.io().mouse_pos`, for hit tests. hudhook feeds imgui the cursor in the window's
/// client pixels, and the render target imgui draws into can be larger than the client area: the
/// player saw the highlighted row at 0.59 of the pointer's position (2026-09-27), 2260/3840. This
/// maps the cursor into the client area and scales it by render target over client (see
/// `measure_mouse`).
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
    // The texture imgui is drawn into. The swap chain's description is not it: on 2026-09-28 it
    // said 1920x1080 while the Music window dragged to imgui x=1900 showed at the middle of a
    // 3840-wide screen, so imgui was drawing 1:1 into the left half of a larger target. This is
    // also what the 2026-09-27 "0.59 of the pointer" was: 2260/3840, a 2260-wide window over a
    // 3840-wide target.
    // SAFETY: a live swap chain, borrowed for this `Present`; `GetBuffer(0)` adds a reference the
    // returned interface releases when it drops, and `GetDesc` fills a local.
    let target = unsafe {
        chain.GetBuffer::<ID3D11Texture2D>(0).ok().map(|texture| {
            let mut texture_desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut texture_desc);
            [texture_desc.Width, texture_desc.Height]
        })
    }
    .filter(|[w, h]| *w > 0 && *h > 0)
    .unwrap_or([buffer_w, buffer_h]);
    TARGET.store(
        (u64::from(target[0]) << 32) | u64::from(target[1]),
        Ordering::Release,
    );
    if !SIZES_LOGGED.swap(true, Ordering::AcqRel) {
        log(format_args!(
            "panels: window client {client_w}x{client_h}, swap chain says {buffer_w}x{buffer_h}, \
             render target {}x{} -- imgui's display and the mouse are the render target's",
            target[0], target[1]
        ));
    }
    if !read || client_w <= 0 || client_h <= 0 {
        return;
    }
    // The cursor in client coordinates, scaled to the render target: Wine reports it across the
    // window's own client area however the window is stretched on the monitor.
    let x = cursor.x as f32 * target[0] as f32 / client_w as f32;
    let y = cursor.y as f32 * target[1] as f32 / client_h as f32;
    MOUSE.store(
        (u64::from(x.to_bits()) << 32) | u64::from(y.to_bits()),
        Ordering::Release,
    );
}

/// The render target's size, packed `w << 32 | h`; `0` before the first frame.
static TARGET: AtomicU64 = AtomicU64::new(0);

/// The size of the texture imgui is drawn into, once a frame has been measured.
#[must_use]
pub fn target_size() -> Option<[u32; 2]> {
    let raw = TARGET.load(Ordering::Acquire);
    (raw != 0).then_some([(raw >> 32) as u32, raw as u32])
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
