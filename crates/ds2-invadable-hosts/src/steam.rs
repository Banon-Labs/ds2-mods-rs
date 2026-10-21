//! The matchmaking interface the census and Seamless Co-op share.
//!
//! # Why the vtable, and why a copy of it
//!
//! Steam keeps one lobby-list request in flight per interface, and the second one sent wins: a
//! census query sent into Seamless's search would cost Seamless its answer, and one sent between
//! Seamless's filters and its request would consume Seamless's filters. So the census has to know
//! when Seamless is staging a query and when one of its requests is still out.
//!
//! Seamless calls through the very object the game's own `SteamMatchmaking()` export returns
//! (measured: the `this` of every call it made, `scripts/frida/ds2sc-lobby-watch.js`), and a C++
//! call reads the object's vtable pointer every time. Pointing that one pointer at a copy of the
//! table whose staging, request and `SetLobbyData` entries are wrappers is enough to see every call:
//! nothing in `steamclient64.dll`, the game or Seamless is patched, and the census itself calls the
//! original entries, never the wrappers.
//!
//! # The lock
//!
//! Every wrapper and every census query take [`LOCK`], so a census query's filters and request are
//! one step that no Seamless call lands inside, and the census sends only when Seamless has no
//! filter staged and no request out. A census query out when Seamless sends is answered as
//! [`Answer::Failed`]: Seamless's request replaced it, which is the order that keeps Seamless's
//! answer.

use std::ffi::{CStr, c_char, c_void};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

use ds2_game_base::mem::safe_read_usize;
use ds2_seamless::lobby;

use crate::LOG_PREFIX;
use crate::census::{self, Answer, Split};
use crate::install::log;

/// A census query sent but unanswered for this long is failed.
const QUERY_TIMEOUT_MS: u64 = 20_000;

static IFACE: AtomicUsize = AtomicUsize::new(0);
static UTILS: AtomicUsize = AtomicUsize::new(0);
/// The table as `steamclient` built it; the census calls its entries.
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);

/// Serialises every lobby-list call either mod makes on the interface.
static LOCK: Mutex<()> = Mutex::new(());

/// Filters Seamless has added since its last request.
static SEAMLESS_STAGED: AtomicU32 = AtomicU32::new(0);
/// Seamless's request still out, or 0.
static SEAMLESS_CALL: AtomicU64 = AtomicU64::new(0);
/// Requests Seamless has sent, so a census query knows whether one went out after it.
static SEAMLESS_REQUESTS: AtomicU64 = AtomicU64::new(0);
/// Wrapper calls of each kind, for the heartbeat: filters, requests, lobby data.
static SEEN_FILTERS: AtomicU64 = AtomicU64::new(0);
static SEEN_PUBLISHES: AtomicU64 = AtomicU64::new(0);

/// The regulation value Seamless last filtered or published on, this session.
static REGULATION: Mutex<Option<String>> = Mutex::new(None);
/// This player's bracket as Seamless last filtered or published it, or `usize::MAX`.
static OWN_BRACKET: AtomicUsize = AtomicUsize::new(usize::MAX);

type StringFilterFn = unsafe extern "system" fn(usize, *const c_char, *const c_char, i32);
type IntFilterFn = unsafe extern "system" fn(usize, *const c_char, i32, i32);
type NearFilterFn = unsafe extern "system" fn(usize, *const c_char, i32);
type OneIntFn = unsafe extern "system" fn(usize, i32);
type CompatFn = unsafe extern "system" fn(usize, u64);
type RequestFn = unsafe extern "system" fn(usize) -> u64;
// `CSteamID GetLobbyByIndex(int)`. `CSteamID` is a class, so MSVC returns it through a hidden pointer
// passed after `this` (measured: called as `(this, index) -> u64` it wrote through the index and
// faulted in `lsteamclient.dll` at 0xbeef, 2026-10-21).
type ByIndexFn = unsafe extern "system" fn(usize, *mut u64, i32) -> *mut u64;
type SetDataFn = unsafe extern "system" fn(usize, u64, *const c_char, *const c_char) -> bool;
type CompletedFn = unsafe extern "system" fn(usize, u64, *mut bool) -> bool;
type ResultFn = unsafe extern "system" fn(usize, u64, *mut c_void, i32, i32, *mut bool) -> bool;
type AccessorFn = unsafe extern "C" fn() -> usize;

