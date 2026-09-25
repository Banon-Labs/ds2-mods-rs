//! The overlay: one detour on `IDXGISwapChain::Present`, and a few hundred triangles.
//!
//! # Why there is no imgui here
//!
//! `../er-mods-rs/crates/er-invasion-path` draws through `hudhook` and imgui's draw list, and
//! arbitrates with the other Elden Ring DLLs over who owns the process's imgui context. Neither
//! half of that applies here. There is exactly one loadable module in this workspace
//! (`docs/LOADING.md`), so there is nobody to arbitrate with -- and what this draws is coloured
//! line segments, which is a vertex buffer and two four-line shaders. An immediate-mode GUI
//! library, its font atlas, its input capture and its dependency tree would all be carried to
//! reach `DrawList::add_line`.
//!
//! # Where the hook goes, and how its address is found
//!
//! `Present` is not an export. It is slot 8 of `IDXGISwapChain`'s vtable, and the only supported
//! way to learn that address is to have Direct3D build one: [`present_address`] creates a
//! throwaway device and swap chain against a message-only window, reads the slot, and releases
//! everything. The game's swap chain is a different object but the same class, so it shares the
//! vtable -- which is what makes one detour cover the real one.
//!
//! **This hooks a Microsoft DLL, not `DarkSoulsII.exe`.** Everything the workspace has learned
//! about Arxan (bd `arxan-vs-minhook-answered-properly-2026-08-26`) is about detours inside the
//! game image, where the integrity checks live. `dxgi.dll` is outside all of that.
//!
//! # State is saved and restored, and that is not optional
//!
//! The detour runs on the game's own device context, between the frame being finished and it
//! being shown. Every piece of pipeline state this touches -- render targets, viewport, shaders,
//! the blend and rasterizer and depth-stencil states, the input layout and the first vertex
//! buffer -- belongs to the renderer that is mid-frame. Changing one and not putting it back is
//! not a subtle bug: it is the game rendering the next frame with an overlay's pixel shader
//! bound.
//!
//! # Every step refuses rather than faults
//!
//! A missing `d3dcompiler_47.dll`, a device that will not make a buffer, a back buffer that will
//! not describe itself: each of those disables the overlay for the session and writes one line
//! saying so. None of them is allowed to become an exception inside `Present`, because an
//! exception there is a crash in someone's invasion with this mod's name on it.

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ds2_hook::{MH_ApplyQueued, MH_Initialize, MhHook};
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_VERTEX_BUFFER, D3D11_BLEND_DESC,
    D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE, D3D11_BLEND_OP_ADD, D3D11_BLEND_SRC_ALPHA,
    D3D11_BUFFER_DESC, D3D11_COLOR_WRITE_ENABLE_ALL, D3D11_CPU_ACCESS_WRITE, D3D11_CULL_NONE,
    D3D11_DEPTH_STENCIL_DESC, D3D11_FILL_SOLID, D3D11_INPUT_ELEMENT_DESC,
    D3D11_INPUT_PER_VERTEX_DATA, D3D11_MAP_WRITE_DISCARD, D3D11_RASTERIZER_DESC,
    D3D11_RENDER_TARGET_BLEND_DESC, D3D11_SDK_VERSION, D3D11_SUBRESOURCE_DATA, D3D11_USAGE_DYNAMIC,
    D3D11_VIEWPORT, D3D11CreateDeviceAndSwapChain, ID3D11BlendState, ID3D11Buffer,
    ID3D11DepthStencilState, ID3D11DepthStencilView, ID3D11Device, ID3D11DeviceContext,
    ID3D11InputLayout, ID3D11PixelShader, ID3D11RasterizerState, ID3D11RenderTargetView,
    ID3D11Texture2D, ID3D11VertexShader,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R32G32B32A32_FLOAT,
    DXGI_MODE_DESC, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_SWAP_CHAIN_DESC, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGISwapChain,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryA};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassExW,
    UnregisterClassW, WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
};
// `Interface` is the trait that gives every COM wrapper its `as_raw`, which is how the vtable is
// reached in `probe_swap_chain`.
use windows::core::{BOOL, Interface, PCSTR, PCWSTR, s, w};

use crate::log::log;

use crate::lines::Vertex;

/// Vertices one frame may draw before the rest are dropped.
///
/// Sized rather than grown: the buffer is created once with `D3D11_USAGE_DYNAMIC` and refilled
/// with `MAP_WRITE_DISCARD` every frame, which is the cheap path precisely because its size never
/// changes. Six vertices per segment, so this is about five thousand segments -- two orders of
/// magnitude more than six arrows.
pub(crate) const MAX_VERTICES: usize = 32_768;

