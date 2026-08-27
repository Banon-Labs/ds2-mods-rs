//! Shared MinHook FFI wrapper + cross-DLL hook union.
//!
//! Ported from `er-mods-rs/crates/er-hook`, behaviour-preserving: the MinHook-generic FFI
//! (`MH_*` externs, [`MH_STATUS`]), the [`MhHook`] wrapper, the hook union
//! ([`register_union_hook`] plus the cross-DLL chaining), and the raw code-patch primitives.
//! MinHook's C source is cc-compiled once here (build.rs) rather than in every DLL that wants a
//! detour.
//!
//! ZERO GAME KNOWLEDGE, BY CONSTRUCTION. Nothing in this crate names an address, an offset, a
//! structure layout or a function of DARK SOULS II -- or of Elden Ring, which is where the code
//! comes from. Every target address, expected byte and stub is a parameter the caller supplies.
//! That is exactly why this crate ports across an eight-year engine gap when almost nothing else
//! in that workspace does: it is a MinHook binding plus the bookkeeping that stops two DLLs from
//! fighting over one prologue, and neither of those is a fact about a game.
//!
//! The design fact that shapes the whole file: a statically linked crate's statics are PER DLL,
//! so every cdylib linking this crate owns its OWN MinHook instance. Within one DLL the union
//! chains handlers so none is dropped. Across DLLs, [`register_shared_hook`] routes a
//! registration into whichever DLL already owns the prologue, so a single instance owns it and
//! every handler still runs. The `#[unsafe(no_mangle)]` C export that makes that possible is
//! deliberately NOT here -- it belongs to the product DLL, so that exactly one module in the
//! process publishes it. This crate only knows how to CALL such an export, and is told which
//! module and symbol to look for by its caller (see [`UnionExport`]).
// PARITY: this crate transcribes MinHook's C ABI, so its names, casing and the items it
// declares-but-does-not-call are the upstream header's shape rather than this repo's.
// A per-item allow would mean annotating essentially every line of a binding file.
#![allow(dead_code, non_snake_case, non_camel_case_types, missing_docs)]

use std::ffi::{CStr, c_void};
use std::ptr::null_mut;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

// ============================================================================
// LOGGING SEAM. The union and the collision registry both have things worth saying, but a log
// SINK is product-specific -- a path, a format, a lifetime this crate knows nothing about. So
// they call through a function pointer the product installs at startup via `set_hook_logger`.
// The default is a no-op (no logger installed). Install it before the first hook registration,
// or the earliest and most interesting lines are the ones you lose. A DLL that only uses the raw
// `MH_*` externs never touches the union and never needs a logger; the seam stays inert for it.
// ============================================================================
/// Signature of a logging sink: the union/registry code hands it `format_args!` output.
pub type HookLogFn = fn(std::fmt::Arguments<'_>);
static HOOK_LOGGER: AtomicUsize = AtomicUsize::new(0);

/// Install the sink for union/registry log lines. Call once, early -- before any hook
/// registration -- since lines emitted before the sink exists are dropped, not buffered.
pub fn set_hook_logger(logger: HookLogFn) {
    HOOK_LOGGER.store(logger as usize, Ordering::Release);
}

fn hook_log(args: std::fmt::Arguments<'_>) {
    let raw = HOOK_LOGGER.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: `raw` is only ever a `HookLogFn` stored by `set_hook_logger`.
        let logger: HookLogFn = unsafe { std::mem::transmute::<usize, HookLogFn>(raw) };
        logger(args);
    }
}

// ============================================================================
// HOOK UNION. MinHook binds ONE detour per address, so two features hooking the same game
// function silently drop one: the second `MH_CreateHook` returns `MH_ERROR_ALREADY_CREATED`,
// the loser reports installed and never runs. This unions them: the FIRST feature to hook an
// address installs a single dispatcher detour (from a fixed pool, so no runtime codegen) that
// owns the real trampoline; every feature's handler is chained by pointing its existing `orig`
// slot at the NEXT handler, with the LAST handler's `orig` = the real game trampoline.
// A handler that calls its orig now calls the next handler in the chain (or the game), so
// existing handlers work unchanged and NO handler is ever silently dropped.
//
// Constraint: the shared signature is `extern "system" fn(usize,usize,usize,usize)->usize`
// -- correct for integer/pointer target functions of up to four arguments. A handler using
// fewer args just ignores the extras; unused register args are harmless. NOT usable for
// float-argument targets, or for anything whose arguments spill onto the stack.
// ============================================================================
pub type UnionFn = unsafe extern "system" fn(usize, usize, usize, usize) -> usize;
// One slot per unique target address; handlers chained onto the same address share a slot.
// Sized for this DLL's own union targets PLUS a companion DLL's, because a companion that routes
// its registrations here (see the cross-DLL section below) spends slots out of THIS pool -- the
// whole point being that a single MinHook instance owns every shared address rather than two
// instances corrupting each other's trampolines.
const MAX_UNION_SLOTS: usize = 96;