unsafe extern "system" {
    fn GetModuleHandleA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

/// The original table's entry at a byte offset.
fn original(slot: usize) -> usize {
    let table = ORIGINAL.load(Ordering::Acquire);
    // SAFETY: `ORIGINAL` is only ever set by `install` to a table it read
    // `STEAM_MATCHMAKING_VTABLE_COPY_ENTRIES` entries from, and every slot asked for is below that.
    unsafe { safe_read_usize(table + slot) }.unwrap_or(0)
}

fn text(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // SAFETY: Steam's filter and lobby-data arguments are NUL-terminated strings the caller owns
    // for the duration of the call, which is the only time this reads them.
    Some(
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned(),
    )
}

/// Record what a Seamless key/value pair says about this player: its regulation and its bracket.
fn note(key: *const c_char, value: *const c_char) {
    let (Some(key), Some(value)) = (text(key), text(value)) else {
        return;
    };
    if key == lobby::REGULATION_KEY {
        let mut slot = REGULATION
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.as_deref() != Some(value.as_str()) {
            log(format_args!(
                "{LOG_PREFIX} pool: Seamless uses regulation {value:?} (was {:?})",
                slot.as_deref()
            ));
            *slot = Some(value);
        }
    } else if key == lobby::BRACKET_KEY
        && let Some(bracket) = census::parse_bracket_value(&value)
    {
        let before = OWN_BRACKET.swap(bracket, Ordering::AcqRel);
        if before != bracket {
            log(format_args!(
                "{LOG_PREFIX} own bracket: Seamless uses {value:?} (bracket {bracket})"
            ));
        }
    }
}

fn staged() {
    SEAMLESS_STAGED.fetch_add(1, Ordering::AcqRel);
    SEEN_FILTERS.fetch_add(1, Ordering::Relaxed);
}

unsafe extern "system" fn wrap_string_filter(
    this: usize,
    key: *const c_char,
    value: *const c_char,
    comparison: i32,
) {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    staged();
    note(key, value);
    let real = original(ds2_rva::STEAM_MATCHMAKING_ADD_STRING_FILTER_SLOT);
    // SAFETY: the original entry of this slot, called with the caller's own arguments.
    unsafe { std::mem::transmute::<usize, StringFilterFn>(real)(this, key, value, comparison) }
}

unsafe extern "system" fn wrap_numerical_filter(
    this: usize,
    key: *const c_char,
    value: i32,
    comparison: i32,
) {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    staged();
    let real = original(ds2_rva::STEAM_MATCHMAKING_ADD_NUMERICAL_FILTER_SLOT);
    // SAFETY: as above.
    unsafe { std::mem::transmute::<usize, IntFilterFn>(real)(this, key, value, comparison) }
}

unsafe extern "system" fn wrap_near_filter(this: usize, key: *const c_char, value: i32) {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    staged();
    let real = original(ds2_rva::STEAM_MATCHMAKING_ADD_NEAR_VALUE_FILTER_SLOT);
    // SAFETY: as above.
    unsafe { std::mem::transmute::<usize, NearFilterFn>(real)(this, key, value) }
}

unsafe extern "system" fn wrap_slots_filter(this: usize, value: i32) {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    staged();
    let real = original(ds2_rva::STEAM_MATCHMAKING_ADD_SLOTS_AVAILABLE_FILTER_SLOT);
    // SAFETY: as above.
    unsafe { std::mem::transmute::<usize, OneIntFn>(real)(this, value) }
}

unsafe extern "system" fn wrap_distance_filter(this: usize, value: i32) {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    staged();
    let real = original(ds2_rva::STEAM_MATCHMAKING_ADD_DISTANCE_FILTER_SLOT);
    // SAFETY: as above.
    unsafe { std::mem::transmute::<usize, OneIntFn>(real)(this, value) }
}

unsafe extern "system" fn wrap_count_filter(this: usize, value: i32) {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    staged();
    let real = original(ds2_rva::STEAM_MATCHMAKING_ADD_RESULT_COUNT_FILTER_SLOT);
    // SAFETY: as above.
    unsafe { std::mem::transmute::<usize, OneIntFn>(real)(this, value) }
}

unsafe extern "system" fn wrap_compatible_filter(this: usize, member: u64) {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    staged();
    let real = original(ds2_rva::STEAM_MATCHMAKING_ADD_COMPATIBLE_MEMBERS_FILTER_SLOT);
    // SAFETY: as above.
    unsafe { std::mem::transmute::<usize, CompatFn>(real)(this, member) }
}

unsafe extern "system" fn wrap_request(this: usize) -> u64 {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let real = original(ds2_rva::STEAM_MATCHMAKING_REQUEST_LOBBY_LIST_SLOT);
    // SAFETY: as above.
    let call = unsafe { std::mem::transmute::<usize, RequestFn>(real)(this) };
    SEAMLESS_STAGED.store(0, Ordering::Release);
    SEAMLESS_CALL.store(call, Ordering::Release);
    SEAMLESS_REQUESTS.fetch_add(1, Ordering::AcqRel);
    call
}

unsafe extern "system" fn wrap_set_lobby_data(
    this: usize,
    lobby_id: u64,
    key: *const c_char,
    value: *const c_char,
) -> bool {
    SEEN_PUBLISHES.fetch_add(1, Ordering::Relaxed);
    note(key, value);
    let real = original(ds2_rva::STEAM_MATCHMAKING_SET_LOBBY_DATA_SLOT);
    // SAFETY: as above.
    unsafe { std::mem::transmute::<usize, SetDataFn>(real)(this, lobby_id, key, value) }
}

fn steam_export(name: &CStr) -> Option<usize> {
    // SAFETY: both strings are NUL-terminated; a missing module or export is a null, handled.
    unsafe {
        let module = GetModuleHandleA(c"steam_api64.dll".as_ptr().cast());
        if module.is_null() {
            return None;
        }
        let proc = GetProcAddress(module, name.as_ptr().cast());
        (!proc.is_null()).then_some(proc as usize)
    }
}

/// Point the interface at the wrapped copy. `Err` says why not; nothing was changed then.
pub(crate) fn install() -> Result<(), String> {
    if ORIGINAL.load(Ordering::Acquire) != 0 {
        return Ok(());
    }
    let matchmaking = steam_export(c"SteamMatchmaking").ok_or("no SteamMatchmaking export")?;
    let utils = steam_export(c"SteamUtils").ok_or("no SteamUtils export")?;
    // SAFETY: niladic exports of the game's own steam_api64.dll, fixed by the Steamworks ABI.
    let (iface, utils) = unsafe {
        (
            std::mem::transmute::<usize, AccessorFn>(matchmaking)(),
            std::mem::transmute::<usize, AccessorFn>(utils)(),
        )
    };
    if iface == 0 || utils == 0 {
        return Err(format!(
            "Steam not ready: matchmaking={iface:#x} utils={utils:#x}"
        ));
    }
    // SAFETY: a fault-safe read of the object's first word, its vtable pointer.
    let table = unsafe { safe_read_usize(iface) }.ok_or("the interface's vtable is unreadable")?;
    let mut copy = vec![0usize; ds2_rva::STEAM_MATCHMAKING_VTABLE_COPY_ENTRIES];
    for (index, entry) in copy.iter_mut().enumerate() {
        // SAFETY: fault-safe reads; an unreadable entry past the table's end stays 0, and a slot
        // past the end is never called.
        *entry = unsafe { safe_read_usize(table + index * 8) }.unwrap_or(0);
    }
    let wrappers: [(usize, usize); 9] = [
        (
            ds2_rva::STEAM_MATCHMAKING_ADD_STRING_FILTER_SLOT,
            wrap_string_filter as *const () as usize,
        ),
        (
            ds2_rva::STEAM_MATCHMAKING_ADD_NUMERICAL_FILTER_SLOT,
            wrap_numerical_filter as *const () as usize,
        ),
        (
            ds2_rva::STEAM_MATCHMAKING_ADD_NEAR_VALUE_FILTER_SLOT,
            wrap_near_filter as *const () as usize,
        ),
        (
            ds2_rva::STEAM_MATCHMAKING_ADD_SLOTS_AVAILABLE_FILTER_SLOT,
            wrap_slots_filter as *const () as usize,
        ),
        (
            ds2_rva::STEAM_MATCHMAKING_ADD_DISTANCE_FILTER_SLOT,
            wrap_distance_filter as *const () as usize,
        ),
        (
            ds2_rva::STEAM_MATCHMAKING_ADD_RESULT_COUNT_FILTER_SLOT,
            wrap_count_filter as *const () as usize,
        ),
        (
            ds2_rva::STEAM_MATCHMAKING_ADD_COMPATIBLE_MEMBERS_FILTER_SLOT,
            wrap_compatible_filter as *const () as usize,
        ),
        (
            ds2_rva::STEAM_MATCHMAKING_REQUEST_LOBBY_LIST_SLOT,
            wrap_request as *const () as usize,
        ),
        (
            ds2_rva::STEAM_MATCHMAKING_SET_LOBBY_DATA_SLOT,
            wrap_set_lobby_data as *const () as usize,
        ),
    ];
    for (slot, _) in wrappers {
        if copy[slot / 8] == 0 {
            return Err(format!("the interface's slot {:#x} is empty", slot));
        }
    }
    for (slot, wrapper) in wrappers {
        copy[slot / 8] = wrapper;
    }
    let copy: &'static mut [usize] = Box::leak(copy.into_boxed_slice());
    UTILS.store(utils, Ordering::Release);
    IFACE.store(iface, Ordering::Release);
    ORIGINAL.store(table, Ordering::Release);
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // SAFETY: `iface` is the live interface object; its first word is its vtable pointer, in
    // writable heap memory. One aligned word store, the same width every caller reads.
    unsafe { core::ptr::write_volatile(iface as *mut usize, copy.as_ptr() as usize) };
    log(format_args!(
        "{LOG_PREFIX} matchmaking interface {iface:#x}: vtable {table:#x} -> wrapped copy {:#x} \
         (9 lobby-list and lobby-data entries noted, the rest the original's)",
        copy.as_ptr() as usize
    ));
    Ok(())
}

/// Whether Seamless has a filter staged or a request out. The census sends nothing while it does.
#[must_use]
pub(crate) fn seamless_busy() -> bool {
    if SEAMLESS_STAGED.load(Ordering::Acquire) != 0 {
        return true;
    }
    let call = SEAMLESS_CALL.load(Ordering::Acquire);
    if call == 0 {
        return false;
    }
    if completed(call).is_some() {
        let _ = SEAMLESS_CALL.compare_exchange(call, 0, Ordering::AcqRel, Ordering::Acquire);
        return false;
    }
    true
}

/// `Some(failed)` once Steam has finished the call.
fn completed(call: u64) -> Option<bool> {
    let utils = UTILS.load(Ordering::Acquire);
    // SAFETY: a fault-safe read of the utils object's vtable and the slot measured for it.
    let entry = unsafe {
        safe_read_usize(utils).and_then(|table| {
            safe_read_usize(table + ds2_rva::STEAM_UTILS_IS_API_CALL_COMPLETED_SLOT)
        })
    }?;
    let mut failed = false;
    // SAFETY: `ISteamUtils::IsAPICallCompleted(this, call, &failed)`.
    let done =
        unsafe { std::mem::transmute::<usize, CompletedFn>(entry)(utils, call, &mut failed) };
    done.then_some(failed)
}

/// A census query sent: its call and when, and how many Seamless requests had gone out by then.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sent {
    call: u64,
    sent_ms: u64,
    seamless_requests: u64,
}