/// The two shaders, as source, compiled once per session.
///
/// The vertex shader does the pixels-to-clip conversion the CPU would otherwise have to do per
/// vertex, and it is the only reason there is a constant buffer at all. `y` is flipped because
/// the projection in `crate::geometry` emits the convention the rest of the overlay uses --
/// origin top-left, `y` down -- and clip space is the other way up.
const SHADER_SOURCE: &[u8] = br#"
cbuffer Viewport : register(b0) { float2 inverse_size; float2 padding; };
struct VsIn  { float2 position : POSITION; float4 color : COLOR; };
struct VsOut { float4 position : SV_Position; float4 color : COLOR; };
VsOut vs_main(VsIn input) {
    VsOut output;
    output.position = float4(
        input.position.x * inverse_size.x * 2.0 - 1.0,
        1.0 - input.position.y * inverse_size.y * 2.0,
        0.0,
        1.0);
    output.color = input.color;
    return output;
}
float4 ps_main(VsOut input) : SV_Target { return input.color; }
"#;

/// The vertex layout, matching [`Vertex`] field for field.
const INPUT_LAYOUT: [D3D11_INPUT_ELEMENT_DESC; 2] = [
    D3D11_INPUT_ELEMENT_DESC {
        SemanticName: s!("POSITION"),
        SemanticIndex: 0,
        Format: DXGI_FORMAT_R32G32_FLOAT,
        InputSlot: 0,
        AlignedByteOffset: 0,
        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
        InstanceDataStepRate: 0,
    },
    D3D11_INPUT_ELEMENT_DESC {
        SemanticName: s!("COLOR"),
        SemanticIndex: 0,
        Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
        InputSlot: 0,
        AlignedByteOffset: 8,
        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
        InstanceDataStepRate: 0,
    },
];

/// `D3DCompile`, as the loader hands it over.
///
/// Declared by hand and reached through `GetProcAddress` rather than by linking
/// `d3dcompiler.lib`, so a prefix without the DLL is a refusal this can log instead of a module
/// the Windows loader declines to start.
type D3DCompileFn = unsafe extern "system" fn(
    src_data: *const c_void,
    src_data_size: usize,
    source_name: PCSTR,
    defines: *const c_void,
    include: *const c_void,
    entrypoint: PCSTR,
    target: PCSTR,
    flags1: u32,
    flags2: u32,
    code: *mut *mut c_void,
    error_msgs: *mut *mut c_void,
) -> windows::core::HRESULT;

