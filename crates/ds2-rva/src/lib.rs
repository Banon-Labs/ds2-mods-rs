//! Every DS2 address this repo knows, and nothing else.
//!
//! # Why this is its own crate, and why it is empty
//!
//! In `../er-mods-rs` the address table lives inside `er-game-base` as `rva.rs`, next to
//! generic process utilities. That works there because the addresses were settled years
//! before the crate was split. Here they are not settled at all: **no reverse engineering has
//! been done yet**, and the assumption that DS2 resembles Elden Ring is exactly the one that
//! must not be allowed to leak into the substrate.
//!
//! So the boundary is enforced by the dependency graph rather than by discipline:
//! `ds2-game-base`, `ds2-hook`, `ds2-hotkey-config` and `ds2-safe-input` do **not** depend on
//! this crate and contain **no** game knowledge. Everything that claims to know something
//! about DARK SOULS II declares it here, where a wrong claim is one file to audit.
//!
//! # The engines are not the same
//!
//! DARK SOULS II shipped in 2014; Elden Ring in 2022. Between them FromSoftware replaced the
//! menu stack, the rendering backend and the object/reflection framework. Concretely, and
//! already verified against this build:
//!
//! - **D3D11, not D3D12.** `DarkSoulsII.exe` imports `d3d11.dll` and `dxgi.dll`.
//! - **No FD4.** The `FD4FileCap` / `DLString<wchar_t>` / DLIO virtual-root layouts that
//!   `er-game-base::filecap` walks postdate this engine.
//! - **No `fromsoftware-rs` bindings.** That workspace has `darksouls3`, `eldenring`,
//!   `nightreign` and `sekiro` members and no `darksouls2`. DS3 is the nearer relative; even
//!   it is a generation away.
//!
//! Do not port an Elden Ring offset into this file. Derive it, from the binary, and say where
//! it came from.
//!
//! # Address convention
//!
//! Every constant here is an **RVA** -- an offset from the image base, not a runtime address.
//! Add [`IMAGE_BASE`] to get the VA the disassembly shows.
//!
//! The authoritative artifact is `darksoulsii-deobf.bin` at the repo root, produced by
//! `../dearxan`'s `deobfuscate` example. It is a flat mapped image: **file offset == RVA**, so
//! a Ghidra address of `0x141234567` is byte `0x1234567` of that file. It is gitignored --
//! it is the copyrighted game binary. Regenerate it with:
//!
//! ```text
//! cargo run --release --manifest-path ../dearxan/Cargo.toml --example deobfuscate \
//!   --no-default-features --features rayon -- <DarkSoulsII.exe> darksoulsii-deobf.bin
//! ```
//!
//! # Recording an address
//!
//! One `pub const`, one doc comment, and the doc comment must say **how the address was
//! established** -- the function it was read out of, the xref count, the string that anchored
//! it. An address with no provenance is a guess wearing a type.

/// Preferred image base of `DarkSoulsII.exe` (`OptionalHeader.ImageBase`).
///
/// `DllCharacteristics` is `0x8160`, so `DYNAMIC_BASE` is set and the loader is free to
/// relocate. **Never assume the live base equals this value** -- resolve it from the loaded
/// module at runtime and add the RVAs below to that. This constant exists to translate
/// between the disassembly's VAs and the RVAs recorded here, nothing more.
pub const IMAGE_BASE: u64 = 0x1_4000_0000;

/// `OptionalHeader.SizeOfImage` -- the mapped size, and therefore the length of
/// `darksoulsii-deobf.bin`.
pub const SIZE_OF_IMAGE: u32 = 0x01d7_6000;

/// Steam application id for DARK SOULS II: Scholar of the First Sin.
pub const STEAM_APP_ID: u32 = 335300;

/// The build these addresses were derived from, as reported by `appmanifest_335300.acf`.
///
/// Every constant below is anchored to this build and to nothing else. A Steam update that
/// changes this value invalidates the whole table until each entry is re-derived.
pub const BUILD_ID: u32 = 9_527_516;

// ============================================================================================
// ADDRESSES
//
// One entry, and it is not a feature's address -- it is the subject of an EXPERIMENT. The
// question "does a MinHook detour survive Arxan in this game" gates every hooking feature this
// repo could ever have, and it cannot be answered without patching some specific function. So
// the first address here is the one chosen to be patched, and its doc comment carries the whole
// derivation, because a hook site picked badly makes the experiment fail for the wrong reason.
//
// Everything else is still empty and still honest. The next entries arrive with the first mod
// that needs one.
// ============================================================================================