/// Send one census query, or `None` when the interface is not free (Seamless staging or waiting)
/// or not installed; nothing went out then.
pub(crate) fn send(query: &census::Query, regulation: &str, now_ms: u64) -> Option<Sent> {
    let iface = IFACE.load(Ordering::Acquire);
    if iface == 0 {
        return None;
    }
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if seamless_busy() {
        return None;
    }
    let bracket = std::ffi::CString::new(query.bracket_value()).ok()?;
    let regulation = std::ffi::CString::new(regulation).ok()?;
    let marker_key = std::ffi::CString::new(lobby::MARKER_KEY).ok()?;
    let marker = std::ffi::CString::new(lobby::MARKER_VALUE).ok()?;
    let open_key = std::ffi::CString::new(lobby::OPEN_KEY).ok()?;
    let open = std::ffi::CString::new(lobby::OPEN_VALUE).ok()?;
    let bracket_key = std::ffi::CString::new(lobby::BRACKET_KEY).ok()?;
    let regulation_key = std::ffi::CString::new(lobby::REGULATION_KEY).ok()?;
    let flags_key = std::ffi::CString::new(lobby::FLAGS_KEY).ok()?;
    // SAFETY: the original entries of the interface's own table, called on the interface with
    // strings that outlive the request that consumes them (Steam copies filters when they are
    // added; they are also alive until this function returns, after the request).
    unsafe {
        let string = std::mem::transmute::<usize, StringFilterFn>(original(
            ds2_rva::STEAM_MATCHMAKING_ADD_STRING_FILTER_SLOT,
        ));
        let equal = ds2_rva::STEAM_LOBBY_COMPARISON_EQUAL;
        string(iface, marker_key.as_ptr(), marker.as_ptr(), equal);
        string(iface, open_key.as_ptr(), open.as_ptr(), equal);
        string(iface, bracket_key.as_ptr(), bracket.as_ptr(), equal);
        string(iface, regulation_key.as_ptr(), regulation.as_ptr(), equal);
        let comparison = match query.split {
            Split::Whole => None,
            Split::FlagsAtMost => Some(ds2_rva::STEAM_LOBBY_COMPARISON_AT_MOST),
            Split::FlagsAbove => Some(ds2_rva::STEAM_LOBBY_COMPARISON_ABOVE),
        };
        if let Some(comparison) = comparison {
            std::mem::transmute::<usize, IntFilterFn>(original(
                ds2_rva::STEAM_MATCHMAKING_ADD_NUMERICAL_FILTER_SLOT,
            ))(
                iface,
                flags_key.as_ptr(),
                census::SPLIT_FLAGS_AT,
                comparison,
            );
        }
        std::mem::transmute::<usize, OneIntFn>(original(
            ds2_rva::STEAM_MATCHMAKING_ADD_DISTANCE_FILTER_SLOT,
        ))(iface, lobby::SEARCH_DISTANCE);
        std::mem::transmute::<usize, OneIntFn>(original(
            ds2_rva::STEAM_MATCHMAKING_ADD_RESULT_COUNT_FILTER_SLOT,
        ))(iface, census::RESULT_COUNT_REQUEST);
        let call = std::mem::transmute::<usize, RequestFn>(original(
            ds2_rva::STEAM_MATCHMAKING_REQUEST_LOBBY_LIST_SLOT,
        ))(iface);
        (call != 0).then_some(Sent {
            call,
            sent_ms: now_ms,
            seamless_requests: SEAMLESS_REQUESTS.load(Ordering::Acquire),
        })
    }
}

