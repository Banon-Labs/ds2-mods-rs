//! Whether a multiplayer session is up, asked the way the shipped Quit Game row asks it.
//!
//! Every row this crate adds is refused while one is: the press does nothing and the row is drawn
//! grey. The question is the game's own -- gate `4`'s branch of `FUN_1400a4e50`, which resolves
//! [`ds2_rva::NET_SERVICE_SESSION_OFFSET`] and calls [`ds2_rva::NET_SESSION_BUSY`] -- so a row of
//! ours is locked exactly when the shipped Quit Game row is greyed for a session.
//!
//! The whole gate predicate is not called, on purpose: it refuses every nonzero gate when the
//! net-server manager is missing, which is the offline case, and that would lock the rows for a
//! player who is alone.
//!
//! If the predicate's prologue does not match, the check answers "no session" for the life of the
//! process and says so once. Rows that stay usable in a session are a visible, logged defect; rows
//! that lock for a solo player on a build this was not read from would be a menu that does nothing.

use std::sync::atomic::{AtomicUsize, Ordering};

use ds2_game_base::mem::safe_read_usize;

use crate::LOG_PREFIX;
use crate::install::log;

/// `bool busy(session)`, returned in `al`. `u8` so a stray byte cannot be an invalid `bool`.
type BusyFn = unsafe extern "system" fn(usize) -> u8;

/// The validated predicate's address, or `0` before [`install`] or after it refused.
static BUSY: AtomicUsize = AtomicUsize::new(0);

/// Validate [`ds2_rva::NET_SESSION_BUSY`]'s prologue. Returns whether the check is live.
pub(crate) fn install(base: usize) -> bool {
    let site = base + ds2_rva::NET_SESSION_BUSY as usize;
    let mut prologue = [0u8; ds2_rva::NET_SESSION_BUSY_PROLOGUE.len()];
    // SAFETY: a resolved RVA inside the loaded game image; `read_bytes` faults safely.
    let read = unsafe { ds2_game_base::mem::read_bytes(site, &mut prologue) };
    if !read || prologue != ds2_rva::NET_SESSION_BUSY_PROLOGUE {
        log(format_args!(
            "{LOG_PREFIX} session-lock NOT installed reason=prologue va=0x{site:016x} read={read} \
             saw={prologue:02x?} want={:02x?} -- added rows stay usable in multiplayer",
            ds2_rva::NET_SESSION_BUSY_PROLOGUE
        ));
        return false;
    }
    BUSY.store(site, Ordering::Release);
    log(format_args!(
        "{LOG_PREFIX} session-lock armed va=0x{site:016x} -- added rows are refused and greyed \
         while a multiplayer session is up"
    ));
    true
}

/// Whether a multiplayer session is up right now. **Game thread only.**
///
/// Four reads and one call into a function that only reads. `false` whenever any link in the chain
/// is missing -- no game manager, no network service, no session -- which is the gate's own answer
/// for a null session.
pub(crate) fn active() -> bool {
    let busy = BUSY.load(Ordering::Acquire);
    if busy == 0 {
        return false;
    }
    let Ok(manager_address) = ds2_game_base::mem::game_rva(ds2_rva::GAME_MANAGER_IMP) else {
        return false;
    };
    // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one.
    let session = unsafe {
        safe_read_usize(manager_address)
            .filter(|&manager| manager != 0)
            .and_then(|manager| safe_read_usize(manager + ds2_rva::NET_SERVICE_OFFSET))
            .filter(|&service| service != 0)
            .and_then(|service| safe_read_usize(service + ds2_rva::NET_SERVICE_SESSION_OFFSET))
    };
    let Some(session) = session.filter(|&session| session != 0) else {
        return false;
    };
    // SAFETY: the prologue matched the function `ds2-rva` transcribed, the signature is the one its
    // disassembly implements (one pointer in RCX, a byte back in AL), and `session` is the non-null
    // object the game's own gate hands it, read the same way. The function only reads.
    unsafe {
        let busy: BusyFn = std::mem::transmute::<usize, BusyFn>(busy);
        busy(session) != 0
    }
}