/// **M1 hook site**: the function the Arxan-survival probe detours. RVA `0x00832e70`.
///
/// # What this address is for
///
/// It is not needed by any feature. It exists so that
/// [`ds2-loader`'s Arxan probe](https://github.com/Banon-Labs/ds2-mods-rs/blob/main/crates/ds2-loader/src/arxan_probe.rs)
/// can patch *something* and watch whether the patch survives. DS2 carries 48 Arxan stubs and
/// 286 Arxan-redirected functions; MinHook works by rewriting a function prologue in `.text`,
/// which is exactly the thing an integrity check looks for. Until a detour has been shown to
/// survive in this game, every plan in this repo that involves hooking is unproven. This
/// constant is the subject of that experiment and nothing else.
///
/// # How it was established
///
/// Measured from `darksoulsii-deobf.bin` (build [`BUILD_ID`]), statically, with no runtime:
///
/// * `.pdata` (`RUNTIME_FUNCTION[]` at RVA `0x189a000`, size `0x117978`, 12 bytes each) gives
///   all **95434** function starts for free. Counting `e8 rel32` call targets that land exactly
///   on one of those starts resolved **149022** direct calls without disassembling 17 MB.
/// * This function is the target of **2052** of them -- rank 3 in the whole binary. A detour
///   here that never fires is a real signal rather than an expected one.
/// * Its prologue is `48 89 5c 24 08` (`mov [rsp+8], rbx`), then `57`, then `48 83 ec 20`. The
///   first instruction is **exactly 5 bytes**, so MinHook relocates one whole instruction into
///   the trampoline and never has to split one -- the trivial case, and the reason this site
///   was preferred over an equally hot one with a 4-byte first instruction.
/// * It is `0x47` bytes long, so the 5-byte `e9 rel32` MinHook writes fits with room to spare,
///   and no branch inside the function targets a byte within those five.
/// * It is **not** one of the 286 Arxan-redirected functions.
///
/// See `docs/ARXAN-PROBE.md` for the experiment this feeds, and `docs/ARXAN-FOOTPRINT.md` for
/// the survey the counts above come from.
///
/// # Resolve it, do not hardcode the VA
///
/// The disassembly shows `0x140832e70`, but [`IMAGE_BASE`] is only the *preferred* base and
/// `DllCharacteristics` is `0x8160` (`DYNAMIC_BASE`). Add this RVA to the base read out of the
/// loaded module at runtime -- `ds2_game_base::mem::game_rva` does exactly that.
pub const ARXAN_PROBE_HOOK_SITE: u32 = 0x0083_2e70;

/// The bytes [`ARXAN_PROBE_HOOK_SITE`] is expected to begin with, before anything patches it.
///
/// The probe reads the live prologue at install time and compares it against this. A mismatch
/// means something else reached this function first -- another mod, an Arxan stub that this
/// build places differently, or the wrong game version -- and the probe declares the run VOID
/// rather than reporting on a patch it did not make cleanly. Five bytes because that is the
/// whole first instruction and the whole of what MinHook overwrites.
pub const ARXAN_PROBE_HOOK_SITE_PROLOGUE: [u8; 5] = [0x48, 0x89, 0x5c, 0x24, 0x08];

/// Length of [`ARXAN_PROBE_HOOK_SITE`], from its `.pdata` `RUNTIME_FUNCTION` entry.
///
/// Recorded because "is there room for a 5-byte jump" is the question a hook site has to answer,
/// and `0x47` answers it without anyone re-reading `.pdata`.
pub const ARXAN_PROBE_HOOK_SITE_LEN: u32 = 0x47;

/// Backup M1 hook site, if [`ARXAN_PROBE_HOOK_SITE`] turns out to be unusable. RVA `0x008389e0`.
///
/// Same derivation, same prologue shape (`48 89 5c 24 08 / 57 / 48 83 ec 20`), **1287** static
/// call sites, `0x4e` bytes long, also not one of the 286. It is the second choice only because
/// it is called less often; on every property that decides whether a hook can be installed it is
/// equivalent.
pub const ARXAN_PROBE_HOOK_SITE_BACKUP: u32 = 0x0083_89e0;

