//! The socket layer: refuse the game's own outbound calls, and count what was refused.
//!
//! # Why this layer is load-bearing rather than decorative
//!
//! `crate::flag` settles what the game believes, and that is not the same thing as what leaves the
//! machine. `FeSubStateTitleGameServerLogin`'s work starter -- `0x1400f9820`, vtable slot 8 -- was
//! disassembled specifically to check, and it **never reads the online flag**. It asks
//! `NetSvrManager` two questions of its own and then builds a login job. Without this file, an
//! "offline" run would still have gone out and logged in.
//!
//! # Why the import table and not the exports of `ws2_32`
//!
//! Every slot patched here is a pointer in `DarkSoulsII.exe`'s own `.idata`. Three consequences,
//! all of them the reason this shape was chosen:
//!
//! 1. **No code is modified**, so Arxan's `.text` integrity checks have nothing to react to. The
//!    same argument `ds2-boot-timeline` makes for its `Sleep` counter.
//! 2. **Only this executable's calls are affected.** `steamclient64.dll` and
//!    `GameOverlayRenderer64.dll` are in this process with their own import tables; Steam's
//!    connection, the overlay and achievements go on working. This is not a firewall and does not
//!    claim to be one.
//! 3. It is reversible by a pointer write, which is what makes `enabled = false` a real switch.
//!
//! # The slots are found by asking `ws2_32`, not by trusting an ordinal table
//!
//! DARK SOULS II imports 43 functions from `WS2_32.dll` and **all but ten are imported by
//! ordinal** -- `connect` and `sendto`, the two that matter most here, among them. Hardcoding
//! "ordinal 4 is `connect`" would be exactly the kind of remembered fact this repo keeps finding
//! to be wrong.
//!
//! So nothing here parses hints or ordinals. [`install`] walks the import descriptors to find the
//! `WS2_32.dll` entry, then for each name it wants asks the already-loaded `ws2_32.dll` for that
//! function's address with `GetProcAddress` and patches whichever IAT slot currently holds that
//! pointer. The loader filled those slots from the same export table `GetProcAddress` reads, so a
//! match is proof of identity; a name that matches no slot is reported and skipped rather than
//! guessed at.
//!
//! # What is refused, and with which error
//!
//! `connect` and `sendto` to anything that is not loopback, plus the two name-resolution calls.
//! They fail with `WSAENETUNREACH` (10051) and `WSAHOST_NOT_FOUND` (11001) -- the errors a machine
//! with no route to the internet produces. That matters: the game already has handling for exactly
//! that condition (it is what raises `FeSubStateTitleOnlineCheckFailWarn` and the "could not
//! retrieve information" box), so this drives a shipped path rather than inventing a new one. An
//! invented error code, or a silent success, would not.
//!
//! `send` and `recv` are deliberately left alone. They operate on a socket that a refused `connect`
//! never gave the game, and blanket-failing them would reach loopback traffic this crate has no
//! business touching.
//!
//! # Loopback is allowed through
//!
//! `127.0.0.0/8` and `::1`. Proton, Wine and the Steam API all use local sockets, and refusing
//! those breaks the game instead of its matchmaking.
//!
//! # These detours never log from the hot path
//!
//! The loader's sink `fsync`s every line. A refusal logs only for the first few occurrences per
//! API; after that it is an atomic increment, and the totals are read back by [`counts`] from
//! whatever thread wants to report them.

use core::ffi::{CStr, c_char, c_void};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::LOG_PREFIX;

unsafe extern "system" {
    fn VirtualProtect(address: *mut c_void, size: usize, new: u32, old: *mut u32) -> i32;
    fn GetModuleHandleA(name: *const c_char) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
}

const PAGE_READWRITE: u32 = 0x04;

/// `SOCKET_ERROR`, the `int` every failing Winsock call returns.
const SOCKET_ERROR: i32 = -1;
/// `WSAENETUNREACH` -- "a socket operation was attempted to an unreachable network". The error a
/// machine with no route produces, and therefore a condition the game already handles.
const WSAENETUNREACH: i32 = 10051;
/// `WSAHOST_NOT_FOUND` -- authoritative "no such host". `getaddrinfo` returns it directly;
/// `gethostbyname` reports it through `WSASetLastError`.
const WSAHOST_NOT_FOUND: i32 = 11001;

