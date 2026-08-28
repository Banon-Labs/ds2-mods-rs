//! Installing the save-directory detour, staging the configured save, and reporting both.

use core::ffi::c_void;
use std::os::windows::ffi::OsStrExt as _;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;
use crate::stage;

/// A log sink, installed by the loader so this crate writes into the same file as everything else.
/// Stored as a `usize` because a `fn` pointer is not an `Atomic` type.
static LOGGER: AtomicUsize = AtomicUsize::new(0);

/// Signature of the sink. Matches the loader's own logging entry point.
pub type LogFn = fn(std::fmt::Arguments<'_>);

/// Point this crate's logging at the loader's log file. Call before [`install`].
pub fn set_logger(logger: LogFn) {
    LOGGER.store(logger as usize, Ordering::Release);
    ds2_hook::set_hook_logger(logger);
}

fn log(args: std::fmt::Arguments<'_>) {
    let raw = LOGGER.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: `raw` is only ever a `LogFn` stored by `set_logger`.
        let logger: LogFn = unsafe { std::mem::transmute::<usize, LogFn>(raw) };
        logger(args);
    }
}

/// The configured source file: a `.sl2`, or a `.zip`/`.7z`/`.rar` containing one.
static SOURCE: OnceLock<PathBuf> = OnceLock::new();

/// Where the staged save is written. Set by the loader, which is the thing that knows the game
/// directory; this crate deliberately does not go looking for it.
static STAGING_ROOT: OnceLock<PathBuf> = OnceLock::new();

/// The staged directory as UTF-16, resolved on the detour's first call and reused after.
///
/// `None` means staging was attempted and failed, which is remembered so a failing archive is not
/// re-opened on every call to a function the game invokes more than once per boot.
static STAGED: OnceLock<Option<Vec<u16>>> = OnceLock::new();

/// The live module base, resolved once in [`install`] so the detour never has to.
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);

/// MinHook's trampoline back to the original directory builder.
static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// The original `FUN_140248db0(std::wstring *out, const wchar_t *steamid)`. Returns nothing.
type SaveDirFn = unsafe extern "system" fn(*mut c_void, *const u16);

/// The game's own `std::wstring::assign(dst, src, len)`. `len` counts `wchar_t`, not bytes.
type AssignFn = unsafe extern "system" fn(*mut c_void, *const u16, usize) -> *mut c_void;

/// Ask for the save to be loaded from `source`. Call before [`install`].
///
/// `source` is a path on the HOST filesystem as the DLL sees it -- under Proton that is the
/// Windows form, because this DLL runs inside the prefix. It names a file, not a directory: a
/// `.sl2`, or a `.zip`/`.7z`/`.rar` with exactly one `DS2SOFS0000.sl2` somewhere inside.
pub fn set_source(source: &str, staging_root: PathBuf) -> bool {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        log(format_args!(
            "{LOG_PREFIX} source-refused reason=empty -- the game's own directory is left alone"
        ));
        return false;
    }
    let ok = SOURCE.set(PathBuf::from(trimmed)).is_ok()
        && STAGING_ROOT.set(staging_root.clone()).is_ok();
    if ok {
        log(format_args!(
            "{LOG_PREFIX} source-armed path={trimmed} staging={}",
            staging_root.display()
        ));
    } else {
        log(format_args!(
            "{LOG_PREFIX} source-refused reason=already-set path={trimmed}"
        ));
    }
    ok
}

/// Whether a source was armed.
pub fn armed() -> bool {
    SOURCE.get().is_some()
}

/// Read a null-terminated UTF-16 string the game handed us.
///
/// # Safety
///
/// `raw` must be null, or point at a null-terminated `wchar_t` string.
unsafe fn wide_to_string(raw: *const u16, limit: usize) -> Option<String> {
    if raw.is_null() {
        return None;
    }
    let mut units = Vec::new();
    for i in 0..limit {
        // SAFETY: the caller promises a null-terminated string; `limit` bounds a missing
        // terminator so a malformed argument cannot walk the process.
        let unit = unsafe { raw.add(i).read() };
        if unit == 0 {
            return Some(String::from_utf16_lossy(&units));
        }
        units.push(unit);
    }
    None
}