// ============================================================================================
// THE TITLE BOOT SCREENS (`ds2-mods-rs-3rr`)
//
// `FeStateTitle` drives a family of `FeSubState*` objects off a shared 8-slot vtable, whose
// slots are v0 destructor, v1 enter, v2 leave, v3 update, v4 unused, v5/v6 a debug-registration
// pair, v7 a bool query. Four of the eight are empty `ret` stubs in `FeSubStateBase`, so any
// override is that substate's real logic. Full trace: `docs/DS2-TITLE-FLOW.md`.
//
// EACH SUBSTATE KEEPS ITS OWN PHASE COUNTER, AND THE OFFSET IS NOT SHARED. Logo's is at `+0x20`;
// the other two are at `+0x10`. That is measured per class from the field each one's own `v3`
// switches on, not inferred from a common base -- assuming one offset for all three would write
// a 4 into an unrelated member of two of them.
//
// The terminal value IS 4 in all three, and in each case the game itself has a shipped path where
// `v1` writes exactly that and returns having done nothing else. That is what makes forcing it a
// reproduction of the game's own behaviour rather than an invented transition.
// ============================================================================================

/// `FeSubStateTitleLogo::v1` (enter). RVA `0x000fd980`, VA `0x1400fd980`.
///
/// The publisher-logo screen. Its own `v1` sets the phase to [`TITLE_SUBSTATE_PHASE_DONE`] and
/// returns immediately when the scene reference at `+0x18` is null -- the game's shipped "there
/// is no logo to show" path, and the one this mod reproduces.
///
/// Not Arxan-redirected: `scripts/ds2-arxan-chain.py 0x1400fd980` terminates at hop 0 with the
/// function's own prologue (`40 53 48 83 ec 20`, 6 relocatable bytes, comfortably over MinHook's
/// five).
pub const FE_SUBSTATE_TITLE_LOGO_ENTER: u32 = 0x000f_d980;

/// Phase-counter offset within `FeSubStateTitleLogo`. **`+0x20`, not `+0x10`.**
///
/// Read from `FeSubStateTitleLogo::v3` at `0x1400febf0`, which opens
/// `mov ecx, DWORD PTR [rcx+0x20]` and then a `dec`/`je` chain over phases 1, 2, 3.
pub const FE_SUBSTATE_TITLE_LOGO_PHASE_OFFSET: usize = 0x20;

/// `FeSubStateWarningNoCopy::v1` (enter). RVA `0x000fded0`.
///
/// The unauthorised-copying warning. Its `v1` calls a virtual at `+0x40` on the singleton at
/// `0x1416751f8` and, when that returns nonzero, writes phase 4 and returns -- so this screen
/// already skips itself under some condition the game knows about.
///
/// Not Arxan-redirected (hop 0, prologue `40 53 48 83 ec 20`).
pub const FE_SUBSTATE_WARNING_NO_COPY_ENTER: u32 = 0x000f_ded0;

/// `FeSubStateTitleUserPolicy::v1` (enter). RVA `0x000f9040`.
///
/// The user-policy screen. Its `v1` has two shipped early-outs, both reading persisted flags out
/// of the system-data object reached as `[[0x1416148f0]+0xa8]+0xd8`: `+0x136e` set means phase 3,
/// `+0x136d` set means phase 4. Those are the game's own "already accepted" flags, which is why
/// 4 is the terminal value here too.
///
/// Not Arxan-redirected (hop 0). Its prologue is `48 89 6c 24 20`, exactly five bytes -- MinHook's
/// minimum, met with nothing to spare.
pub const FE_SUBSTATE_TITLE_USER_POLICY_ENTER: u32 = 0x000f_9040;

/// Phase-counter offset within `FeSubStateWarningNoCopy` and `FeSubStateTitleUserPolicy`.
///
/// Read from each one's own `v3` (`0x1400ff360` and `0x1400f96f0`), both of which open
/// `mov edx, DWORD PTR [rcx+0x10]` before their `dec`/`je` chain.
pub const FE_SUBSTATE_PHASE_OFFSET: usize = 0x10;

/// The phase value that means "this substate is finished", for all three boot screens.
///
/// Reached by three independent paths in `FeSubStateTitleLogo` alone: normal completion at the end
/// of phase 3, the player's skip-button path at `0x1400fec6c`, and the null-scene path in `v1`.
/// A player pressing the skip button lands here every time and the flow advances, which is the
/// evidence that something consumes it -- the consumer itself has not been located and does not
/// need to be.
pub const TITLE_SUBSTATE_PHASE_DONE: u32 = 4;