/// `AF_INET`. `sockaddr_in` keeps its four address bytes at `+0x04`.
const AF_INET: u16 = 2;
/// `AF_INET6`. `sockaddr_in6` keeps its sixteen address bytes at `+0x08`.
const AF_INET6: u16 = 23;

/// How many refusals of one API get their own log line before the counters take over.
///
/// The loader's log sink `fsync`s per line and these run on the game's network thread. Three is
/// enough to show the first destination in the log -- which is the interesting one -- without any
/// risk of a chatty socket turning the log into the bottleneck.
const LOG_FIRST_N: u64 = 3;

/// Signature of the sink, matching [`crate::LogFn`].
type LogFn = fn(std::fmt::Arguments<'_>);
static LOGGER: AtomicUsize = AtomicUsize::new(0);

/// Point this module's logging at the loader's log file.
pub fn set_logger(logger: LogFn) {
    LOGGER.store(logger as usize, Ordering::Release);
}

fn log(args: std::fmt::Arguments<'_>) {
    let raw = LOGGER.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: `raw` is only ever a `LogFn` stored by `set_logger`.
        let logger: LogFn = unsafe { std::mem::transmute::<usize, LogFn>(raw) };
        logger(args);
    }
}

// ------------------------------------------------------------------------------------------
// The four fronted imports. Each keeps the pointer the loader resolved, so an allowed call is
// forwarded to the real function and not emulated.
// ------------------------------------------------------------------------------------------

type ConnectFn = unsafe extern "system" fn(usize, *const u8, i32) -> i32;
type SendToFn = unsafe extern "system" fn(usize, *const u8, i32, i32, *const u8, i32) -> i32;
type GetAddrInfoFn =
    unsafe extern "system" fn(*const u8, *const u8, *const c_void, *mut *mut c_void) -> i32;
type GetHostByNameFn = unsafe extern "system" fn(*const u8) -> *mut c_void;
type WsaSetLastErrorFn = unsafe extern "system" fn(i32);

static ORIGINAL_CONNECT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SENDTO: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_GETADDRINFO: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_GETHOSTBYNAME: AtomicUsize = AtomicUsize::new(0);
/// Resolved from `ws2_32` at install, NOT linked. Linking it would add `WS2_32.dll` to this DLL's
/// own import table, which is a change to the loader's shape that this feature does not need.
static WSA_SET_LAST_ERROR: AtomicUsize = AtomicUsize::new(0);

static BLOCKED_CONNECT: AtomicU64 = AtomicU64::new(0);
static BLOCKED_SENDTO: AtomicU64 = AtomicU64::new(0);
static BLOCKED_RESOLVE: AtomicU64 = AtomicU64::new(0);
static ALLOWED_LOOPBACK: AtomicU64 = AtomicU64::new(0);

/// What the socket layer refused, and what it let through.
///
/// A run with `connect=0 sendto=0 resolve=0` is the interesting one: it means the flag layer
/// stopped the game before it ever reached a socket, and this layer had nothing to do. That is a
/// measurement, not a failure, and it is the only way to find out which layer is load-bearing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockedCounts {
    /// `connect` calls refused because the destination was not loopback.
    pub connect: u64,
    /// `sendto` calls refused for the same reason.
    pub sendto: u64,
    /// `getaddrinfo` + `gethostbyname` calls refused.
    pub resolve: u64,
    /// Calls forwarded to the real function because the destination was loopback.
    pub allowed_loopback: u64,
}

/// Set once at least one import slot has been fronted. What separates "nothing was blocked"
/// from "nothing was watching", which are the same four zeros otherwise.
static ANY_FRONTED: AtomicUsize = AtomicUsize::new(0);

/// Read the counters, or `None` if no import was ever fronted. Safe to call from any thread at any
/// time -- including `DLL_PROCESS_DETACH`, which is the point of it being nothing but atomic loads.
pub fn counts() -> Option<BlockedCounts> {
    if ANY_FRONTED.load(Ordering::Acquire) == 0 {
        return None;
    }
    Some(BlockedCounts {
        connect: BLOCKED_CONNECT.load(Ordering::Relaxed),
        sendto: BLOCKED_SENDTO.load(Ordering::Relaxed),
        resolve: BLOCKED_RESOLVE.load(Ordering::Relaxed),
        allowed_loopback: ALLOWED_LOOPBACK.load(Ordering::Relaxed),
    })
}

