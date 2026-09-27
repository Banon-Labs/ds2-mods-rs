//! The detour on `IDXGISwapChain::Present`, and the thread that finds where to put it.
//!
//! # Where the hook goes, and how its address is found
//!
//! `Present` is not an export. It is slot 8 of `IDXGISwapChain`'s vtable, and it is read off the
//! game's own swap chain: [`install`] starts a thread that waits for the pointer `ds2-rva` records
//! at `GRAPHICS_DEVICE`, reads the slot and hooks it. Nothing here creates a device or a swap
//! chain -- a throwaway one is a whole second renderer to a `dxgi.dll` proxy such as DS2LE
//! `PathTracing`, and releasing it crashed the game.
//!
//! **This hooks the DXGI module that owns the swap chain, not `DarkSoulsII.exe`** -- the system
//! `dxgi.dll`, or a proxy in the game folder when one is installed; the log line names which.
//! Everything the workspace has learned about Arxan (bd `arxan-vs-minhook-answered-properly-2026-08-26`)
//! is about detours inside the game image, where the integrity checks live. The hook is outside
//! all of that; only the pointer walk reads the game image.
//!
//! # The frame is shown whatever happens here
//!
//! Every table runs before the real `Present` and none of them can stop it being called: a
//! drawer or panel that fails is logged by its owner and the frame goes out without it.

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ds2_hook::{MH_Initialize, MhHook};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleFileNameW, GetModuleHandleExW,
};
use windows::core::PCWSTR;

use crate::frame_hook::{run_drawers, run_frame_hook};
use crate::log::log;

/// The trampoline back to the real `Present`.
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);

/// Set by the first [`install`], so every later call is a no-op that answers the first one's
/// outcome instead of starting a second watcher.
static STARTED: AtomicBool = AtomicBool::new(false);

/// Whether the watcher thread started. Only meaningful once [`STARTED`] is set.
static WATCHING: AtomicBool = AtomicBool::new(false);

/// Set on the first frame the detour runs, so that fact is logged once.
static FIRST_FRAME: AtomicBool = AtomicBool::new(false);

/// `IDXGISwapChain::Present`, as the detour must declare it.
type PresentFn = unsafe extern "system" fn(*mut c_void, u32, u32) -> windows::core::HRESULT;

/// Our `Present`: clock, drawers, panels, then the real one.
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
    if !FIRST_FRAME.swap(true, Ordering::Relaxed) {
        // Without this "nothing drew" and "the detour is never called" are the same silence.
        log(format_args!("first frame: the Present detour is running"));
    }
    run_frame_hook();
    run_drawers(swap_chain);
    crate::panels::render(swap_chain);
    let original = ORIGINAL.load(Ordering::Acquire);
    if original == 0 {
        return windows::core::HRESULT(0);
    }
    // SAFETY: the trampoline MinHook produced for `Present`, with its own arguments.
    unsafe {
        let real: PresentFn = core::mem::transmute::<usize, PresentFn>(original);
        real(swap_chain, sync_interval, flags)
    }
}

/// Slot 8 of `IDXGISwapChain`: `QueryInterface`, `AddRef`, `Release`, `SetPrivateData`,
/// `SetPrivateDataInterface`, `GetPrivateData`, `GetParent`, `GetDevice`, then `Present`.
const PRESENT_VTABLE_SLOT: usize = 8;

/// How often the watcher looks for the game's swap chain before it exists.
const SWAP_CHAIN_POLL: core::time::Duration = core::time::Duration::from_millis(50);

/// How long the watcher waits before saying, once, that the swap chain has not appeared.
const SWAP_CHAIN_SLOW: core::time::Duration = core::time::Duration::from_secs(30);

/// The game's own `IDXGISwapChain`, or `None` while `graphics-init` has not finished.
///
/// `[GRAPHICS_DEVICE] + 0xd20` is the holder, and its first qword is the chain; the hops and the
/// disassembly behind them are on the constants in `ds2-rva`. Every read goes through
/// `safe_read_usize`, so a hop that is not mapped yet is `None` rather than a fault.
fn game_swap_chain() -> Option<usize> {
    let global = ds2_game_base::mem::game_rva(ds2_rva::GRAPHICS_DEVICE).ok()?;
    let non_null = |value: usize| (value != 0).then_some(value);
    // SAFETY: each read is `ReadProcessMemory` on this process, which answers `None` for an
    // unmapped address instead of faulting.
    unsafe {
        let device = non_null(ds2_game_base::mem::safe_read_usize(global)?)?;
        let holder = non_null(ds2_game_base::mem::safe_read_usize(
            device + ds2_rva::GRAPHICS_DEVICE_SWAP_CHAIN_HOLDER_OFFSET,
        )?)?;
        non_null(ds2_game_base::mem::safe_read_usize(
            holder + ds2_rva::SWAP_CHAIN_HOLDER_DXGI_OFFSET,
        )?)
    }
}

/// `Present` off the game's own swap chain's vtable.
fn present_address(chain: usize) -> Option<usize> {
    // SAFETY: as in `game_swap_chain` -- guarded reads of a COM object's vtable pointer and one of
    // its slots.
    unsafe {
        let vtable = ds2_game_base::mem::safe_read_usize(chain)?;
        let present = ds2_game_base::mem::safe_read_usize(
            vtable + PRESENT_VTABLE_SLOT * core::mem::size_of::<usize>(),
        )?;
        (present != 0).then_some(present)
    }
}