/// Collect a census query: `None` while Steam has not answered.
pub(crate) fn poll(sent: &Sent, now_ms: u64) -> Option<Answer> {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Seamless sent after us: Steam replaced ours with its request, and the interface's list, if
    // ours ever arrived, is no longer ours to read.
    if SEAMLESS_REQUESTS.load(Ordering::Acquire) != sent.seamless_requests {
        return Some(Answer::Failed);
    }
    let Some(failed) = completed(sent.call) else {
        return (now_ms.saturating_sub(sent.sent_ms) > QUERY_TIMEOUT_MS).then_some(Answer::Failed);
    };
    if failed {
        return Some(Answer::Failed);
    }
    let utils = UTILS.load(Ordering::Acquire);
    // SAFETY: fault-safe reads of the utils vtable and its measured slot.
    let result = unsafe {
        safe_read_usize(utils).and_then(|table| {
            safe_read_usize(table + ds2_rva::STEAM_UTILS_GET_API_CALL_RESULT_SLOT)
        })
    };
    let Some(result) = result else {
        return Some(Answer::Failed);
    };
    let mut matching = 0u32;
    let mut io_failed = false;
    // SAFETY: `GetAPICallResult(this, call, &LobbyMatchList_t, sizeof, k_iCallback, &failed)`;
    // `LobbyMatchList_t` is one `u32`.
    let ok = unsafe {
        std::mem::transmute::<usize, ResultFn>(result)(
            utils,
            sent.call,
            (&raw mut matching).cast(),
            4,
            ds2_rva::STEAM_LOBBY_MATCH_LIST_CALLBACK,
            &mut io_failed,
        )
    };
    if !ok || io_failed {
        return Some(Answer::Failed);
    }
    let iface = IFACE.load(Ordering::Acquire);
    let by_index = original(ds2_rva::STEAM_MATCHMAKING_GET_LOBBY_BY_INDEX_SLOT);
    let ids = (0..matching)
        .filter_map(|index| i32::try_from(index).ok())
        .map(|index| {
            let mut id = 0u64;
            // SAFETY: `GetLobbyByIndex(this, &out, index)` with an index below the count Steam
            // reported for this very query, read before any other request could replace the list
            // (the lock); `out` is a local `u64`, the size of a `CSteamID`.
            unsafe { std::mem::transmute::<usize, ByIndexFn>(by_index)(iface, &raw mut id, index) };
            id
        })
        .filter(|id| *id != 0)
        .collect();
    Some(Answer::Lobbies(ids))
}

/// The regulation value Seamless was seen using this session, if it has been.
#[must_use]
pub(crate) fn seen_regulation() -> Option<String> {
    REGULATION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// This player's bracket as Seamless last used it, if it has this session.
#[must_use]
pub(crate) fn seen_own_bracket() -> Option<usize> {
    let bracket = OWN_BRACKET.load(Ordering::Acquire);
    (bracket < census::BRACKET_COUNT).then_some(bracket)
}

/// Wrapper calls seen: (Seamless filters, Seamless requests, lobby-data writes).
#[must_use]
pub(crate) fn seen_counts() -> (u64, u64, u64) {
    (
        SEEN_FILTERS.load(Ordering::Relaxed),
        SEAMLESS_REQUESTS.load(Ordering::Relaxed),
        SEEN_PUBLISHES.load(Ordering::Relaxed),
    )
}