/// Set Winsock's thread-local error code, if the setter was resolved.
fn set_last_error(code: i32) {
    let raw = WSA_SET_LAST_ERROR.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: `raw` came from `GetProcAddress(ws2_32, "WSASetLastError")`, whose signature is
        // `void WSASetLastError(int)`.
        let set: WsaSetLastErrorFn =
            unsafe { std::mem::transmute::<usize, WsaSetLastErrorFn>(raw) };
        unsafe { set(code) };
    }
}

/// A `sockaddr`, rendered for a log line, and whether it is loopback.
///
/// Returns `None` for a null pointer or a family this function does not decode. **An address it
/// cannot read is treated as NOT loopback** by the callers below -- an unrecognised destination is
/// refused, which is the direction that cannot leak a packet.
///
/// # Safety
///
/// `addr` is the pointer the game handed to `connect`/`sendto`, and `len` its stated length. Both
/// are trusted only as far as the bounds check here: nothing is read unless `len` covers it.
unsafe fn decode(addr: *const u8, len: i32) -> Option<(bool, String)> {
    if addr.is_null() || len < 2 {
        return None;
    }
    let len = len as usize;
    // SAFETY: `len >= 2` was just checked, and `sa_family` is the first two bytes of every
    // `sockaddr` variant.
    let family = unsafe { addr.cast::<u16>().read_unaligned() };
    match family {
        AF_INET if len >= 8 => {
            // `sockaddr_in`: family u16, port u16 (network order), then four address bytes.
            let mut octets = [0u8; 4];
            let mut port = [0u8; 2];
            // SAFETY: `len >= 8` covers both reads.
            unsafe {
                port.copy_from_slice(std::slice::from_raw_parts(addr.add(2), 2));
                octets.copy_from_slice(std::slice::from_raw_parts(addr.add(4), 4));
            }
            let port = u16::from_be_bytes(port);
            Some((
                octets[0] == 127,
                format!(
                    "{}.{}.{}.{}:{port}",
                    octets[0], octets[1], octets[2], octets[3]
                ),
            ))
        }
        AF_INET6 if len >= 24 => {
            // `sockaddr_in6`: family u16, port u16, flowinfo u32, then sixteen address bytes.
            let mut bytes = [0u8; 16];
            let mut port = [0u8; 2];
            // SAFETY: `len >= 24` covers both reads.
            unsafe {
                port.copy_from_slice(std::slice::from_raw_parts(addr.add(2), 2));
                bytes.copy_from_slice(std::slice::from_raw_parts(addr.add(8), 16));
            }
            let port = u16::from_be_bytes(port);
            let loopback = bytes[..15].iter().all(|b| *b == 0) && bytes[15] == 1;
            let rendered = bytes
                .chunks(2)
                .map(|pair| format!("{:02x}{:02x}", pair[0], pair[1]))
                .collect::<Vec<_>>()
                .join(":");
            Some((loopback, format!("[{rendered}]:{port}")))
        }
        _ => None,
    }
}

/// Whether to let a destination through, plus the text for a log line.
///
/// # Safety
///
/// As [`decode`].
unsafe fn allow(addr: *const u8, len: i32) -> (bool, String) {
    match unsafe { decode(addr, len) } {
        Some((loopback, rendered)) => (loopback, rendered),
        // An address this crate cannot decode is refused. The alternative -- forward what we do
        // not understand -- is the one that can put a packet on the wire by accident.
        None => (false, "<undecodable>".to_string()),
    }
}

unsafe extern "system" fn detour_connect(socket: usize, addr: *const u8, len: i32) -> i32 {
    let (allowed, rendered) = unsafe { allow(addr, len) };
    if allowed {
        ALLOWED_LOOPBACK.fetch_add(1, Ordering::Relaxed);
        let raw = ORIGINAL_CONNECT.load(Ordering::Acquire);
        if raw != 0 {
            // SAFETY: published from the IAT slot before it was overwritten.
            let original: ConnectFn = unsafe { std::mem::transmute::<usize, ConnectFn>(raw) };
            return unsafe { original(socket, addr, len) };
        }
    }
    let n = BLOCKED_CONNECT.fetch_add(1, Ordering::Relaxed) + 1;
    if n <= LOG_FIRST_N {
        log(format_args!(
            "{LOG_PREFIX} refused api=connect dest={rendered} error={WSAENETUNREACH} count={n}"
        ));
    }
    set_last_error(WSAENETUNREACH);
    SOCKET_ERROR
}