struct UnionEntry {
    target: usize,
    trampoline: usize,
    /// handler fn ptr + its caller-owned `orig` slot, in chain order.
    handlers: Vec<(usize, &'static AtomicUsize)>,
}
static UNIONS: Mutex<Vec<UnionEntry>> = Mutex::new(Vec::new());
/// Lock-free head-handler per slot, read on every dispatch (no mutex in the hot path).
#[allow(clippy::declare_interior_mutable_const)]
static UNION_HEADS: [AtomicUsize; MAX_UNION_SLOTS] =
    [const { AtomicUsize::new(0) }; MAX_UNION_SLOTS];

unsafe extern "system" fn union_dispatch<const N: usize>(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
) -> usize {
    let head = UNION_HEADS[N].load(Ordering::Acquire);
    if head == 0 {
        return 0;
    }
    let f: UnionFn = unsafe { std::mem::transmute::<usize, UnionFn>(head) };
    unsafe { f(a, b, c, d) }
}

macro_rules! union_dispatchers {
    ($($n:literal)*) => { [ $( union_dispatch::<$n> as UnionFn ),* ] };
}
static DISPATCHERS: [UnionFn; MAX_UNION_SLOTS] = union_dispatchers!(
    0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23
    24 25 26 27 28 29 30 31 32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47
    48 49 50 51 52 53 54 55 56 57 58 59 60 61 62 63 64 65 66 67 68 69 70 71
    72 73 74 75 76 77 78 79 80 81 82 83 84 85 86 87 88 89 90 91 92 93 94 95
);

/// Register `handler` on `target`, chaining through `orig_slot`. First registrant installs
/// the dispatcher + owns the trampoline; later ones append and no handler is ever dropped.
///
/// # Safety
/// `handler` must be a valid `UnionFn` matching the target's ABI; `orig_slot` must be the
/// static the handler reads to call its original.
pub unsafe fn register_union_hook(
    target: usize,
    handler: UnionFn,
    orig_slot: &'static AtomicUsize,
) -> Result<(), MH_STATUS> {
    match unsafe { MH_Initialize() } {
        MH_STATUS::MH_OK | MH_STATUS::MH_ERROR_ALREADY_INITIALIZED => {}
        s => return Err(s),
    }
    let handler_addr = handler as usize;
    let mut unions = UNIONS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = unions.iter_mut().find(|e| e.target == target) {
        // already skip a duplicate registration of the SAME handler (idempotent retries).
        if entry.handlers.iter().any(|(h, _)| *h == handler_addr) {
            return Ok(());
        }
        if let Some((_, prev_orig)) = entry.handlers.last() {
            prev_orig.store(handler_addr, Ordering::Release); // prev -> new
        }
        orig_slot.store(entry.trampoline, Ordering::Release); // new -> game orig
        entry.handlers.push((handler_addr, orig_slot));
        hook_log(format_args!(
            "HOOK UNION: game addr 0x{target:x} now chains {} handlers (added {})",
            entry.handlers.len(),
            as_dll_off(handler_addr)
        ));
        return Ok(());
    }
    let slot = unions.len();
    if slot >= MAX_UNION_SLOTS {
        return Err(MH_STATUS::MH_ERROR_MEMORY_ALLOC);
    }
    let mut trampoline = null_mut();
    unsafe {
        MH_CreateHook(
            target as *mut c_void,
            DISPATCHERS[slot] as *mut c_void,
            &mut trampoline,
        )
    }
    .ok()?;
    // ARM THE SLOT BEFORE ENABLING THE DETOUR. These two stores used to happen AFTER
    // `MH_EnableHook`, leaving a window in which the dispatcher was live but its head was still 0
    // -- and `union_dispatch` returns 0 for a null head WITHOUT calling the game. On a rarely-hit
    // target that window is invisible; on a hot one called throughout boot, a single unlucky call
    // hands the engine a zero where it expected a real return value. The dispatcher is
    // unreachable until the detour is enabled, so publishing the head first is free.
    UNION_HEADS[slot].store(handler_addr, Ordering::Release);
    orig_slot.store(trampoline as usize, Ordering::Release); // sole handler -> game orig
    match unsafe { MH_EnableHook(target as *mut c_void) } {
        MH_STATUS::MH_OK | MH_STATUS::MH_ERROR_ENABLED => {}
        s => {
            // Nothing is patched, so leave no armed head behind for a later slot reuse to inherit.
            UNION_HEADS[slot].store(0, Ordering::Release);
            orig_slot.store(0, Ordering::Release);
            return Err(s);
        }
    }
    unions.push(UnionEntry {
        target,
        trampoline: trampoline as usize,
        handlers: vec![(handler_addr, orig_slot)],
    });
    Ok(())
}

// ============================================================================
// CROSS-DLL UNION -- THE COMPANION SIDE.
//
// `register_union_hook` above unions handlers inside ONE DLL, and cannot do more than that:
// its registry, its dispatcher pool and its MinHook instance are all statics, and a statically
// linked crate's statics are PER DLL. Two cdylibs that both link this crate therefore own two
// INDEPENDENT MinHook instances. If both detour one prologue, the second `MH_CreateHook` gets
// `MH_ERROR_ALREADY_CREATED`: the loser reports installed, never runs, and every feature behind
// it looks unimplemented -- nothing crashes and nothing logs an error.
//
// That is measured, not hypothetical. In the Elden Ring workspace this crate came from, two
// shipped DLLs detoured the same prologue; loaded together, the loser reported its hook installed
// and counted ZERO invocations for an entire session, while the identical build loaded alone
// counted over a hundred. No crash, no error, no log line -- the feature simply did nothing.
//
// The fix is for every OTHER DLL to register through the product DLL's union export rather than
// through its own instance -- one MinHook instance owns the prologue and both handlers CHAIN.
// [`register_shared_hook`] is that call: it uses the product's union when the product is in the
// process and this DLL's own union when it is not, so a standalone run of a companion behaves
// exactly as before. WHICH module and WHICH symbol is the caller's to name ([`UnionExport`]);
// this crate is substrate and holds no product identity of its own.
// ============================================================================

/// C-ABI shape of the product DLL's union-register export: `(target, handler, *mut orig_slot)
/// -> 0 ok | -1 null slot | positive `MH_STATUS` on MinHook failure`.
pub type UnionRegisterFn = unsafe extern "system" fn(usize, UnionFn, *mut usize) -> i32;

/// Which module publishes the cross-DLL union register, and under what symbol.
///
/// Supplied by the caller rather than hard-coded here. This crate is substrate: the
/// `#[unsafe(no_mangle)]` export lives in the product DLL so that exactly one module in the
/// process owns it, and a binding crate has no business asserting what that module is called.
/// Both fields are read exactly as `GetModuleHandleA` / `GetProcAddress` read them, so
/// `dll_name` is the module's BASE name (it need not be on any particular path) and
/// `export_name` is the undecorated symbol.
///
/// TREAT `export_name` AS AN ABI, NOT AS BRANDING. Companion DLLs resolve it out of the product
/// BY STRING at runtime, and users install these DLLs one at a time from separate releases -- so
/// an already-downloaded companion has to keep finding the export next to a freshly built
/// product. Renaming it silently drops that companion back onto its own MinHook instance, which
/// is the trampoline collision this whole API exists to remove, and no build gate can see it
/// happen. (That lesson was paid for in the Elden Ring workspace; inheriting it is free.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnionExport {
    /// Base name of the module that exports the union register, NUL-terminated.
    pub dll_name: &'static CStr,
    /// The exported symbol's undecorated name, NUL-terminated.
    pub export_name: &'static CStr,
}