/// Read a live MSVC `std::basic_string<wchar_t>` back out, for logging.
///
/// # Safety
///
/// `string` must point at a constructed `std::wstring` owned by the game.
unsafe fn read_wstring(string: *const c_void) -> String {
    if string.is_null() {
        return String::from("<null>");
    }
    // SAFETY: the caller promises a constructed string; the two field offsets and the small-string
    // discriminant are recorded in `ds2-rva` and read out of the game's own string helpers.
    let (len, capacity) = unsafe {
        (
            string
                .byte_add(ds2_rva::WSTRING_LEN_OFFSET)
                .cast::<usize>()
                .read(),
            string
                .byte_add(ds2_rva::WSTRING_CAPACITY_OFFSET)
                .cast::<usize>()
                .read(),
        )
    };
    if len > 0x8000 {
        return format!("<len={len} capacity={capacity} -- not a string?>");
    }
    // SAFETY: above the small-string maximum the first field is a pointer to the characters;
    // at or below it, the characters are the first field.
    let data = unsafe {
        if capacity > ds2_rva::WSTRING_SSO_MAX {
            string.cast::<*const u16>().read()
        } else {
            string.cast::<u16>()
        }
    };
    if data.is_null() {
        return String::from("<null-data>");
    }
    // SAFETY: `len` characters starting at `data`, both taken from the string itself.
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(data, len) })
}

/// Stage the configured source for `steam_id`, returning the directory as UTF-16 with a trailing
/// separator -- which is this function's job, since the caller appends the file name to it.
fn stage_now(steam_id: &str) -> Option<Vec<u16>> {
    let source = SOURCE.get()?;
    let root = STAGING_ROOT.get()?;
    match stage::stage(source, steam_id, root) {
        Ok(staged) => {
            log(format_args!(
                "{LOG_PREFIX} staged kind={} bytes={} replaced={} previous={} dir={}",
                staged.kind,
                staged.bytes,
                staged.rebound.replaced,
                staged.rebound.previous.as_deref().unwrap_or("<none>"),
                staged.directory.display()
            ));
            let mut wide: Vec<u16> = staged.directory.as_os_str().encode_wide().collect();
            if !matches!(wide.last(), Some(&c) if c == u16::from(b'\\') || c == u16::from(b'/')) {
                wide.push(u16::from(b'\\'));
            }
            Some(wide)
        }
        Err(error) => {
            // NOT fatal. A failed stage falls through to the game's own directory, which is the
            // only behaviour that leaves a bootable game -- and it is logged loudly because the
            // player would otherwise be quietly playing their own save believing otherwise.
            log(format_args!(
                "{LOG_PREFIX} stage-failed source={} error={error} -- FALLING BACK to the game's \
                 own save directory",
                source.display()
            ));
            None
        }
    }
}