unsafe extern "system" fn detour_sendto(
    socket: usize,
    buffer: *const u8,
    length: i32,
    flags: i32,
    addr: *const u8,
    addr_len: i32,
) -> i32 {
    let (allowed, rendered) = unsafe { allow(addr, addr_len) };
    if allowed {
        ALLOWED_LOOPBACK.fetch_add(1, Ordering::Relaxed);
        let raw = ORIGINAL_SENDTO.load(Ordering::Acquire);
        if raw != 0 {
            // SAFETY: published from the IAT slot before it was overwritten.
            let original: SendToFn = unsafe { std::mem::transmute::<usize, SendToFn>(raw) };
            return unsafe { original(socket, buffer, length, flags, addr, addr_len) };
        }
    }
    let n = BLOCKED_SENDTO.fetch_add(1, Ordering::Relaxed) + 1;
    if n <= LOG_FIRST_N {
        log(format_args!(
            "{LOG_PREFIX} refused api=sendto dest={rendered} bytes={length} \
             error={WSAENETUNREACH} count={n}"
        ));
    }
    set_last_error(WSAENETUNREACH);
    SOCKET_ERROR
}

/// The host name a resolver was asked for, rendered for a log line. Truncated, because this is a
/// pointer from the game and a log line is not the place to trust a length.
///
/// # Safety
///
/// `name` is the caller's NUL-terminated string, read through the guarded reader so a bad pointer
/// yields `<unreadable>` rather than a fault.
unsafe fn host_name(name: *const u8) -> String {
    if name.is_null() {
        return "<null>".to_string();
    }
    match unsafe { ds2_game_base::mem::safe_read_cstr(name as usize, 253) } {
        Some(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        None => "<unreadable>".to_string(),
    }
}

unsafe extern "system" fn detour_getaddrinfo(
    node: *const u8,
    service: *const u8,
    hints: *const c_void,
    result: *mut *mut c_void,
) -> i32 {
    let _ = (service, hints);
    let n = BLOCKED_RESOLVE.fetch_add(1, Ordering::Relaxed) + 1;
    if n <= LOG_FIRST_N {
        log(format_args!(
            "{LOG_PREFIX} refused api=getaddrinfo host={} error={WSAHOST_NOT_FOUND} count={n}",
            unsafe { host_name(node) }
        ));
    }
    // The caller will pass `*result` to `freeaddrinfo` on some paths whatever we return, so it is
    // zeroed rather than left holding whatever the stack had.
    if !result.is_null() {
        // SAFETY: an out-parameter the caller owns and has just given us to write.
        unsafe { result.write(std::ptr::null_mut()) };
    }
    WSAHOST_NOT_FOUND
}

unsafe extern "system" fn detour_gethostbyname(name: *const u8) -> *mut c_void {
    let n = BLOCKED_RESOLVE.fetch_add(1, Ordering::Relaxed) + 1;
    if n <= LOG_FIRST_N {
        log(format_args!(
            "{LOG_PREFIX} refused api=gethostbyname host={} error={WSAHOST_NOT_FOUND} count={n}",
            unsafe { host_name(name) }
        ));
    }
    set_last_error(WSAHOST_NOT_FOUND);
    std::ptr::null_mut()
}

/// One import to front: what to ask `ws2_32` for, where to stash the original, and what to put in
/// its place.
struct Target {
    /// A `CStr` because it goes straight to `GetProcAddress`, which wants a NUL-terminated string,
    /// and because the same value has to be printable in a log line.
    name: &'static CStr,
    original: &'static AtomicUsize,
    detour: usize,
}

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Import slots now pointing at a detour.
    pub patched: usize,
    /// Imports looked for. Carried so a caller reporting "3 of 4" needs no second source for the
    /// total.
    pub attempted: usize,
    /// Whether the `WS2_32.dll` import descriptor was found at all. False means the walk failed,
    /// and `patched == 0` then means something quite different from "the names did not match".
    pub found_ws2_32: bool,
}