/// Which MinHook instance a [`register_shared_hook`] call ended up on. Worth logging: it is the
/// difference between "chained onto the product's detour" and "installed a second instance that
/// may be about to lose a trampoline race".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookRoute {
    /// Chained into the product DLL's single union -- the product is co-loaded.
    ProductUnion,
    /// This DLL's own union -- the product is absent, or this IS the product.
    LocalUnion,
}

/// Default poll budget for [`register_shared_hook`]: ~1s at 25ms.
///
/// A budget is needed rather than a single probe because nothing guarantees the loader has mapped
/// the product DLL by the time a companion's install thread runs. A companion that probed too
/// early would find no module, take the local union, and recreate the exact collision this API
/// exists to remove. Co-loaded DLLs are mapped within a few milliseconds of each other, so this
/// budget is orders of magnitude past the real race; the fallback is correct behaviour rather
/// than a failure, so overshooting costs nothing but a late arm.
#[cfg(windows)]
const PRODUCT_RESOLVE_TRIES: u32 = 40;
#[cfg(windows)]
const PRODUCT_RESOLVE_SLEEP_MS: u32 = 25;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleA(name: *const std::ffi::c_char) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const std::ffi::c_char) -> *mut c_void;
    fn Sleep(ms: u32);
}

/// Resolve the product DLL's union-register export, polling `tries` times at `sleep_ms`
/// intervals. `None` means the product is not in this process (a standalone companion run) or
/// this DLL *is* the product -- in both cases the caller owns the address itself.
///
/// Pass `tries = 1, sleep_ms = 0` for a non-blocking probe.
#[cfg(windows)]
pub fn resolve_product_union_register(
    export: UnionExport,
    tries: u32,
    sleep_ms: u32,
) -> Option<UnionRegisterFn> {
    for attempt in 0..tries.max(1) {
        let hmod = unsafe { GetModuleHandleA(export.dll_name.as_ptr()) };
        // Resolving our OWN export would route right back into the local union through a C-ABI
        // round trip. Same outcome, so this is a clarity guard rather than a correctness one --
        // but it also means the product can call `register_shared_hook` without special-casing.
        if !hmod.is_null() && hmod as usize != dll_base() {
            let proc = unsafe { GetProcAddress(hmod, export.export_name.as_ptr()) };
            if !proc.is_null() {
                // SAFETY: the export's C-ABI shape is fixed by the product DLL, and both images
                // stay mapped for the process lifetime, so the pointer stays valid.
                return Some(unsafe { std::mem::transmute::<*mut c_void, UnionRegisterFn>(proc) });
            }
        }
        if attempt + 1 < tries.max(1) && sleep_ms > 0 {
            unsafe { Sleep(sleep_ms) };
        }
    }
    None
}

