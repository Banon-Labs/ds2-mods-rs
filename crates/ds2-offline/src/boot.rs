//! The two-byte patch that takes the network substates off the boot path.
//!
//! `FeSubStateTitleUserPolicy`'s enter already has an offline path: when system-data byte
//! `+0x136e` is set it goes to `0x2a` (the "playing offline" notice) and then to `0x47` `TopMenu`,
//! and `0x38`, `0x39` `GameServerLogin` and `0x44` Information never run. This removes the `je`
//! that skips that path, so every boot takes it. The byte itself is left alone because the game
//! saves it; see [`ds2_rva::USER_POLICY_OFFLINE_BRANCH`].
//!
//! It is not a separate switch. It is applied whenever `[offline]` is on, because the login it
//! removes is the one thing this crate must never let reach FromSoftware's servers.

use crate::{LOG_PREFIX, install::log};

/// Replace the branch and read it back. True only when the bytes in memory are the stub.
///
/// The two bytes go in one at a time through [`ds2_hook::write_code_byte`], so there is an
/// instant where only one of them is written and the instruction is neither the `je` nor the
/// `nop`s. That is safe only because this runs from the loader's post-Arxan callback, before the
/// title flow exists and so before anything can execute `UserPolicy`'s enter.
///
/// # Safety
///
/// Writes two bytes of executable memory in the loaded game image. `base` must be the live base
/// of `DarkSoulsII.exe`, and no thread may be running `0x1400f9040`.
pub unsafe fn skip_login(base: usize) -> bool {
    let site = base + ds2_rva::USER_POLICY_OFFLINE_BRANCH as usize;
    let expected = ds2_rva::USER_POLICY_OFFLINE_BRANCH_EXPECTED;
    let stub = ds2_rva::USER_POLICY_OFFLINE_BRANCH_STUB;

    let mut found = [0u8; 2];
    // SAFETY: two bytes of the loaded image; `read_bytes` fails closed on an unmapped range.
    if !unsafe { ds2_game_base::mem::read_bytes(site, &mut found) } {
        log(format_args!(
            "{LOG_PREFIX} skip-login ABORT va=0x{site:016x} -- the branch could not be read"
        ));
        return false;
    }
    if found != expected {
        // A different build, or another mod owns this address. Writing `nop`s over bytes nobody
        // has read would land in the middle of some other instruction.
        log(format_args!(
            "{LOG_PREFIX} skip-login ABORT va=0x{site:016x} found={} expected={}",
            hex2(found),
            hex2(expected),
        ));
        return false;
    }
    for (offset, value) in stub.iter().enumerate() {
        // SAFETY: `site..site+2` was just read and holds the recorded `je`, and this function's
        // contract says nothing is executing it.
        if !unsafe { ds2_hook::write_code_byte(site + offset, *value) } {
            log(format_args!(
                "{LOG_PREFIX} skip-login VirtualProtect refused va=0x{:016x}",
                site + offset
            ));
            return false;
        }
    }
    let mut live = [0u8; 2];
    // SAFETY: the same two bytes, read back.
    let read = unsafe { ds2_game_base::mem::read_bytes(site, &mut live) };
    let landed = read && live == stub;
    log(format_args!(
        "{LOG_PREFIX} skip-login va=0x{site:016x} wrote={} live={} landed={landed}",
        hex2(stub),
        hex2(live),
    ));
    landed
}

/// Two bytes as `xx xx`, so a log line can be compared against the disassembly by eye.
fn hex2(bytes: [u8; 2]) -> String {
    format!("{:02x} {:02x}", bytes[0], bytes[1])
}