/// Read a NUL-terminated ASCII name out of the loaded image.
///
/// # Safety
///
/// `addr` must point into the mapped module.
unsafe fn module_string(addr: usize) -> Option<String> {
    let bytes = unsafe { ds2_game_base::mem::safe_read_cstr(addr, 64)? };
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Locate `DarkSoulsII.exe`'s `WS2_32.dll` import descriptor and return its `FirstThunk` RVA.
///
/// Walks the PE headers of the LOADED image rather than of `darksoulsii-deobf.bin`, because it is
/// the loaded IAT that has to be patched. Every read goes through the guarded readers, so a header
/// layout this does not expect returns `None` instead of faulting.
///
/// # Safety
///
/// `base` must be the live base of a mapped PE image.
unsafe fn first_thunk_rva(base: usize) -> Option<u32> {
    // `IMAGE_DOS_HEADER.e_lfanew` at `+0x3c`.
    let nt = base + unsafe { ds2_game_base::mem::safe_read_i32(base + 0x3c)? } as usize;
    // `IMAGE_NT_HEADERS64`: Signature u32, FileHeader 20 bytes, then OptionalHeader at `+0x18`.
    // The data directory sits `0x70` into a PE32+ OptionalHeader, and entry 1 is the import table.
    let import_rva = unsafe { ds2_game_base::mem::safe_read_i32(nt + 0x18 + 0x70 + 8)? } as u32;
    if import_rva == 0 {
        return None;
    }
    // `IMAGE_IMPORT_DESCRIPTOR` is 20 bytes; `Name` is at `+0x0c` and `FirstThunk` at `+0x10`.
    // The array ends at an all-zero entry, and the bound is a backstop against a corrupt table
    // rather than a real limit -- this image has 16 descriptors.
    for index in 0..64usize {
        let descriptor = base + import_rva as usize + index * 20;
        let name_rva = unsafe { ds2_game_base::mem::safe_read_i32(descriptor + 0x0c)? } as u32;
        if name_rva == 0 {
            return None;
        }
        let name = unsafe { module_string(base + name_rva as usize)? };
        if name.eq_ignore_ascii_case("ws2_32.dll") {
            return Some(unsafe { ds2_game_base::mem::safe_read_i32(descriptor + 0x10)? } as u32);
        }
    }
    None
}

/// Overwrite one IAT slot, publishing the pointer it held first.
///
/// # Safety
///
/// `slot` must be a pointer-sized cell inside the image's `.idata`.
unsafe fn patch_slot(slot: *mut usize, detour: usize, original: &AtomicUsize) -> bool {
    let mut old_protect = 0u32;
    // SAFETY: one pointer-sized slot inside the image's own import table.
    let ok = unsafe {
        VirtualProtect(
            slot.cast::<c_void>(),
            std::mem::size_of::<usize>(),
            PAGE_READWRITE,
            &raw mut old_protect,
        )
    };
    if ok == 0 {
        return false;
    }
    // SAFETY: the slot is writable now and holds the import the loader resolved.
    unsafe {
        // PUBLISHED BEFORE THE OVERWRITE. Another thread can already be inside the real function;
        // one that enters the detour and reads a zero here would refuse a loopback call it should
        // have forwarded.
        original.store(slot.read(), Ordering::Release);
        slot.write(detour);
        let mut restored = 0u32;
        VirtualProtect(
            slot.cast::<c_void>(),
            std::mem::size_of::<usize>(),
            old_protect,
            &raw mut restored,
        );
    }
    true
}

/// Front the game's outbound Winsock imports.
///
/// # Safety
///
/// `base` must be the live game module base. Writes pointers into the image's import table; call
/// from the loader's post-Arxan callback, on the entry-point thread, before the network threads
/// exist.
pub unsafe fn install(base: usize) -> Outcome {
    // Through the fn-pointer types first: `as usize` on a function ITEM is a zero-sized cast that
    // clippy rejects, and rightly.
    let connect: ConnectFn = detour_connect;
    let sendto: SendToFn = detour_sendto;
    let getaddrinfo: GetAddrInfoFn = detour_getaddrinfo;
    let gethostbyname: GetHostByNameFn = detour_gethostbyname;
    let targets = [
        Target {
            name: c"connect",
            original: &ORIGINAL_CONNECT,
            detour: connect as usize,
        },
        Target {
            name: c"sendto",
            original: &ORIGINAL_SENDTO,
            detour: sendto as usize,
        },
        Target {
            name: c"getaddrinfo",
            original: &ORIGINAL_GETADDRINFO,
            detour: getaddrinfo as usize,
        },
        Target {
            name: c"gethostbyname",
            original: &ORIGINAL_GETHOSTBYNAME,
            detour: gethostbyname as usize,
        },
    ];

    // SAFETY: `ws2_32.dll` is statically imported by the game, so it is already loaded and this
    // only takes a handle to it -- `GetModuleHandleA` does not reference-count and does not load.
    let ws2_32 = unsafe { GetModuleHandleA(c"ws2_32.dll".as_ptr()) };
    if ws2_32.is_null() {
        log(format_args!(
            "{LOG_PREFIX} winsock-failed stage=GetModuleHandleA module=ws2_32.dll"
        ));
        return Outcome {
            attempted: targets.len(),
            ..Outcome::default()
        };
    }
    // SAFETY: a named export of the module just resolved.
    let setter = unsafe { GetProcAddress(ws2_32, c"WSASetLastError".as_ptr()) };
    WSA_SET_LAST_ERROR.store(setter as usize, Ordering::Release);
    if setter.is_null() {
        // Not fatal: refusing without setting an error code still refuses. Logged because a caller
        // that then reads `WSAGetLastError` would see a stale value, which would be confusing in
        // exactly the situation someone is debugging this.
        log(format_args!(
            "{LOG_PREFIX} winsock-warning WSASetLastError unresolved; refusals carry no error code"
        ));
    }

    let Some(thunk_rva) = (unsafe { first_thunk_rva(base) }) else {
        log(format_args!(
            "{LOG_PREFIX} winsock-failed stage=import-walk module=WS2_32.dll"
        ));
        return Outcome {
            attempted: targets.len(),
            ..Outcome::default()
        };
    };

    let mut patched = 0;
    for target in &targets {
        // SAFETY: a name looked up in a module handle that is known good.
        let wanted = unsafe { GetProcAddress(ws2_32, target.name.as_ptr()) } as usize;
        let label = target.name.to_string_lossy();
        if wanted == 0 {
            log(format_args!(
                "{LOG_PREFIX} winsock-skip import={label} reason=GetProcAddress-null"
            ));
            continue;
        }
        // Match by ADDRESS. The import is by ordinal for most of these, so there is no name in the
        // thunk to compare against -- but the loader filled this slot from the same export table
        // `GetProcAddress` just read, so an equal pointer is an identification.
        let mut found = false;
        for index in 0..64usize {
            let slot =
                (base + thunk_rva as usize + index * std::mem::size_of::<usize>()) as *mut usize;
            let Some(current) = (unsafe { ds2_game_base::mem::safe_read_usize(slot as usize) })
            else {
                break;
            };
            if current == 0 {
                break;
            }
            if current != wanted {
                continue;
            }
            if unsafe { patch_slot(slot, target.detour, target.original) } {
                patched += 1;
                found = true;
                ANY_FRONTED.store(1, Ordering::Release);
                log(format_args!(
                    "{LOG_PREFIX} fronted import=WS2_32!{label} slot=0x{:016x} \
                     original=0x{wanted:016x}",
                    slot as usize
                ));
            } else {
                log(format_args!(
                    "{LOG_PREFIX} winsock-failed stage=VirtualProtect import={label} \
                     slot=0x{:016x}",
                    slot as usize
                ));
            }
            break;
        }
        if !found {
            // Not an error on its own: an import the game does not actually have is nothing to
            // front. It IS worth a line, because "we blocked nothing" and "there was nothing to
            // block" look identical in the counters otherwise.
            log(format_args!(
                "{LOG_PREFIX} winsock-skip import={label} reason=no-matching-iat-slot"
            ));
        }
    }

    Outcome {
        patched,
        attempted: targets.len(),
        found_ws2_32: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `sockaddr_in`: family, port (network order), four address bytes, eight bytes of padding.
    fn sockaddr_in(octets: [u8; 4], port: u16) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(16);
        buffer.extend_from_slice(&AF_INET.to_le_bytes());
        buffer.extend_from_slice(&port.to_be_bytes());
        buffer.extend_from_slice(&octets);
        buffer.extend_from_slice(&[0u8; 8]);
        buffer
    }

    /// A `sockaddr_in6`: family, port, flowinfo, sixteen address bytes, scope id.
    fn sockaddr_in6(address: [u8; 16], port: u16) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(28);
        buffer.extend_from_slice(&AF_INET6.to_le_bytes());
        buffer.extend_from_slice(&port.to_be_bytes());
        buffer.extend_from_slice(&0u32.to_le_bytes());
        buffer.extend_from_slice(&address);
        buffer.extend_from_slice(&0u32.to_le_bytes());
        buffer
    }

    fn decision(buffer: &[u8]) -> (bool, String) {
        // SAFETY: a buffer this test owns, with its real length.
        unsafe { allow(buffer.as_ptr(), buffer.len() as i32) }
    }

    #[test]
    fn loopback_v4_is_allowed_and_the_rest_is_not() {
        assert!(decision(&sockaddr_in([127, 0, 0, 1], 27015)).0);
        // The whole of 127/8, not just 127.0.0.1 -- the allowance is per the RFC, not per the one
        // address everybody types.
        assert!(decision(&sockaddr_in([127, 42, 3, 9], 80)).0);
        assert!(!decision(&sockaddr_in([8, 8, 8, 8], 53)).0);
        // The near miss that a sloppy check would let through.
        assert!(!decision(&sockaddr_in([128, 0, 0, 1], 80)).0);
        // 0.0.0.0 is not loopback. It reaches the default route, which is the wire.
        assert!(!decision(&sockaddr_in([0, 0, 0, 0], 80)).0);
    }

    #[test]
    fn loopback_v6_is_exactly_one_address() {
        let mut loopback = [0u8; 16];
        loopback[15] = 1;
        assert!(decision(&sockaddr_in6(loopback, 27015)).0);
        // `::2` is not loopback, and a check that only looked at the last byte would say it was.
        let mut near = [0u8; 16];
        near[15] = 2;
        assert!(!decision(&sockaddr_in6(near, 27015)).0);
        // `2001:db8::1` -- a routable address whose last byte is also 1.
        let mut routable = [0u8; 16];
        routable[0] = 0x20;
        routable[1] = 0x01;
        routable[2] = 0x0d;
        routable[3] = 0xb8;
        routable[15] = 1;
        assert!(!decision(&sockaddr_in6(routable, 27015)).0);
    }

    /// **The direction that matters.** Every way of failing to understand an address has to come
    /// out as "refuse", because the other answer puts a packet on the wire.
    #[test]
    fn anything_undecodable_is_refused() {
        assert!(!decision(&[]).0);
        assert!(!decision(&[2]).0);
        // Right family, truncated below the address field.
        assert!(!decision(&sockaddr_in([8, 8, 8, 8], 53)[..6]).0);
        assert!(!decision(&sockaddr_in6([0u8; 16], 53)[..20]).0);
        // A family this crate does not decode -- `AF_IPX`, say. Not loopback, so not allowed.
        assert!(!decision(&[6, 0, 0, 0, 0, 0, 0, 0]).0);
        // A null pointer with a plausible length.
        // SAFETY: `allow` is documented to reject a null pointer before dereferencing it, and
        // this test is the check on that.
        assert!(!unsafe { allow(std::ptr::null(), 16) }.0);
    }

    #[test]
    fn the_rendered_destination_names_the_address_the_game_asked_for() {
        assert_eq!(
            decision(&sockaddr_in([203, 0, 113, 7], 50000)).1,
            "203.0.113.7:50000"
        );
        assert_eq!(decision(&[]).1, "<undecodable>");
        let mut loopback = [0u8; 16];
        loopback[15] = 1;
        assert_eq!(
            decision(&sockaddr_in6(loopback, 443)).1,
            "[0000:0000:0000:0000:0000:0000:0000:0001]:443"
        );
    }

    /// The counters have to start where a report of "nothing was blocked" is distinguishable from
    /// "nothing was watching". Before any import is fronted, there is no answer to give.
    #[test]
    fn counts_are_absent_until_something_is_fronted() {
        assert_eq!(counts(), None);
    }
}