/// Register `handler` on `target` through whichever union owns the process's MinHook instance for
/// it: the product DLL's when the product is co-loaded, this DLL's own otherwise.
///
/// Use this -- never a bare [`MhHook`] -- for any prologue a SECOND DLL in this process might
/// also detour.
///
/// # Safety
/// `handler` must be a valid [`UnionFn`] matching `target`'s ABI (<=4 integer/pointer args), and
/// `orig_slot` must be the `'static` cell that handler reads to call its original. Note that the
/// value stored there may be the NEXT handler in the chain rather than the game trampoline, so the
/// handler must call it through the 4-argument [`UnionFn`] signature, not the game's narrower one.
#[cfg(windows)]
pub unsafe fn register_shared_hook(
    export: UnionExport,
    target: usize,
    handler: UnionFn,
    orig_slot: &'static AtomicUsize,
) -> Result<HookRoute, MH_STATUS> {
    unsafe {
        register_shared_hook_with_budget(
            export,
            target,
            handler,
            orig_slot,
            PRODUCT_RESOLVE_TRIES,
            PRODUCT_RESOLVE_SLEEP_MS,
        )
    }
}

/// [`register_shared_hook`] with an explicit resolve budget.
///
/// Pass `tries = 1, sleep_ms = 0` when the caller is driven by a GAME FRAME rather than by its own
/// install thread. The default budget exists because a companion's install thread can outrun the
/// loader's `LoadLibrary` of the product; a frame-driven caller cannot -- by the time the game is
/// ticking, every co-loaded DLL has long since been mapped -- so one probe is already the right
/// answer there, and the polling budget would only be a stall on the game thread in the case where
/// the product is genuinely absent.
///
/// # Safety
/// Same contract as [`register_shared_hook`].
#[cfg(windows)]
pub unsafe fn register_shared_hook_with_budget(
    export: UnionExport,
    target: usize,
    handler: UnionFn,
    orig_slot: &'static AtomicUsize,
    tries: u32,
    sleep_ms: u32,
) -> Result<HookRoute, MH_STATUS> {
    if let Some(register) = resolve_product_union_register(export, tries, sleep_ms) {
        // AtomicUsize is a repr(transparent) usize, so handing the product a `*mut usize` into our
        // own static is sound; our image outlives every dispatch.
        let slot_ptr = orig_slot.as_ptr();
        return match unsafe { register(target, handler, slot_ptr) } {
            0 => Ok(HookRoute::ProductUnion),
            // -1 is the export's null-slot rejection, which cannot happen here (the pointer comes
            // from a live static) -- reported as UNKNOWN rather than silently mapped to a status.
            code if code < 0 => Err(MH_STATUS::MH_UNKNOWN),
            code => Err(mh_status_from_i32(code)),
        };
    }
    unsafe { register_union_hook(target, handler, orig_slot) }.map(|()| HookRoute::LocalUnion)
}

/// Reconstruct an [`MH_STATUS`] from the `i32` the cross-DLL export returns.
fn mh_status_from_i32(code: i32) -> MH_STATUS {
    match code {
        0 => MH_STATUS::MH_OK,
        1 => MH_STATUS::MH_ERROR_ALREADY_INITIALIZED,
        2 => MH_STATUS::MH_ERROR_NOT_INITIALIZED,
        3 => MH_STATUS::MH_ERROR_ALREADY_CREATED,
        4 => MH_STATUS::MH_ERROR_NOT_CREATED,
        5 => MH_STATUS::MH_ERROR_ENABLED,
        6 => MH_STATUS::MH_ERROR_DISABLED,
        7 => MH_STATUS::MH_ERROR_NOT_EXECUTABLE,
        8 => MH_STATUS::MH_ERROR_UNSUPPORTED_FUNCTION,
        9 => MH_STATUS::MH_ERROR_MEMORY_ALLOC,
        10 => MH_STATUS::MH_ERROR_MEMORY_PROTECT,
        11 => MH_STATUS::MH_ERROR_MODULE_NOT_FOUND,
        12 => MH_STATUS::MH_ERROR_FUNCTION_NOT_FOUND,
        _ => MH_STATUS::MH_UNKNOWN,
    }
}

/// Central hook registry. Every MinHook detour creation records its TARGET game address here.
/// MinHook binds only ONE detour per address: when a second feature hooks an address that is
/// already claimed, MH_CreateHook returns MH_ERROR_ALREADY_CREATED and the loser's handler NEVER
/// runs. Which detour wins depends on thread install order, so on native Windows it is a
/// non-deterministic race (Wine's scheduler happens to be consistent, which is why the same build
/// can look fine under Proton and flake on Windows). This registry turns that invisible race into
/// an explicit LOGGED COLLISION at install time, naming the target address and both detours -- so
/// a contested address is visible immediately instead of surfacing as a flaky runtime bug.
static HOOK_REGISTRY: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