/// The detour. Replaces the whole directory -- root and Steam ID folder both -- when armed.
///
/// # Safety
///
/// Called by the game with the Microsoft x64 ABI. `out` is a constructed `std::wstring` the caller
/// owns; `steamid` is the running account's ID as text, or null.
unsafe extern "system" fn detour_save_dir(out: *mut c_void, steamid: *const u16) {
    let pass_through = |note: &str| {
        let trampoline = TRAMPOLINE.load(Ordering::Acquire);
        if trampoline != 0 {
            // SAFETY: MinHook published this trampoline for this site, and the signature is the
            // one established from the call site at `0x1402e635c`.
            let original: SaveDirFn =
                unsafe { std::mem::transmute::<usize, SaveDirFn>(trampoline) };
            unsafe { original(out, steamid) };
        }
        log(format_args!(
            "{LOG_PREFIX} save-dir passthrough reason={note} path={}",
            unsafe { read_wstring(out) }
        ));
    };

    if !armed() {
        pass_through("not-armed");
        return;
    }
    // The account ID the game itself is about to use for the folder name. This is why the rebind
    // needs nothing from the config -- 64 units is far beyond a 16-character ID.
    let Some(steam_id) = (unsafe { wide_to_string(steamid, 64) }) else {
        pass_through("no-steam-id");
        return;
    };

    let staged = STAGED.get_or_init(|| stage_now(&steam_id));
    let Some(directory) = staged else {
        pass_through("stage-failed");
        return;
    };

    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        pass_through("no-module-base");
        return;
    }
    // SAFETY: `WSTRING_ASSIGN` is the game's own assign, at a recorded RVA in the loaded image.
    let assign: AssignFn =
        unsafe { std::mem::transmute::<usize, AssignFn>(base + ds2_rva::WSTRING_ASSIGN as usize) };
    // SAFETY: `out` is the caller's constructed string; `directory` lives in a `OnceLock` for the
    // rest of the process. Using the game's assign rather than writing the fields keeps its
    // allocation on its own heap.
    unsafe { assign(out, directory.as_ptr(), directory.len()) };

    // Read it back rather than logging what was intended. The two differ exactly when this is
    // broken, which is the only time the line matters.
    log(format_args!(
        "{LOG_PREFIX} save-dir redirected steam-id={steam_id} path={}",
        unsafe { read_wstring(out) }
    ));
}

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The directory builder is now detoured.
    pub hooked: bool,
    /// A source was armed by [`set_source`].
    pub armed: bool,
}

/// Detour the save-directory builder. Call from the post-Arxan callback, never `DllMain`.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Must run after `neuter_arxan`. The site was
/// checked with `scripts/ds2-arxan-chain.py`, which terminates at hop 0 with the clean prologue
/// `48 89 5c 24 08` at the entry -- an ordinary function, not one of the five-byte `e9` redirects
/// Arxan installs.
///
/// Staging itself does NOT happen here. It happens on the detour's first call, which is on the
/// game thread with the save system already up -- a far better place for archive decompression and
/// file writes than the loader callback that runs before the entry point.
pub unsafe fn install() -> Outcome {
    let armed = armed();
    let base = match ds2_game_base::mem::game_module_base() {
        Ok(base) => base,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} install-failed stage=module-base error={error}"
            ));
            return Outcome {
                hooked: false,
                armed,
            };
        }
    };
    MODULE_BASE.store(base, Ordering::Release);

    // MinHook is statically linked into this DLL, so ALREADY_INITIALIZED can only mean this ran
    // twice. Treat it as success, exactly as the other feature crates do.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=MH_Initialize status={status:?}"
        ));
        return Outcome {
            hooked: false,
            armed,
        };
    }

    let address = base + ds2_rva::SAVE_DIR_BUILD as usize;
    let hook = match unsafe { MhHook::new(address as *mut c_void, detour_save_dir as *mut c_void) }
    {
        Ok(hook) => hook,
        Err(status) => {
            log(format_args!(
                "{LOG_PREFIX} hook-failed site=save-dir va=0x{address:016x} stage=MH_CreateHook \
                 status={status:?}"
            ));
            return Outcome {
                hooked: false,
                armed,
            };
        }
    };
    // Published BEFORE the site is patched: every pass-through arm calls straight back through it,
    // and a detour that read a zero here would leave the save directory empty.
    TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
    let status = unsafe { MH_EnableHook(address as *mut c_void) };
    if status != MH_STATUS::MH_OK {
        log(format_args!(
            "{LOG_PREFIX} hook-failed site=save-dir va=0x{address:016x} stage=MH_EnableHook \
             status={status:?}"
        ));
        return Outcome {
            hooked: false,
            armed,
        };
    }
    log(format_args!(
        "{LOG_PREFIX} install hooked=true armed={armed} rva=0x{:08x} va=0x{address:016x}",
        ds2_rva::SAVE_DIR_BUILD
    ));
    Outcome {
        hooked: true,
        armed,
    }
}