/// Minimal `ID3DBlob`: a pointer to bytes and their length, at vtable slots 3 and 4.
///
/// Three COM methods and no generated binding for them in the feature set this crate takes, so
/// the two accessors are called through the vtable directly. The slot numbers are `IUnknown`'s
/// three (`QueryInterface`, `AddRef`, `Release`) followed by `GetBufferPointer` and
/// `GetBufferSize`, which is the whole interface.
#[repr(C)]
struct BlobVtable {
    query_interface: unsafe extern "system" fn(*mut c_void, *const c_void, *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    get_buffer_pointer: unsafe extern "system" fn(*mut c_void) -> *mut c_void,
    get_buffer_size: unsafe extern "system" fn(*mut c_void) -> usize,
}

/// An `ID3DBlob` that releases itself.
struct Blob(*mut c_void);

impl Blob {
    /// The bytes, or an empty slice if the blob is somehow empty.
    ///
    /// # Safety
    ///
    /// The returned slice borrows the blob's own allocation and must not outlive it.
    unsafe fn bytes(&self) -> &[u8] {
        // SAFETY: a non-null `ID3DBlob` from `D3DCompile`; its first field is its vtable.
        unsafe {
            let vtable = *(self.0 as *const *const BlobVtable);
            let pointer = ((*vtable).get_buffer_pointer)(self.0).cast::<u8>();
            let size = ((*vtable).get_buffer_size)(self.0);
            if pointer.is_null() || size == 0 {
                return &[];
            }
            core::slice::from_raw_parts(pointer, size)
        }
    }
}

impl Drop for Blob {
    fn drop(&mut self) {
        if self.0.is_null() {
            return;
        }
        // SAFETY: released exactly once, from the owner, on the thread that created it.
        unsafe {
            let vtable = *(self.0 as *const *const BlobVtable);
            ((*vtable).release)(self.0);
        }
    }
}

/// Compile one shader entry point, or `None` with the reason already logged.
fn compile(compiler: D3DCompileFn, entry: PCSTR, target: PCSTR) -> Option<Blob> {
    let mut code: *mut c_void = core::ptr::null_mut();
    let mut errors: *mut c_void = core::ptr::null_mut();
    // SAFETY: `compiler` is `D3DCompile` resolved from `d3dcompiler_47.dll`; the source is a
    // static byte string and both out-pointers are owned locals.
    let hresult = unsafe {
        compiler(
            SHADER_SOURCE.as_ptr().cast(),
            SHADER_SOURCE.len(),
            s!("ds2-invasion-path"),
            core::ptr::null(),
            core::ptr::null(),
            entry,
            target,
            0,
            0,
            &mut code,
            &mut errors,
        )
    };
    // The error blob is freed whether or not the compile succeeded: `D3DCompile` may emit
    // warnings alongside a successful result, and leaking one per session is still a leak.
    let errors = if errors.is_null() {
        None
    } else {
        Some(Blob(errors))
    };
    if hresult.is_err() || code.is_null() {
        let detail = errors
            .as_ref()
            // SAFETY: the slice is used before `errors` is dropped at the end of this function.
            .map(|blob| String::from_utf8_lossy(unsafe { blob.bytes() }).into_owned())
            .unwrap_or_default();
        log(format_args!(
            "overlay: shader compile failed hresult=0x{:08x} {detail}",
            hresult.0
        ));
        return None;
    }
    Some(Blob(code))
}

/// Everything the overlay needs from one device, built once.
struct Resources {
    device: ID3D11Device,
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    input_layout: ID3D11InputLayout,
    vertices: ID3D11Buffer,
    constants: ID3D11Buffer,
    blend: ID3D11BlendState,
    rasterizer: ID3D11RasterizerState,
    depth_stencil: ID3D11DepthStencilState,
}

impl Resources {
    /// Build everything, or `None` with the reason logged.
    fn new(device: &ID3D11Device) -> Option<Self> {
        // SAFETY: `LoadLibraryA` with a static, NUL-terminated name.
        let module = unsafe { LoadLibraryA(s!("d3dcompiler_47.dll")) }.ok()?;
        // SAFETY: `module` is a live handle from the call above.
        let Some(entry) = (unsafe { GetProcAddress(module, s!("D3DCompile")) }) else {
            log(format_args!(
                "overlay: d3dcompiler_47.dll has no D3DCompile -- no overlay this session"
            ));
            return None;
        };
        // SAFETY: `GetProcAddress` returned a function pointer for the documented signature of
        // `D3DCompile`, which `D3DCompileFn` transcribes.
        let compiler: D3DCompileFn = unsafe { core::mem::transmute(entry) };

        let vertex_code = compile(compiler, s!("vs_main"), s!("vs_4_0"))?;
        let pixel_code = compile(compiler, s!("ps_main"), s!("ps_4_0"))?;
        // SAFETY: both slices borrow blobs that outlive these calls, and Direct3D copies the
        // bytecode it is given.
        let (vertex_bytes, pixel_bytes) = unsafe { (vertex_code.bytes(), pixel_code.bytes()) };

        let mut vertex_shader = None;
        let mut pixel_shader = None;
        let mut input_layout = None;
        let mut vertices = None;
        let mut constants = None;
        let mut blend = None;
        let mut rasterizer = None;
        let mut depth_stencil = None;

        let vertex_buffer_desc = D3D11_BUFFER_DESC {
            ByteWidth: (MAX_VERTICES * core::mem::size_of::<Vertex>()) as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_VERTEX_BUFFER.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        // Sixteen bytes: two floats of inverse viewport size and two of padding, because a
        // constant buffer's size must be a multiple of sixteen.
        let initial_constants = [0.0f32; 4];
        let constant_buffer_desc = D3D11_BUFFER_DESC {
            ByteWidth: 16,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let constant_data = D3D11_SUBRESOURCE_DATA {
            pSysMem: initial_constants.as_ptr().cast(),
            SysMemPitch: 0,
            SysMemSlicePitch: 0,
        };

        // Straight alpha blending, and the destination alpha is left alone: the back buffer's
        // alpha channel is not the game's transparency, and writing into it has produced
        // washed-out frames on some drivers.
        let mut blend_desc = D3D11_BLEND_DESC::default();
        blend_desc.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: BOOL(1),
            SrcBlend: D3D11_BLEND_SRC_ALPHA,
            DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_ONE,
            DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
        };

        let rasterizer_desc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            // No culling: the quads are built in screen space from whatever order the projection
            // produced, so half of them would be wound the other way and vanish.
            CullMode: D3D11_CULL_NONE,
            FrontCounterClockwise: BOOL(0),
            DepthBias: 0,
            DepthBiasClamp: 0.0,
            SlopeScaledDepthBias: 0.0,
            DepthClipEnable: BOOL(0),
            ScissorEnable: BOOL(0),
            MultisampleEnable: BOOL(0),
            AntialiasedLineEnable: BOOL(0),
        };

        // Depth off, stencil off: the overlay is meant to be visible through the world. A line to
        // a player behind a wall is the entire point of pointing at them.
        let depth_desc = D3D11_DEPTH_STENCIL_DESC {
            DepthEnable: BOOL(0),
            StencilEnable: BOOL(0),
            ..Default::default()
        };

        // SAFETY: every call below is a `ID3D11Device` creation method given a fully initialised
        // descriptor and an owned out-parameter. Each is checked before the next is attempted.
        unsafe {
            device
                .CreateVertexShader(vertex_bytes, None, Some(&mut vertex_shader))
                .ok()?;
            device
                .CreatePixelShader(pixel_bytes, None, Some(&mut pixel_shader))
                .ok()?;
            device
                .CreateInputLayout(&INPUT_LAYOUT, vertex_bytes, Some(&mut input_layout))
                .ok()?;
            device
                .CreateBuffer(&vertex_buffer_desc, None, Some(&mut vertices))
                .ok()?;
            device
                .CreateBuffer(
                    &constant_buffer_desc,
                    Some(&constant_data),
                    Some(&mut constants),
                )
                .ok()?;
            device
                .CreateBlendState(&blend_desc, Some(&mut blend))
                .ok()?;
            device
                .CreateRasterizerState(&rasterizer_desc, Some(&mut rasterizer))
                .ok()?;
            device
                .CreateDepthStencilState(&depth_desc, Some(&mut depth_stencil))
                .ok()?;
        }

        Some(Self {
            device: device.clone(),
            vertex_shader: vertex_shader?,
            pixel_shader: pixel_shader?,
            input_layout: input_layout?,
            vertices: vertices?,
            constants: constants?,
            blend: blend?,
            rasterizer: rasterizer?,
            depth_stencil: depth_stencil?,
        })
    }
}

/// Everything the pipeline held before the overlay touched it.
///
/// Restored in [`Drop`] rather than at the end of the draw so an early return cannot skip it.
struct SavedState {
    context: ID3D11DeviceContext,
    render_targets: [Option<ID3D11RenderTargetView>; 1],
    depth_view: Option<ID3D11DepthStencilView>,
    viewports: Vec<D3D11_VIEWPORT>,
    input_layout: Option<ID3D11InputLayout>,
    vertex_buffer: [Option<ID3D11Buffer>; 1],
    vertex_stride: [u32; 1],
    vertex_offset: [u32; 1],
    topology: windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY,
    vertex_shader: Option<ID3D11VertexShader>,
    pixel_shader: Option<ID3D11PixelShader>,
    vertex_constants: [Option<ID3D11Buffer>; 1],
    blend: Option<ID3D11BlendState>,
    blend_factor: [f32; 4],
    blend_mask: u32,
    rasterizer: Option<ID3D11RasterizerState>,
    depth_stencil: Option<ID3D11DepthStencilState>,
    stencil_reference: u32,
}

impl SavedState {
    /// Capture everything this module is about to change.
    fn capture(context: &ID3D11DeviceContext) -> Self {
        let mut render_targets: [Option<ID3D11RenderTargetView>; 1] = [None];
        let mut depth_view: Option<ID3D11DepthStencilView> = None;
        // `RSGetViewports` is asked for its count first: the game may have more than one bound,
        // and restoring only the first would silently resize whatever used the others.
        let mut viewport_count = 0u32;
        let mut vertex_buffer: [Option<ID3D11Buffer>; 1] = [None];
        let mut vertex_stride = [0u32; 1];
        let mut vertex_offset = [0u32; 1];
        let mut vertex_shader: Option<ID3D11VertexShader> = None;
        let mut pixel_shader: Option<ID3D11PixelShader> = None;
        let mut vertex_constants: [Option<ID3D11Buffer>; 1] = [None];
        let mut blend: Option<ID3D11BlendState> = None;
        let mut blend_factor = [0.0f32; 4];
        let mut blend_mask = 0u32;
        let mut depth_stencil: Option<ID3D11DepthStencilState> = None;
        let mut stencil_reference = 0u32;

        // SAFETY: every call is a `Get` on the immediate context, writing into owned locals. The
        // `Get` family AddRefs what it returns; the `Option<Interface>`s free those on drop.
        let (topology, viewports, input_layout, rasterizer) = unsafe {
            context.OMGetRenderTargets(Some(&mut render_targets), Some(&mut depth_view));
            context.RSGetViewports(&mut viewport_count, None);
            let mut viewports = vec![D3D11_VIEWPORT::default(); viewport_count as usize];
            if viewport_count > 0 {
                context.RSGetViewports(&mut viewport_count, Some(viewports.as_mut_ptr()));
            }
            // These two return a `Result` rather than filling an out-parameter, and an `Err`
            // means "nothing was bound" rather than a failure -- which is a state that must be
            // restored as faithfully as any other, hence `ok()` and not `?`.
            let input_layout = context.IAGetInputLayout().ok();
            context.IAGetVertexBuffers(
                0,
                1,
                Some(vertex_buffer.as_mut_ptr()),
                Some(vertex_stride.as_mut_ptr()),
                Some(vertex_offset.as_mut_ptr()),
            );
            let topology = context.IAGetPrimitiveTopology();
            context.VSGetShader(&mut vertex_shader, None, None);
            context.PSGetShader(&mut pixel_shader, None, None);
            context.VSGetConstantBuffers(0, Some(&mut vertex_constants));
            context.OMGetBlendState(
                Some(&mut blend),
                Some(&mut blend_factor),
                Some(&mut blend_mask),
            );
            let rasterizer = context.RSGetState().ok();
            context.OMGetDepthStencilState(Some(&mut depth_stencil), Some(&mut stencil_reference));
            (topology, viewports, input_layout, rasterizer)
        };

        Self {
            context: context.clone(),
            render_targets,
            depth_view,
            viewports,
            input_layout,
            vertex_buffer,
            vertex_stride,
            vertex_offset,
            topology,
            vertex_shader,
            pixel_shader,
            vertex_constants,
            blend,
            blend_factor,
            blend_mask,
            rasterizer,
            depth_stencil,
            stencil_reference,
        }
    }
}

impl Drop for SavedState {
    fn drop(&mut self) {
        // SAFETY: each value was read from this same context moments ago and is being handed
        // back to it unchanged.
        unsafe {
            self.context
                .OMSetRenderTargets(Some(&self.render_targets), self.depth_view.as_ref());
            if !self.viewports.is_empty() {
                self.context.RSSetViewports(Some(&self.viewports));
            }
            self.context.IASetInputLayout(self.input_layout.as_ref());
            self.context.IASetVertexBuffers(
                0,
                1,
                Some(self.vertex_buffer.as_ptr()),
                Some(self.vertex_stride.as_ptr()),
                Some(self.vertex_offset.as_ptr()),
            );
            self.context.IASetPrimitiveTopology(self.topology);
            self.context.VSSetShader(self.vertex_shader.as_ref(), None);
            self.context.PSSetShader(self.pixel_shader.as_ref(), None);
            self.context
                .VSSetConstantBuffers(0, Some(&self.vertex_constants));
            self.context.OMSetBlendState(
                self.blend.as_ref(),
                Some(&self.blend_factor),
                self.blend_mask,
            );
            self.context.RSSetState(self.rasterizer.as_ref());
            self.context
                .OMSetDepthStencilState(self.depth_stencil.as_ref(), self.stencil_reference);
        }
    }
}

/// Draw `vertices` over the frame `swap_chain` is about to show.
///
/// Returns `false` when something refused, which disables the overlay rather than retrying every
/// frame -- a failure here repeats sixty times a second and would be a log file rather than a
/// diagnostic.
fn draw(swap_chain: &IDXGISwapChain, vertices: &[Vertex]) -> bool {
    if vertices.is_empty() {
        return true;
    }
    // SAFETY: `swap_chain` is the game's own, handed to the detour by the caller.
    let Ok(device) = (unsafe { swap_chain.GetDevice::<ID3D11Device>() }) else {
        log(format_args!(
            "overlay: the swap chain is not a D3D11 device"
        ));
        return false;
    };
    // SAFETY: slot 0 is the back buffer of any swap chain.
    let Ok(back_buffer) = (unsafe { swap_chain.GetBuffer::<ID3D11Texture2D>(0) }) else {
        log(format_args!("overlay: no back buffer"));
        return false;
    };

    let mut description = Default::default();
    // SAFETY: `back_buffer` is a live texture; `GetDesc` fills the out-parameter.
    unsafe { back_buffer.GetDesc(&mut description) };
    let (width, height) = (description.Width as f32, description.Height as f32);
    if width <= 0.0 || height <= 0.0 {
        return false;
    }

    let mut render_target: Option<ID3D11RenderTargetView> = None;
    // SAFETY: a render target view over the back buffer, with the texture's own format.
    if unsafe { device.CreateRenderTargetView(&back_buffer, None, Some(&mut render_target)) }
        .is_err()
    {
        log(format_args!("overlay: no render target view"));
        return false;
    }

    let resources = match resources(&device) {
        Some(resources) => resources,
        None => return false,
    };

    // SAFETY: the immediate context of a live device.
    let context = unsafe { device.GetImmediateContext() };
    let Ok(context) = context else {
        return false;
    };

    // Captured BEFORE anything is bound and restored when this value drops, which happens on
    // every path out of this function including the early returns below.
    let _saved = SavedState::capture(&context);

    let count = vertices.len().min(MAX_VERTICES);
    // SAFETY: a `DYNAMIC` buffer mapped `WRITE_DISCARD`, the only legal map for one.
    let mapped = unsafe {
        let mut mapped = Default::default();
        if context
            .Map(
                &resources.vertices,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                Some(&mut mapped),
            )
            .is_err()
        {
            return false;
        }
        mapped
    };
    // SAFETY: `pData` is a mapping of at least `MAX_VERTICES` vertices, and `count` is clamped
    // to that; the source is a slice of the same `repr(C)` type.
    unsafe {
        core::ptr::copy_nonoverlapping(vertices.as_ptr(), mapped.pData.cast::<Vertex>(), count);
        context.Unmap(&resources.vertices, 0);
    }

    let inverse = [1.0 / width, 1.0 / height, 0.0, 0.0];
    // SAFETY: as above, for the sixteen-byte constant buffer.
    unsafe {
        let mut mapped = Default::default();
        if context
            .Map(
                &resources.constants,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                Some(&mut mapped),
            )
            .is_ok()
        {
            core::ptr::copy_nonoverlapping(inverse.as_ptr(), mapped.pData.cast::<f32>(), 4);
            context.Unmap(&resources.constants, 0);
        }
    }

    let viewport = D3D11_VIEWPORT {
        TopLeftX: 0.0,
        TopLeftY: 0.0,
        Width: width,
        Height: height,
        MinDepth: 0.0,
        MaxDepth: 1.0,
    };
    let stride = [core::mem::size_of::<Vertex>() as u32];
    let offset = [0u32];
    let vertex_buffers = [Some(resources.vertices.clone())];
    let constant_buffers = [Some(resources.constants.clone())];

    // SAFETY: every binding below is a live object created from this same device, and the state
    // they replace was captured above and is restored when `_saved` drops.
    unsafe {
        context.OMSetRenderTargets(Some(&[render_target]), None);
        context.RSSetViewports(Some(&[viewport]));
        context.RSSetState(&resources.rasterizer);
        context.OMSetBlendState(&resources.blend, Some(&[0.0f32; 4]), 0xffff_ffff);
        context.OMSetDepthStencilState(&resources.depth_stencil, 0);
        context.IASetInputLayout(&resources.input_layout);
        context.IASetVertexBuffers(
            0,
            1,
            Some(vertex_buffers.as_ptr()),
            Some(stride.as_ptr()),
            Some(offset.as_ptr()),
        );
        context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        context.VSSetShader(&resources.vertex_shader, None);
        context.VSSetConstantBuffers(0, Some(&constant_buffers));
        context.PSSetShader(&resources.pixel_shader, None);
        context.Draw(count as u32, 0);
    }
    true
}

/// The per-device resources, built on first use and rebuilt if the device changes.
///
/// A device change is a real event -- a resolution or fullscreen switch recreates it -- and
/// resources belonging to a dead device are not merely stale, they are a use-after-free waiting
/// for the next draw.
fn resources(device: &ID3D11Device) -> Option<&'static Resources> {
    // SAFETY: the detour runs on exactly one thread -- the one calling `Present` -- so this
    // `static mut` is only ever touched from there. It is `static` rather than a field because
    // the detour is a bare `extern "system"` function with nowhere to hang state.
    static mut RESOURCES: Option<Resources> = None;
    // SAFETY: as above. The reference handed back lives as long as the process.
    unsafe {
        let slot = &raw mut RESOURCES;
        if let Some(existing) = (*slot).as_ref()
            && existing.device == *device
        {
            return (*slot).as_ref();
        }
        *slot = Resources::new(device);
        (*slot).as_ref()
    }
}