/// Our DLL's load base, so detours can be reported as `dll+0xNNN` (identifiable against the map/disasm)
/// instead of an absolute pointer that shifts every launch.
fn dll_base() -> usize {
    use std::sync::OnceLock;
    static BASE: OnceLock<usize> = OnceLock::new();
    *BASE.get_or_init(|| {
        unsafe extern "system" {
            fn GetModuleHandleExW(flags: u32, addr: *const c_void, module: *mut *mut c_void)
            -> i32;
        }
        const FROM_ADDRESS: u32 = 0x4;
        const UNCHANGED_REFCOUNT: u32 = 0x2;
        let mut h: *mut c_void = null_mut();
        let anchor = dll_base as *const c_void; // any address inside our DLL
        if unsafe { GetModuleHandleExW(FROM_ADDRESS | UNCHANGED_REFCOUNT, anchor, &mut h) } != 0 {
            h as usize
        } else {
            0
        }
    })
}

fn as_dll_off(p: usize) -> String {
    let b = dll_base();
    if b != 0 && p >= b {
        format!("dll+0x{:x}", p - b)
    } else {
        format!("0x{p:x}")
    }
}

fn registry_record(target: usize, detour: usize, create_status: MH_STATUS) {
    if let Ok(mut reg) = HOOK_REGISTRY.lock() {
        let prior: Vec<String> = reg
            .iter()
            .filter(|(t, _)| *t == target)
            .map(|(_, d)| as_dll_off(*d))
            .collect();
        reg.push((target, detour));
        if !prior.is_empty() || create_status == MH_STATUS::MH_ERROR_ALREADY_CREATED {
            hook_log(format_args!(
                "HOOK REGISTRY COLLISION: game addr 0x{target:x} already hooked by detour(s) [{}], NOW ALSO detour {} (MH_CreateHook={create_status:?}) -- only ONE binds, the loser's handler never fires (silent native-Windows race source)",
                prior.join(", "),
                as_dll_off(detour)
            ));
        }
    }
}

#[allow(non_camel_case_types)]
#[must_use]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MH_STATUS {
    MH_UNKNOWN = -1,
    MH_OK = 0,
    MH_ERROR_ALREADY_INITIALIZED,
    MH_ERROR_NOT_INITIALIZED,
    MH_ERROR_ALREADY_CREATED,
    MH_ERROR_NOT_CREATED,
    MH_ERROR_ENABLED,
    MH_ERROR_DISABLED,
    MH_ERROR_NOT_EXECUTABLE,
    MH_ERROR_UNSUPPORTED_FUNCTION,
    MH_ERROR_MEMORY_ALLOC,
    MH_ERROR_MEMORY_PROTECT,
    MH_ERROR_MODULE_NOT_FOUND,
    MH_ERROR_FUNCTION_NOT_FOUND,
}

unsafe extern "system" {
    pub fn MH_Initialize() -> MH_STATUS;
    pub fn MH_Uninitialize() -> MH_STATUS;
    pub fn MH_CreateHook(
        pTarget: *mut c_void,
        pDetour: *mut c_void,
        ppOriginal: *mut *mut c_void,
    ) -> MH_STATUS;
    pub fn MH_EnableHook(pTarget: *mut c_void) -> MH_STATUS;
    pub fn MH_QueueEnableHook(pTarget: *mut c_void) -> MH_STATUS;
    pub fn MH_DisableHook(pTarget: *mut c_void) -> MH_STATUS;
    pub fn MH_QueueDisableHook(pTarget: *mut c_void) -> MH_STATUS;
    pub fn MH_ApplyQueued() -> MH_STATUS;
}

impl MH_STATUS {
    pub fn ok_context(self, _context: &str) -> Result<(), MH_STATUS> {
        self.ok()
    }

    pub fn ok(self) -> Result<(), MH_STATUS> {
        if self == MH_STATUS::MH_OK {
            Ok(())
        } else {
            Err(self)
        }
    }
}

/// Original address, hook function address, and trampoline for a given hook.
pub struct MhHook {
    addr: *mut c_void,
    hook_impl: *mut c_void,
    trampoline: *mut c_void,
}

impl MhHook {
    /// # Safety
    ///
    /// Installs native code detours; caller must ensure ABI and lifetime are valid.
    pub unsafe fn new(addr: *mut c_void, hook_impl: *mut c_void) -> Result<Self, MH_STATUS> {
        let mut trampoline = null_mut();
        let status = unsafe { MH_CreateHook(addr, hook_impl, &mut trampoline) };
        registry_record(addr as usize, hook_impl as usize, status);
        status.ok_context("MH_CreateHook")?;

        Ok(Self {
            addr,
            hook_impl,
            trampoline,
        })
    }

    pub fn trampoline(&self) -> *mut c_void {
        self.trampoline
    }

