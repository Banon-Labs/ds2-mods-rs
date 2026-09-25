//! Installing the save-directory detour, staging the configured save, and reporting both.

use core::ffi::c_void;
use std::os::windows::ffi::OsStrExt as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;
use crate::active::active_save_file_name;
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

pub(crate) fn log(args: std::fmt::Arguments<'_>) {
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

/// A folder the whole launch reads and writes its saves in, instead of the game's own.
///
/// Set from `[save_redirect] directory` by the loader. Nothing is copied into it and nothing is
/// copied out: the game builds its container path inside it and reads and writes that file for the
/// rest of the session, so the save a player autoloaded is the save their progress goes back into.
static DIRECTORY: OnceLock<PathBuf> = OnceLock::new();

/// The staged directory as UTF-16, resolved on the detour's first call and reused after.
///
/// `None` means staging was attempted and failed, which is remembered so a failing archive is not
/// re-opened on every call to a function the game invokes more than once per boot.
static STAGED: OnceLock<Option<Vec<u16>>> = OnceLock::new();

/// The directory the detour last left behind, redirected or not.
///
/// Recorded in BOTH arms, because the crate that wants it -- the Save Game to File row -- needs the
/// directory the game is actually using, and whether that is ours or the game's own is exactly the
/// distinction it must not have to care about. A `Mutex<Option<..>>` rather than a `OnceLock`: the
/// answer changes if a redirect is armed, and the last one written is the true one.
static LIVE_DIRECTORY: Mutex<Option<PathBuf>> = Mutex::new(None);

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

/// Play out of `directory` for this whole launch. Call before [`install`].
///
/// # This is the key that was removed, built the other way round
///
/// `[save_redirect] path` named a file, and the only thing it could do with a file was copy it --
/// into the staging folder, which the next launch rewrote from the same source. So a session
/// started that way played a duplicate and lost everything done in it, under a help string that
/// said it had loaded the save. It was deleted rather than fixed, and this is the fix: a directory
/// is not copied anywhere. The game's own builder is answered with it, the game opens its own
/// container name inside it, and every write goes to that same file. Nothing here can lose a save
/// because nothing here writes one.
///
/// Refused when the folder is not there, because the alternative is a game showing no characters
/// with nothing on screen to say why -- DS2 hides the LOAD GAME row when it finds no container, so
/// "redirected to a folder that does not exist" and "there was never a save" look identical.
///
/// The container inside it does not have to exist. An empty writable folder is a legitimate fresh
/// start, and the game creates the save there the way it would in its own.
pub fn set_directory(directory: &str) -> bool {
    let trimmed = directory.trim();
    if trimmed.is_empty() {
        log(format_args!(
            "{LOG_PREFIX} directory-refused reason=empty -- the game's own directory is left alone"
        ));
        return false;
    }
    let path = PathBuf::from(trimmed);
    if !path.is_dir() {
        log(format_args!(
            "{LOG_PREFIX} directory-refused reason=not-a-directory path={trimmed} -- the game's \
             own directory is left alone. A redirect to a folder that is not there shows up as a \
             game with no saves, which is why this refuses instead of arming"
        ));
        return false;
    }
    let container = path.join(active_save_file_name());
    if DIRECTORY.set(path).is_err() {
        log(format_args!(
            "{LOG_PREFIX} directory-refused reason=already-set path={trimmed}"
        ));
        return false;
    }
    // Which of the two this is, said before the game boots rather than inferred from an empty
    // character list afterwards.
    let holding = if container.is_file() {
        "holds a container already, so this launch plays it and saves back into it"
    } else {
        "holds no container yet, so this launch starts fresh and creates one there"
    };
    log(format_args!(
        "{LOG_PREFIX} directory-armed path={trimmed} -- {holding}. Nothing is copied in either \
         direction"
    ));
    true
}

/// Whether a source or a directory was armed.
pub fn armed() -> bool {
    SOURCE.get().is_some() || DIRECTORY.get().is_some()
}

/// The directory the game's save-directory builder last produced, redirected or not.
///
/// `None` before the detour has run once, which on a live pause menu it always has -- the game
/// builds a save path to find out whether there is anything to load. The path is the WINDOWS form
/// the game itself built, ending in a separator.
pub fn live_directory() -> Option<PathBuf> {
    LIVE_DIRECTORY.lock().ok()?.clone()
}

/// The running account's Steam ID, as the game spells it when it names its own save folder.
///
/// `None` before the directory builder has run once. Sixteen hex characters on every account seen
/// so far, but stored as whatever the game passed rather than parsed, because the only thing it is
/// ever used for is being handed back to the game in a folder name or a rebind.
static STEAM_ID: Mutex<Option<String>> = Mutex::new(None);

/// The running account's Steam ID.
///
/// # Why this exists rather than reading the directory's last component
///
/// That is what the first version of `ds2-save-file`'s swap flow did, and the live log caught it:
/// with a launch-time redirect armed, the directory is
/// `...\Game\ds2-save-staging\` and its last component is `ds2-save-staging`. A container rebound to
/// that string is bound to an account that does not exist, and the game would show no characters in
/// it -- which looks exactly like a save that failed to stage. The ID the game itself passes to the
/// directory builder is the one that is always right.
pub fn live_steam_id() -> Option<String> {
    STEAM_ID.lock().ok()?.clone()
}

/// Record the ID the game handed the directory builder.
fn record_steam_id(id: &str) {
    if id.is_empty() || id.starts_with('<') {
        return;
    }
    if let Ok(mut held) = STEAM_ID.lock()
        && held.as_deref() != Some(id)
    {
        *held = Some(id.to_owned());
    }
}

/// Remember the directory the game is using, for the crates that need to find the `.sl2` in it.
///
/// Called with whatever `read_wstring` produced, which on a failed read is one of its own
/// `<...>` placeholders rather than a path -- so those are dropped instead of being remembered as a
/// directory named `<null>`.
fn record_live_directory(path: &str) {
    if path.is_empty() || path.starts_with('<') {
        return;
    }
    if let Ok(mut live) = LIVE_DIRECTORY.lock() {
        *live = Some(PathBuf::from(path));
    }
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
/// The folder this launch plays out of, as UTF-16 with a trailing separator.
///
/// The handoff wins over the configured directory, and has to: a handoff exists because the player
/// picked a file in the pause menu one launch ago and is waiting for it, while
/// `[save_redirect] directory` is a standing default they set once. Answering the default over the
/// request would silently discard the thing they just asked for.
fn redirect_directory(steam_id: &str) -> Option<Vec<u16>> {
    if SOURCE.get().is_some() {
        return stage_now(steam_id);
    }
    let directory = DIRECTORY.get()?;
    // The caller appends the container name to whatever this leaves behind, so the trailing
    // separator is this function's job -- the same contract `stage_now` fulfils below.
    let mut wide: Vec<u16> = directory.as_os_str().encode_wide().collect();
    if !matches!(wide.last(), Some(&c) if c == u16::from(b'\\') || c == u16::from(b'/')) {
        wide.push(u16::from(b'\\'));
    }
    Some(wide)
}

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

/// A directory that outranks everything else this detour would answer, for as long as it is set.
///
/// Set by the in-session character swap while it is at the title, and cleared by it on every path
/// that ends the flow. It is a `Mutex<Option<..>>` rather than the `OnceLock` the launch staging
/// uses because its whole purpose is to be armed and disarmed inside one process.
static SESSION_OVERRIDE: Mutex<Option<Vec<u16>>> = Mutex::new(None);

/// Times the session override answered, so a flow can report that it did rather than assume it.
static SESSION_ANSWERED: AtomicUsize = AtomicUsize::new(0);

/// Point every container directory this session builds at `windows_path`, until [`clear_session_directory`].
///
/// The path is taken as the game spells one: a Windows path ending in a separator, which is what
/// the original produces and what its callers append a filename to. A missing separator is added,
/// because the container would otherwise be looked for beside the folder rather than inside it.
///
/// Returns the number of times the override has answered so far, which is zero on a fresh arm.
pub fn set_session_directory(windows_path: &str) -> usize {
    let mut wide: Vec<u16> = windows_path.encode_utf16().collect();
    if !matches!(wide.last(), Some(&c) if c == u16::from(b'\\')) {
        wide.push(u16::from(b'\\'));
    }
    if let Ok(mut held) = SESSION_OVERRIDE.lock() {
        *held = Some(wide);
    }
    SESSION_ANSWERED.swap(0, Ordering::Relaxed)
}

/// Stop answering the session directory. Idempotent; returns how many times it answered.
pub fn clear_session_directory() -> usize {
    if let Ok(mut held) = SESSION_OVERRIDE.lock() {
        *held = None;
    }
    SESSION_ANSWERED.load(Ordering::Relaxed)
}

/// How many times the session override has answered since it was armed.
pub fn session_answers() -> usize {
    SESSION_ANSWERED.load(Ordering::Relaxed)
}

fn session_override() -> Option<Vec<u16>> {
    SESSION_OVERRIDE.lock().ok().and_then(|held| held.clone())
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
            // SAFETY: `original` is the trampoline MinHook produced for this target, so calling it runs the
            // bytes the detour displaced. The arguments are this detour's own, passed through untouched.
            unsafe { original(out, steamid) };
        }
        // SAFETY: the original has seated the caller's string, or nothing has and it is still the
        // constructed one the caller passed in.
        let produced = unsafe { read_wstring(out) };
        record_live_directory(&produced);
        log(format_args!(
            "{LOG_PREFIX} save-dir passthrough reason={note} path={produced}"
        ));
    };

    // The account ID the game itself is about to use for the folder name. This is why the rebind
    // needs nothing from the config -- 64 units is far beyond a 16-character ID.
    //
    // Read and recorded BEFORE the arming check, in both arms, because a caller that wants the ID
    // wants it whether or not a redirect is armed -- and the arm is exactly the case where the
    // directory it could otherwise be read off is NOT named after the account.
    // SAFETY: every pointer here is one the game handed this detour, or is derived from it by an
    // offset this crate validated before installing. The callee's own contract asks for exactly
    // that live object, and reads inside it go through the fault-tolerant readers.
    let found = unsafe { wide_to_string(steamid, 64) };
    if let Some(id) = found.as_deref() {
        record_steam_id(id);
    }

    // THE SESSION OVERRIDE COMES FIRST, and it is the seam the in-session swap has to use.
    //
    // `SLLoadSession`'s directory virtual is not what a container read opens. The work method
    // (`SL_LOAD_SESSION_WORK`) reads the content's own string inline -- `[this+0xe8]`, through the
    // same accessor the virtual uses -- and never calls that virtual, so swapping its vtable slot
    // changes the answer to a question the read does not ask. One run measured exactly that: the
    // slot was armed, `load-answered=1` says the game reached it, and the read still failed,
    // because the content's string had been built here, at session setup, from the player's own
    // folder.
    //
    // So the directory a container read uses is the one this function produces, and this is where
    // an in-session redirect belongs. It moves both sides at once, which is safe only in the window
    // the swap arms it for: between the return to the title and a character being chosen, no
    // character is loaded, so there is nothing a save could write.
    if let Some(directory) = session_override() {
        let base = MODULE_BASE.load(Ordering::Acquire);
        if base == 0 {
            pass_through("session-override-no-module-base");
            return;
        }
        // SAFETY: `WSTRING_ASSIGN` is the game's own assign, at a recorded RVA in the loaded image.
        let assign: AssignFn = unsafe {
            std::mem::transmute::<usize, AssignFn>(base + ds2_rva::WSTRING_ASSIGN as usize)
        };
        // SAFETY: `out` is the caller's constructed string and `directory` is this module's own
        // buffer, alive for the length of the call.
        unsafe { assign(out, directory.as_ptr(), directory.len()) };
        // SAFETY: the assign above seated the caller's own constructed string.
        let produced = unsafe { read_wstring(out) };
        let n = SESSION_ANSWERED.fetch_add(1, Ordering::Relaxed) + 1;
        log(format_args!(
            "{LOG_PREFIX} save-dir session-override count={n} path={produced}"
        ));
        return;
    }

    if !armed() {
        pass_through("not-armed");
        return;
    }
    let Some(steam_id) = found else {
        pass_through("no-steam-id");
        return;
    };

    let staged = STAGED.get_or_init(|| redirect_directory(&steam_id));
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
    // SAFETY: the assign above seated the caller's own constructed string.
    let produced = unsafe { read_wstring(out) };
    record_live_directory(&produced);
    log(format_args!(
        "{LOG_PREFIX} save-dir redirected steam-id={steam_id} path={produced}"
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
    // SAFETY: `MH_Initialize` takes no arguments and is documented as safe to call again on an
    // already-initialised library, which the status below distinguishes.
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
    // SAFETY: the target is an RVA this crate validated against the prologue it expects before
    // reaching here, and the detour is a `'static` fn item of the matching ABI.
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
    // SAFETY: the target is the address `MhHook::new` above already registered with MinHook.
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