/// Set once the overlay has refused, so a failure is logged once rather than every frame.
static DISABLED: AtomicBool = AtomicBool::new(false);

/// The trampoline back to the real `Present`.
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);

/// `IDXGISwapChain::Present`, as the detour must declare it.
type PresentFn = unsafe extern "system" fn(*mut c_void, u32, u32) -> windows::core::HRESULT;

/// Our `Present`. Draws, then calls the real one.
///
/// # Safety
///
/// Installed by MinHook over `IDXGISwapChain::Present`, whose ABI this matches. It is called by
/// Direct3D with a live swap chain.
unsafe extern "system" fn present(
    swap_chain: *mut c_void,
    sync_interval: u32,
    flags: u32,
) -> windows::core::HRESULT {
    let original = ORIGINAL.load(Ordering::Acquire);
    // The frame is shown whatever happens here. An overlay that can fail must not be able to
    // fail into a black screen.
    let call_original = |swap_chain: *mut c_void| {
        if original == 0 {
            return windows::core::HRESULT(0);
        }
        // SAFETY: the trampoline MinHook produced for `Present`, with its own arguments.
        unsafe {
            let real: PresentFn = core::mem::transmute(original);
            real(swap_chain, sync_interval, flags)
        }
    };

    if DISABLED.load(Ordering::Relaxed) || swap_chain.is_null() {
        return call_original(swap_chain);
    }

    // SAFETY: a live `IDXGISwapChain` pointer from Direct3D. `from_raw_borrowed` borrows without
    // taking a reference count, which is what a detour must do -- releasing the game's swap
    // chain would be catastrophic.
    let borrowed = unsafe { IDXGISwapChain::from_raw_borrowed(&swap_chain) };
    if let Some(chain) = borrowed {
        // THE CAMERA CAPTURE INSTALLS HERE, and it has to be here rather than inside `draw`.
        //
        // `draw` returns immediately when there are no vertices, and there are no vertices until
        // a camera is known, and a camera is not known until the capture has run -- so putting
        // the install down there made it unreachable by exactly the condition it exists to fix.
        // A live run found that: the log had no capture line at all.
        //
        // SAFETY: a borrowed live swap chain; `install_capture` is idempotent and does nothing
        // after the first success.
        unsafe { install_capture(chain) };
        let vertices = crate::frame(chain);
        if !draw(chain, &vertices) {
            DISABLED.store(true, Ordering::Relaxed);
            log(format_args!(
                "overlay: disabled for this session -- the frame is untouched from here on"
            ));
        }
    }
    call_original(swap_chain)
}