    /// # Safety
    ///
    /// Enables a native detour through MinHook's queued API.
    pub unsafe fn queue_enable(&self) -> Result<(), MH_STATUS> {
        unsafe { MH_QueueEnableHook(self.addr) }.ok_context("MH_QueueEnableHook")
    }

    /// # Safety
    ///
    /// Disables a native detour through MinHook's queued API.
    pub unsafe fn queue_disable(&self) -> Result<(), MH_STATUS> {
        unsafe { MH_QueueDisableHook(self.addr) }.ok_context("MH_QueueDisableHook")
    }
}

// ============================================================================
// RAW CODE-PATCH PRIMITIVES. They live in this crate because they are the same "reach into the
// game image and rewrite bytes" capability MinHook itself provides, so anything that wants one
// already depends on this crate and needs no extra seam to reach it.
//
// Every one of them takes the address, the expected byte and the stub as PARAMETERS. That is
// what keeps this crate free of game knowledge: it knows how to flip page protection and flush an
// icache, and nothing whatsoever about what it is patching.
//
// The `windows` crate is NOT pulled in for these -- this crate has zero `[dependencies]` and
// keeps it that way, following the raw-extern pattern already used above for `GetModuleHandleExW`
// and the `MH_*` family.
// ============================================================================

/// Init value for the `VirtualProtect` out-params; overwritten by the call.
const PAGE_PROTECT_UNSET: u32 = 0;
/// `PAGE_EXECUTE_READWRITE` (winnt.h), the protection a code patch needs.
const PAGE_EXECUTE_READWRITE: u32 = 0x40;
/// Win32 `BOOL` false; `VirtualProtect` returns zero on failure.
const WIN32_FALSE: i32 = 0;
/// `-1` cast to a handle: the current-process pseudo-handle `FlushInstructionCache` accepts
/// without an `OpenProcess` round-trip.
const CURRENT_PROCESS_PSEUDO_HANDLE: isize = -1;
/// Both primitives write exactly the 3 bytes of a `[u8; 3]` stub.
const STUB_LEN: usize = 3;
const BYTE_STEP: usize = 1;
const BYTE_START: usize = 0;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn VirtualProtect(
        addr: *mut c_void,
        size: usize,
        new_protect: u32,
        old_protect: *mut u32,
    ) -> i32;
    /// Flush the CPU instruction cache after patching executable code so other threads see the
    /// new bytes (current-process pseudo-handle -1).
    fn FlushInstructionCache(process: isize, base: *const c_void, size: usize) -> i32;
}

/// Bytes touched by [`write_code_byte`]. Named so the protection, the store, and the cache flush
/// visibly agree on one length.
const ONE_CODE_BYTE: usize = 1;

/// The page operations a code-byte write performs, behind a seam. [`Win32CodePage`] is the only
/// production implementation; the seam exists because the two ways this primitive can be wrong are
/// both invisible to a compile check -- a page left `PAGE_EXECUTE_READWRITE` after the write, and a
/// refused protection change that stores the byte anyway -- so the SEQUENCE is asserted on the host
/// instead of only in a game.
trait CodePageOps {
    /// `VirtualProtect`: returns whether the protection change was allowed, writing the previous
    /// protection into `old_protect`.
    fn protect(&mut self, addr: usize, len: usize, new_protect: u32, old_protect: &mut u32)
    -> bool;

    /// # Safety
    ///
    /// `addr` must be writable for the duration of the call.
    unsafe fn store(&mut self, addr: usize, value: u8);

    /// Flush the instruction cache so threads already inside this code see the new byte.
    fn flush(&mut self, addr: usize, len: usize);
}

/// Shared body of [`write_code_byte`]: unlock, store, relock to the PREVIOUS protection, flush.
///
/// Returns whether the protection change was allowed. A refused change returns before the store,
/// so nothing is written and no protection is left changed.
///
/// # Safety
///
/// With [`Win32CodePage`], `address` must be a byte of currently-mapped code in this process that
/// is safe to overwrite; the store is an unsynchronised write into executable memory.
unsafe fn write_code_byte_with<O: CodePageOps>(ops: &mut O, address: usize, value: u8) -> bool {
    let mut old_protect = PAGE_PROTECT_UNSET;
    if !ops.protect(
        address,
        ONE_CODE_BYTE,
        PAGE_EXECUTE_READWRITE,
        &mut old_protect,
    ) {
        hook_log(format_args!(
            "write_code_byte: VirtualProtect failed at 0x{address:x}"
        ));
        return false;
    }
    unsafe { ops.store(address, value) };
    let mut restored = PAGE_PROTECT_UNSET;
    ops.protect(address, ONE_CODE_BYTE, old_protect, &mut restored);
    ops.flush(address, ONE_CODE_BYTE);
    true
}

/// The production [`CodePageOps`]: Win32 `VirtualProtect` + `FlushInstructionCache` against the
/// current process.
#[cfg(windows)]
struct Win32CodePage;