/// The file name of the module holding `address`, for the log line. `?` when there is none.
///
/// With a `dxgi.dll` proxy installed the slot lands in the proxy rather than the system DLL, and
/// that is worth one line per run: it is what says which `Present` the overlay is drawing through.
fn owning_module(address: usize) -> String {
    let mut module = HMODULE::default();
    // SAFETY: FROM_ADDRESS reads `address` as an address, not a string; UNCHANGED_REFCOUNT takes
    // no reference, so there is nothing to release.
    let found = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(address as *const u16),
            &mut module,
        )
    };
    if found.is_err() {
        return String::from("?");
    }
    let mut path = [0u16; 260];
    // SAFETY: a module handle just returned, and a buffer this frame owns.
    let length = unsafe { GetModuleFileNameW(Some(module), &mut path) } as usize;
    let path = String::from_utf16_lossy(&path[..length.min(path.len())]);
    path.rsplit(['\\', '/']).next().unwrap_or("?").to_owned()
}

/// Detour [`present`], once the address is known. `false` has been logged.
///
/// # Safety
///
/// `address` must be `Present` on a live swap chain's vtable.
unsafe fn hook_present(address: usize) -> bool {
    // SAFETY: `MH_Initialize` takes no arguments and is safe to call again on an already-
    // initialised library, which the status below distinguishes.
    let status = unsafe { MH_Initialize() };
    if status != ds2_hook::MH_STATUS::MH_OK
        && status != ds2_hook::MH_STATUS::MH_ERROR_ALREADY_INITIALIZED
    {
        log(format_args!("MH_Initialize said {status:?}"));
        return false;
    }
    // SAFETY: `address` is slot 8 of the game's own `IDXGISwapChain` vtable and `present` matches
    // its ABI.
    let hook = match unsafe { MhHook::new(address as *mut c_void, present as *mut c_void) } {
        Ok(hook) => hook,
        Err(status) => {
            log(format_args!("MH_CreateHook said {status:?}"));
            return false;
        }
    };
    // The trampoline is stored before the hook is enabled, so the detour can never run without
    // one.
    ORIGINAL.store(hook.trampoline() as usize, Ordering::Release);
    // `ds2_hook::MH_EnableHook` patches at once, or queues into the loader's boot batch if that is
    // still open -- it will not be by the time a swap chain exists, but either is correct.
    // `MhHook` has no `Drop`; the detour belongs to MinHook's table from here on.
    // SAFETY: the target `MhHook::new` just registered.
    let status = unsafe { ds2_hook::MH_EnableHook(address as *mut c_void) };
    if status != ds2_hook::MH_STATUS::MH_OK {
        log(format_args!("MH_EnableHook said {status:?}"));
        return false;
    }
    true
}

/// Wait for the game's swap chain, then hook its `Present`. Runs on its own thread.
fn watch_for_swap_chain() {
    let started = std::time::Instant::now();
    let mut said_slow = false;
    let chain = loop {
        if let Some(chain) = game_swap_chain() {
            break chain;
        }
        if !said_slow && started.elapsed() >= SWAP_CHAIN_SLOW {
            said_slow = true;
            log(format_args!(
                "no swap chain at [GRAPHICS_DEVICE]+0x{:x} after {}s -- still waiting",
                ds2_rva::GRAPHICS_DEVICE_SWAP_CHAIN_HOLDER_OFFSET,
                SWAP_CHAIN_SLOW.as_secs()
            ));
        }
        std::thread::sleep(SWAP_CHAIN_POLL);
    };
    let Some(address) = present_address(chain) else {
        log(format_args!(
            "the game's swap chain 0x{chain:x} has no readable Present -- nothing draws or ticks \
             this session"
        ));
        return;
    };
    // SAFETY: `address` was read off the game's own live swap chain just above.
    if unsafe { hook_present(address) } {
        log(format_args!(
            "Present hooked at 0x{address:x} in {} (the game's own swap chain 0x{chain:x}, found \
             after {} ms)",
            owning_module(address),
            started.elapsed().as_millis()
        ));
    } else {
        log(format_args!(
            "not hooked -- nothing draws or ticks this session"
        ));
    }
}

/// Start the watcher that hooks `Present` once the game has a swap chain. Idempotent: every
/// feature that draws or ticks calls it, and only the first call starts a thread.
///
/// `false` means no watcher could be started and nothing will draw or tick this session; that has
/// been logged. `true` means the hook is on its way, and the watcher logs whether it went in.
pub fn install() -> bool {
    if STARTED.swap(true, Ordering::AcqRel) {
        return WATCHING.load(Ordering::Acquire);
    }
    let watching = match std::thread::Builder::new()
        .name(String::from("ds2-overlay-swapchain"))
        .spawn(watch_for_swap_chain)
    {
        Ok(_) => {
            log(format_args!(
                "waiting for the game's own swap chain -- none is created here"
            ));
            true
        }
        Err(error) => {
            log(format_args!(
                "no watcher thread ({error}) -- nothing draws or ticks this session"
            ));
            false
        }
    };
    WATCHING.store(watching, Ordering::Release);
    watching
}