/// Set once the constant-buffer capture has been offered the context.
static CAPTURE_TRIED: AtomicBool = AtomicBool::new(false);

/// Hand the game's own immediate context to `crate::capture`, once.
///
/// # Safety
///
/// `chain` must be a live swap chain. Called from the `Present` detour.
unsafe fn install_capture(chain: &IDXGISwapChain) {
    if CAPTURE_TRIED.swap(true, Ordering::Relaxed) {
        return;
    }
    // SAFETY: the game's own swap chain, from which its device and that device's immediate
    // context are reached by the documented accessors.
    unsafe {
        let Ok(device) = chain.GetDevice::<ID3D11Device>() else {
            return;
        };
        let Ok(context) = device.GetImmediateContext() else {
            return;
        };
        crate::capture::install(&context);
    }
}

/// Find `IDXGISwapChain::Present` by making Direct3D build a swap chain and reading its vtable.
///
/// The throwaway window is never shown: `CreateWindowExW` without `WS_VISIBLE` on a class
/// registered for this purpose, destroyed before this returns along with the class itself. A
/// leaked window class is the kind of thing that makes a second call to this function fail with
/// an error nobody connects to the first.
fn present_address() -> Option<usize> {
    // SAFETY: the module handle of the running process, which always exists.
    let instance: HMODULE = unsafe { GetModuleHandleW(None) }.ok()?;
    let class_name = w!("ds2_invasion_path_probe");
    let class = WNDCLASSEXW {
        cbSize: core::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(probe_window_proc),
        hInstance: instance.into(),
        lpszClassName: class_name,
        ..Default::default()
    };
    // SAFETY: a fully initialised class descriptor with a static name and a real window
    // procedure.
    if unsafe { RegisterClassExW(&class) } == 0 {
        log(format_args!("overlay: could not register the probe class"));
        return None;
    }

    let result = probe_swap_chain(class_name, instance);

    // SAFETY: unregistering the class this function registered, after its only window is gone.
    unsafe {
        let _ = UnregisterClassW(class_name, Some(instance.into()));
    }
    result
}