#[cfg(windows)]
impl CodePageOps for Win32CodePage {
    fn protect(
        &mut self,
        addr: usize,
        len: usize,
        new_protect: u32,
        old_protect: &mut u32,
    ) -> bool {
        let allowed = unsafe { VirtualProtect(addr as *mut c_void, len, new_protect, old_protect) };
        allowed != WIN32_FALSE
    }

    unsafe fn store(&mut self, addr: usize, value: u8) {
        unsafe { *(addr as *mut u8) = value };
    }

    fn flush(&mut self, addr: usize, len: usize) {
        unsafe { FlushInstructionCache(CURRENT_PROCESS_PSEUDO_HANDLE, addr as *const c_void, len) };
    }
}

/// Write a single byte of executable code at `address`, with the protection dance the write needs:
/// `PAGE_EXECUTE_READWRITE`, the store, the original protection back, then an instruction-cache
/// flush so threads already inside that code see the new byte.
///
/// Returns whether `VirtualProtect` allowed the write. It deliberately does NOT report whether the
/// byte landed: a caller patching game code should read it back, because another mod can own the
/// same address, and a successful `VirtualProtect` says nothing about that.
///
/// Unlike [`patch_3byte_stub`] and [`apply_xor_ret_stub`], this does NOT validate the byte it
/// overwrites. Those two abort when the existing byte is not the expected one, which is what stops
/// a version-drifted RVA from being patched mid-instruction; a caller of this primitive gets no
/// such guard and must check the address itself.
///
/// # Safety
///
/// `address` must be a byte of currently-mapped code in this process that is safe to overwrite.
/// The store is unsynchronised: it is a single byte, so it cannot tear, but a thread may execute
/// the patched instruction at any point during the call.
#[cfg(windows)]
pub unsafe fn write_code_byte(address: usize, value: u8) -> bool {
    unsafe { write_code_byte_with(&mut Win32CodePage, address, value) }
}

/// Write a self-contained 3-byte stub over the function body at `base+rva`, after validating that
/// the byte already there is `expected_first`. RWX via VirtualProtect, write, restore the previous
/// protection, icache flush. Returns true on success.
///
/// The `expected_first` check is the guard that stops a version-drifted RVA from being patched
/// mid-instruction. Both it and `stub` are the caller's facts: this function holds no knowledge of
/// what it is patching, and `label` is only used to name the target in a log line.
#[cfg(windows)]
pub fn patch_3byte_stub(
    base: usize,
    rva: usize,
    expected_first: u8,
    stub: [u8; STUB_LEN],
    label: &str,
) -> bool {
    let target = (base + rva) as *mut u8;
    let existing = unsafe { *target };
    if existing != expected_first {
        hook_log(format_args!(
            "{label}: ABORT -- byte at 0x{:x} is 0x{existing:x}, expected 0x{expected_first:x}",
            base + rva
        ));
        return false;
    }
    let mut old_protect = PAGE_PROTECT_UNSET;
    let protect_ok = unsafe {
        VirtualProtect(
            target as *mut c_void,
            STUB_LEN,
            PAGE_EXECUTE_READWRITE,
            &mut old_protect,
        )
    };
    if protect_ok == WIN32_FALSE {
        hook_log(format_args!("{label}: VirtualProtect failed"));
        return false;
    }
    let mut i = BYTE_START;
    while i < STUB_LEN {
        unsafe { *target.add(i) = stub[i] };
        i += BYTE_STEP;
    }
    let mut restored = PAGE_PROTECT_UNSET;
    unsafe { VirtualProtect(target as *mut c_void, STUB_LEN, old_protect, &mut restored) };
    unsafe {
        FlushInstructionCache(
            CURRENT_PROCESS_PSEUDO_HANDLE,
            target as *const c_void,
            STUB_LEN,
        )
    };
    true
}