/// **The redirected probe site**: `applySpEffect`, RVA `0x0014bec0`. Deliberately Arxan's.
///
/// [`ARXAN_PROBE_HOOK_SITE`] is a clean function, and that is exactly its limitation: walking it
/// with `scripts/ds2-arxan-chain.py` terminates at hop 0 because its own prologue is at its own
/// entry. Arxan has no presence there, so a detour on it can never provoke Arxan, and both arms
/// of `ds2-mods-rs-z6m` surviving was a null result by construction. This constant exists so the
/// experiment can be run somewhere Arxan actually is.
///
/// # Why this one
///
/// It is the only site found so far that is both genuinely redirected and functionally
/// load-bearing -- `ds2-mods-rs-a1g` needs it -- so one experiment answers both questions.
///
/// # Why hooking it is sound, despite [`ARXAN_REDIRECTED_DO_NOT_HOOK`] below
///
/// That exclusion says a detour over Arxan's redirect would break control flow. It does not,
/// and the reason is in MinHook's `trampoline.c`: for a relative `E9` whose destination lies
/// outside the five bytes being patched, MinHook does not copy the instruction, it emits a
/// `JMP_ABS` (`FF 25` + an absolute 8-byte address) to that destination and marks the trampoline
/// complete. So the trampoline jumps to `0x141b3cbe1`, Arxan's chain runs the stolen prologue,
/// and execution rejoins the original function at entry + `0x14` exactly as it would unhooked.
/// `oldPos` is 5, which meets MinHook's minimum, so the hook installs.
///
/// The two constants below stay excluded for a different reason: they are Arxan's own hot
/// dispatch functions rather than game functions that happen to be redirected.
///
/// # How it was established
///
/// `scripts/ds2-arxan-chain.py 0x14014bec0`, statically, no runtime: five hops -- the entry
/// `jmp`, two obfuscation thunks, then two fragments carrying 16 instructions of genuine stolen
/// prologue -- rejoining game `.text` at entry + `0x14`. Confirmed separately that no Arxan
/// encrypted region covers it (2969 regions examined, span `0x140001680`-`0x141cfa783`), so a
/// stub cannot silently decrypt original bytes back over the detour. See
/// `docs/ARXAN-FOOTPRINT.md`.
pub const ARXAN_PROBE_REDIRECTED_SITE: u32 = 0x0014_bec0;

/// The five bytes [`ARXAN_PROBE_REDIRECTED_SITE`] begins with: Arxan's own redirect.
///
/// `e9 1c 0d 9f 01` is `jmp 0x141b3cbe1`. The displacement is relative and both ends move
/// together, so these bytes are identical whatever base the image loads at -- no relocation
/// applies to them, which is what makes a fixed expectation valid here.
///
/// `neuter_arxan` does not rewrite this. Its patch set is a `JmpHook` at each stub's
/// `test rsp, 0xf` plus `Write`s of decrypted regions, of which DS2 gets zero. So both arms
/// install over byte-identical bytes and the comparison between them is meaningful.
pub const ARXAN_PROBE_REDIRECTED_SITE_PROLOGUE: [u8; 5] = [0xe9, 0x1c, 0x0d, 0x9f, 0x01];

/// **NEVER HOOK THESE.** The two hottest functions in the binary, and both are Arxan's.
///
/// `0x00832cb0` (12401 call sites) begins `e9 c1 50 34 01` -> `0x141b77d76`, and `0x00c2c9e0`
/// (4866 call sites) begins `e9 ba e7 f3 00` -> `0x141b6b19f`. Both jumps land in `.text` #2
/// (VA `0x141aaf000`-`0x141d43000`), Arxan's own section. They are recorded here as a named
/// exclusion rather than left out, because the next person ranking functions by call count will
/// find exactly these two at the top and needs to know why they are skipped.
///
/// Detouring one of them would mean writing over Arxan's own redirect. The experiment would then
/// fail -- or the game would crash -- for a reason that has nothing to do with the question being
/// asked, which is whether Arxan reverts an *ordinary* hook.
pub const ARXAN_REDIRECTED_DO_NOT_HOOK: [u32; 2] = [0x0083_2cb0, 0x00c2_c9e0];