/// The probe window's procedure: the default, for every message.
///
/// # Safety
///
/// Called by `user32` for a window this module created.
unsafe extern "system" fn probe_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: forwarding the message unchanged to the default handler.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// The body of [`present_address`], split out so the class is unregistered on every path.
fn probe_swap_chain(class_name: PCWSTR, instance: HMODULE) -> Option<usize> {
    // SAFETY: a window of the class registered immediately above. Not `WS_VISIBLE`, so nothing
    // appears on screen.
    let window = unsafe {
        CreateWindowExW(
            Default::default(),
            class_name,
            w!("ds2-invasion-path"),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            64,
            64,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
    .ok()?;

    let description = DXGI_SWAP_CHAIN_DESC {
        BufferDesc: DXGI_MODE_DESC {
            Width: 64,
            Height: 64,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            ..Default::default()
        },
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 1,
        OutputWindow: window,
        Windowed: BOOL(1),
        ..Default::default()
    };

    let mut swap_chain: Option<IDXGISwapChain> = None;
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    let levels = [D3D_FEATURE_LEVEL_11_0];
    // SAFETY: a fully initialised swap chain description over a window created above; every
    // out-parameter is an owned local.
    let created = unsafe {
        D3D11CreateDeviceAndSwapChain(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            Default::default(),
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&description),
            Some(&mut swap_chain),
            Some(&mut device),
            None,
            Some(&mut context),
        )
    };

    let address = match created {
        Ok(()) => swap_chain.as_ref().map(|chain| {
            // SAFETY: a live COM object -- its first field is a pointer to its vtable, and slot
            // 8 of `IDXGISwapChain` is `Present`: three `IUnknown` methods, four
            // `IDXGIObject`, one `IDXGIDeviceSubObject`.
            unsafe {
                let raw = chain.as_raw().cast::<*const usize>();
                *(*raw).add(PRESENT_VTABLE_SLOT)
            }
        }),
        Err(error) => {
            log(format_args!(
                "overlay: no probe device (0x{:08x}) -- no overlay this session",
                error.code().0
            ));
            None
        }
    };

    // Dropped before the window is destroyed, because a swap chain outliving its output window is
    // undefined and debug layers say so loudly.
    drop(context);
    drop(swap_chain);
    drop(device);
    // SAFETY: destroying the window this function created, which nothing else holds.
    unsafe {
        let _ = DestroyWindow(window);
    }
    address
}

/// Slot 8 of `IDXGISwapChain`: `QueryInterface`, `AddRef`, `Release`, `SetPrivateData`,
/// `SetPrivateDataInterface`, `GetPrivateData`, `GetParent`, `GetDevice`, then `Present`.
const PRESENT_VTABLE_SLOT: usize = 8;

/// Install the `Present` detour. `false` means no overlay this session, already logged.
///
/// # Safety
///
/// Installs a native code detour. Call once, from the loader's install position.
pub(crate) unsafe fn install() -> bool {
    let Some(address) = present_address() else {
        return false;
    };
    // SAFETY: MinHook's own initialiser; idempotent and safe to call when another module in this
    // DLL has already done it.
    // SAFETY: `MH_Initialize` takes no arguments and is safe to call again on an already-
    // initialised library, which the status below distinguishes.
    let status = unsafe { MH_Initialize() };
    if status != ds2_hook::MH_STATUS::MH_OK
        && status != ds2_hook::MH_STATUS::MH_ERROR_ALREADY_INITIALIZED
    {
        log(format_args!("overlay: MH_Initialize said {status:?}"));
        return false;
    }
    // SAFETY: `address` is slot 8 of a real `IDXGISwapChain` vtable and `present` matches its
    // ABI; the trampoline is stored before the hook is enabled, so the detour can never run
    // without one.
    // SAFETY: the target is an RVA this crate validated against the prologue it expects before
    // reaching here, and the detour is a `'static` fn item of the matching ABI.
    let hook = match unsafe { MhHook::new(address as *mut c_void, present as *mut c_void) } {
        Ok(hook) => hook,
        Err(status) => {
            log(format_args!("overlay: MH_CreateHook said {status:?}"));
            return false;
        }
    };
    ORIGINAL.store(hook.trampoline() as usize, Ordering::Release);
    // SAFETY: enabling the hook created immediately above.
    if let Err(status) = unsafe { hook.queue_enable() } {
        log(format_args!("overlay: queue_enable said {status:?}"));
        return false;
    }
    // SAFETY: applies this DLL's queued hooks. Other features in the loader queue theirs the
    // same way.
    if unsafe { MH_ApplyQueued() } != ds2_hook::MH_STATUS::MH_OK {
        log(format_args!("overlay: MH_ApplyQueued refused"));
        return false;
    }
    // `hook` goes out of scope here and that is FINE, which is worth one sentence because it
    // looks like a bug. `MhHook` is a handle, not a guard: it has no `Drop`, and the detour it
    // created is owned by MinHook's own table until `MH_DisableHook` or `MH_Uninitialize`.
    // Dropping the handle uninstalls nothing. An earlier version wrote `mem::forget` here to
    // "keep it alive", which clippy correctly called out as a no-op on a type with no destructor
    // -- and which would have quietly stopped meaning anything if `MhHook` ever grew one.
    log(format_args!(
        "overlay: Present hooked at 0x{address:x} -- the overlay can draw"
    ));
    true
}

// The screen-space expansion that turns projected points into these vertices lives in
// `crate::lines`, where `cargo test` on Linux can reach it.