/// [`patch_3byte_stub`] plus a success log line, for the common case of neutering a function by
/// replacing its body with `xor eax,eax; ret` (return 0).
///
/// In er-mods-rs this was a second, fully duplicated copy of the body above, kept that way only so
/// its log strings -- which carried the product feature's name as a prefix -- stayed byte-identical
/// to what the in-product original emitted. There is no such log text to preserve here, and
/// carrying a product's feature name into a substrate crate would make every future caller's
/// diagnostics lie, so the prefix is gone and with it the reason the duplication existed.
///
/// `expected_first` and `stub` stay parameters: which byte a target's prologue starts with, and
/// which three bytes encode the stub for it, are the caller's facts and never this crate's.
#[cfg(windows)]
pub fn apply_xor_ret_stub(
    base: usize,
    rva: usize,
    expected_first: u8,
    stub: [u8; STUB_LEN],
    label: &str,
) {
    if patch_3byte_stub(base, rva, expected_first, stub, label) {
        hook_log(format_args!(
            "{label}: patched 0x{:x} -> xor eax,eax;ret (function now returns 0)",
            base + rva
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page operation, recorded rather than performed.
    #[derive(Debug, PartialEq, Eq)]
    enum Op {
        Protect {
            addr: usize,
            len: usize,
            new_protect: u32,
        },
        Store {
            addr: usize,
            value: u8,
        },
        Flush {
            addr: usize,
            len: usize,
        },
    }

    /// Stands in for a real code page. `original_protect` is what it reports as the page's previous
    /// protection, so a test can assert that exact value is handed back on the second call.
    struct FakePage {
        ops: Vec<Op>,
        original_protect: u32,
        protect_allowed: bool,
    }

    impl FakePage {
        fn allowing(original_protect: u32) -> Self {
            Self {
                ops: Vec::new(),
                original_protect,
                protect_allowed: true,
            }
        }

        fn refusing() -> Self {
            Self {
                ops: Vec::new(),
                original_protect: 0,
                protect_allowed: false,
            }
        }
    }

    impl CodePageOps for FakePage {
        fn protect(
            &mut self,
            addr: usize,
            len: usize,
            new_protect: u32,
            old_protect: &mut u32,
        ) -> bool {
            self.ops.push(Op::Protect {
                addr,
                len,
                new_protect,
            });
            if !self.protect_allowed {
                return false;
            }
            *old_protect = self.original_protect;
            true
        }

        unsafe fn store(&mut self, addr: usize, value: u8) {
            self.ops.push(Op::Store { addr, value });
        }

        fn flush(&mut self, addr: usize, len: usize) {
            self.ops.push(Op::Flush { addr, len });
        }
    }

    const PAGE_EXECUTE_READ: u32 = 0x20;
    const TEST_ADDR: usize = 0x1234_5678;
    const TEST_BYTE: u8 = 0xcc;

    /// The whole sequence, in order: unlock to RWX, store, relock, flush.
    #[test]
    fn writes_between_unlocking_and_relocking_then_flushes() {
        let mut page = FakePage::allowing(PAGE_EXECUTE_READ);

        let wrote = unsafe { write_code_byte_with(&mut page, TEST_ADDR, TEST_BYTE) };

        assert!(wrote);
        assert_eq!(
            page.ops,
            vec![
                Op::Protect {
                    addr: TEST_ADDR,
                    len: ONE_CODE_BYTE,
                    new_protect: PAGE_EXECUTE_READWRITE,
                },
                Op::Store {
                    addr: TEST_ADDR,
                    value: TEST_BYTE,
                },
                Op::Protect {
                    addr: TEST_ADDR,
                    len: ONE_CODE_BYTE,
                    new_protect: PAGE_EXECUTE_READ,
                },
                Op::Flush {
                    addr: TEST_ADDR,
                    len: ONE_CODE_BYTE,
                },
            ]
        );
    }

    /// The hazard: a patched page left writable-and-executable for the rest of the process. The
    /// relock must name the protection the page actually had, not a guess and not RWX.
    #[test]
    fn does_not_leave_the_page_executable_and_writable() {
        for original in [PAGE_EXECUTE_READ, 0x02, 0x04, 0x80] {
            let mut page = FakePage::allowing(original);

            unsafe { write_code_byte_with(&mut page, TEST_ADDR, TEST_BYTE) };

            let last_protect = page
                .ops
                .iter()
                .filter_map(|op| match op {
                    Op::Protect { new_protect, .. } => Some(*new_protect),
                    _ => None,
                })
                .next_back()
                .expect("a protection change");
            assert_eq!(
                last_protect, original,
                "page relocked to the wrong protection"
            );
            assert_ne!(last_protect, PAGE_EXECUTE_READWRITE, "page left RWX");
        }
    }

    /// A refused protection change must abort before the store. Writing anyway would fault, or
    /// worse, succeed on a page that was already writable and hide the refusal.
    #[test]
    fn refused_protection_change_writes_nothing() {
        let mut page = FakePage::refusing();

        let wrote = unsafe { write_code_byte_with(&mut page, TEST_ADDR, TEST_BYTE) };

        assert!(!wrote);
        assert_eq!(
            page.ops,
            vec![Op::Protect {
                addr: TEST_ADDR,
                len: ONE_CODE_BYTE,
                new_protect: PAGE_EXECUTE_READWRITE,
            }],
            "nothing may follow a refused VirtualProtect"
        );
    }

    /// What the `ONE_CODE_BYTE` doc claims: the protection, the store and the flush cover the same
    /// one byte at the same address. A length that disagreed would unlock or flush a range the
    /// caller never asked about.
    #[test]
    fn every_page_operation_covers_the_same_single_byte() {
        let mut page = FakePage::allowing(PAGE_EXECUTE_READ);

        unsafe { write_code_byte_with(&mut page, TEST_ADDR, TEST_BYTE) };

        assert_eq!(ONE_CODE_BYTE, size_of::<u8>());
        for op in &page.ops {
            let (addr, len) = match op {
                Op::Protect { addr, len, .. } | Op::Flush { addr, len } => (*addr, *len),
                Op::Store { addr, .. } => (*addr, ONE_CODE_BYTE),
            };
            assert_eq!(addr, TEST_ADDR, "{op:?} touched a different address");
            assert_eq!(len, ONE_CODE_BYTE, "{op:?} covered a different length");
        }
    }
}
