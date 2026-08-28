//! The two code patches on the network service's online flag.
//!
//! Both are three-byte stubs through [`ds2_hook::patch_3byte_stub`], which validates the byte it is
//! about to overwrite before it writes anything. Neither needs MinHook, a trampoline or a
//! relocation: the two functions are five and four bytes long, they are leaves, and both sit at
//! their own entry rather than behind an Arxan redirect (checked with `scripts/ds2-arxan-chain.py`,
//! which terminates at hop 0 on the real prologue for each).
//!
//! # Why the pair, in this order
//!
//! `pin` is the primary and `report` is the backstop, and they are separate switches so a run that
//! misbehaves can be attributed to one of them by editing one line.
//!
//! * `pin` makes `NetService::setOnline` a `ret`. The constructor already wrote zero into the
//!   flag, so the object simply stays as built.
//! * `report` makes `NetService::isOnline` `xor eax,eax; ret`, which answers all 34 of its readers
//!   regardless of what the byte holds.
//!
//! Applying only `report` would leave the byte free to become 1 and lie to any reader that does
//! not use the getter. Applying only `pin` rests on "the setter is the only writer", which is an
//! inference from one pattern search over one image. Applying both costs six bytes.
//!
//! # Every patch is read back
//!
//! [`patch_and_verify`] compares the bytes in memory against the stub it asked for and returns
//! that comparison, not the write's permission. See its own doc for why the difference is
//! load-bearing in this crate specifically.

use crate::{LOG_PREFIX, install::log};

/// What [`apply`] did, per patch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// `NetService::setOnline` is now `ret`, so the flag keeps its constructed zero.
    pub setter_pinned: bool,
    /// `NetService::isOnline` now returns zero unconditionally.
    pub getter_forced: bool,
}

/// Patch one of the two, then **read the bytes back** and report what is actually there.
///
/// `patch_3byte_stub` returning true means the expected byte was found and `VirtualProtect`
/// allowed the write. That is not the same claim as "the stub is in memory" -- `ds2_hook`'s own
/// documentation says so, in as many words: a caller patching game code should read it back,
/// because another mod can own the same address and a successful `VirtualProtect` says nothing
/// about that.
///
/// The distinction is not academic here. Every other feature in this workspace fails visibly --
/// a screen still shows, a dialog still waits. This one fails by telling the player they are
/// offline when they are not, and the only thing standing between that and a soft-ban is whether
/// three bytes are where this function says they are. So the log line carries the bytes rather
/// than a boolean, and the return value is the comparison rather than the write's permission.
///
/// # Safety
///
/// Writes three bytes of executable memory in the loaded game image. Must run after Arxan has been
/// dealt with (the loader's post-Arxan callback) and before the title flow reaches the network
/// substates, which in practice is the same position every other patch in this workspace installs
/// from. `base` must be the live base of `DarkSoulsII.exe`.
unsafe fn patch_and_verify(
    base: usize,
    rva: u32,
    expected_first: u8,
    stub: [u8; 3],
    label: &str,
) -> bool {
    let site = base + rva as usize;
    if !ds2_hook::patch_3byte_stub(
        base,
        rva as usize,
        expected_first,
        stub,
        &format!("{LOG_PREFIX} {label}"),
    ) {
        // `patch_3byte_stub` has already logged which guard refused -- a wrong first byte names
        // both bytes, a refused `VirtualProtect` says so. Repeating it here would only add a
        // second line saying less.
        return false;
    }
    let mut live = [0u8; 3];
    // SAFETY: three bytes of the function just patched, in the loaded image.
    if !unsafe { ds2_game_base::mem::read_bytes(site, &mut live) } {
        log(format_args!(
            "{LOG_PREFIX} {label} UNVERIFIED va=0x{site:016x} -- the write was permitted but the \
             bytes could not be read back"
        ));
        return false;
    }
    let landed = live == stub;
    log(format_args!(
        "{LOG_PREFIX} {label} va=0x{site:016x} wrote={} live={} landed={landed}",
        hex3(stub),
        hex3(live),
    ));
    if !landed {
        log(format_args!(
            "{LOG_PREFIX} {label} OVERWRITTEN -- something else owns this address; this run is \
             NOT offline by way of {label}"
        ));
    }
    landed
}

/// Three bytes as `xx xx xx`, so a log line can be compared against the disassembly by eye.
fn hex3(bytes: [u8; 3]) -> String {
    format!("{:02x} {:02x} {:02x}", bytes[0], bytes[1], bytes[2])
}

/// Apply whichever of the two patches was asked for, and verify each.
///
/// # Safety
///
/// Patches executable memory. See [`patch_and_verify`].
pub unsafe fn apply(base: usize, pin: bool, report: bool) -> Outcome {
    Outcome {
        // `NetService::setOnline` -> `ret`. The primary: the constructor already wrote zero into
        // the flag, so this keeps the object in the state the game built it in.
        setter_pinned: pin
            && unsafe {
                patch_and_verify(
                    base,
                    ds2_rva::NET_SET_ONLINE,
                    ds2_rva::NET_SET_ONLINE_EXPECTED_FIRST,
                    ds2_rva::NET_SET_ONLINE_STUB,
                    "set-online",
                )
            },
        // `NetService::isOnline` -> `xor eax,eax; ret`. The backstop, answering all 34 readers
        // whatever the byte holds.
        getter_forced: report
            && unsafe {
                patch_and_verify(
                    base,
                    ds2_rva::NET_IS_ONLINE,
                    ds2_rva::NET_IS_ONLINE_EXPECTED_FIRST,
                    ds2_rva::NET_IS_ONLINE_STUB,
                    "is-online",
                )
            },
    }
}

/// Read the flag back out of the live network service, or `None` if the chain is not up yet.
///
/// This is the crate's own audit of its own patch, and it deliberately does not go through
/// `isOnline` -- reading the byte the getter would have read is the only check that survives
/// [`patch_and_verify`] having patched the getter to lie about it.
///
/// Every hop is a [`ds2_game_base::mem`] guarded read, so calling this before `GameManagerImp`
/// exists returns `None` rather than faulting. At the position the loader installs from, the
/// singleton is usually still null; that is expected and is why the caller logs the absence rather
/// than treating it as a failure.
///
/// # Safety
///
/// Dereferences game pointers. The reads are bounds- and null-checked by the helpers, but the
/// offsets are only meaningful for build 9527516.
pub unsafe fn read_flag(base: usize) -> Option<u8> {
    let manager_slot = base + ds2_rva::GAME_MANAGER_IMP as usize;
    let manager = unsafe { ds2_game_base::mem::safe_read_usize(manager_slot)? };
    if manager == 0 {
        return None;
    }
    let service =
        unsafe { ds2_game_base::mem::safe_read_usize(manager + ds2_rva::NET_SERVICE_OFFSET)? };
    if service == 0 {
        return None;
    }
    unsafe { ds2_game_base::mem::safe_read_u8(service + ds2_rva::NET_ONLINE_FLAG_OFFSET) }
}
