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

// ============================================================================================
// THE TITLE-FLOW MESSAGE DIALOGS (`ds2-mods-rs-j3b`)
//
// Skipping the three boot screens does not reach the title menu, because the flow then stops on
// message boxes that wait for a button. Six classes draw them, and ALL SIX SHARE ONE `update`:
// `FeSubStateCommonWindowBase::v3`. That is the whole reason this is three constants and not
// eighteen -- there is exactly one place where a dialog decides it is finished.
//
// WHAT THE SHARED UPDATE ACTUALLY DOES, read from `0x140105150`. On the frame a button is
// pressed, its ONLY effect is a store into the result byte at `+0x31`; the dispatch that follows
// reads that byte and nothing else about the press. So writing `+0x31` is not an approximation of
// a button press, it IS the button press, minus the polling. Every remaining constant below is a
// field that store depends on.
//
// The layout was read from the two functions that own it -- `v1` (enter) at `0x140104db0` and
// `v3` (update) at `0x140105150` -- and NOT inferred from the boot screens above. It is a
// different base class with a different layout: these dialogs keep their phase at `+0x30`, where
// `FeSubStateWarningNoCopy` keeps its at `+0x10` and `FeSubStateTitleLogo` keeps its at `+0x20`.
// ============================================================================================

/// `FeSubStateCommonWindowBase::v3` (update). RVA `0x00105150`, VA `0x140105150`.
///
/// The one function every message box in the title flow runs each frame, and the only place any
/// of them transitions. Its phase-1 branch polls for input, stores a result into
/// [`FE_DIALOG_RESULT_OFFSET`], and then dispatches on that byte through vtable slot 8 or 9.
///
/// Shared by all six classes whose `v3` slot holds this address: `FeSubStateCommonWindowBase`,
/// `FeSubStateCommonWindow`, `FeSubStateOfflineModeWindow`, `FeSubStateTitleOnlineCheckFailWarn`,
/// `FeSubStateTitleInformationFailWarn` and `FeSubStateTitleDeleteProfile`. Enumerated by scanning
/// every RTTI-described vtable in the image for that slot value, so the list is exhaustive for
/// this build rather than the ones that happened to be looked for.
///
/// # Why hooking it is sound
///
/// `scripts/ds2-arxan-chain.py 0x140105150` terminates at hop 0: **not** Arxan-redirected, its own
/// prologue is at its own entry. `.pdata` gives it length `0xed`, ample room for a five-byte
/// `e9`. Its first instruction is `48 89 5c 24 08` (`mov [rsp+8], rbx`) -- exactly five bytes, so
/// MinHook relocates one whole instruction and never has to split one. That is the same trivial
/// case as [`ARXAN_PROBE_HOOK_SITE`], which is already proven to install and fire in this game.
pub const FE_DIALOG_UPDATE: u32 = 0x0010_5150;

/// Phase byte within a common-window substate. `+0x30`.
///
/// `1` while the box is up and waiting, `2` while it plays its close animation, then `3` or `4`
/// once closed. `v1` (enter) ends with `mov WORD PTR [rdi+0x30], 1` -- a **16-bit** store that
/// sets this byte to 1 and [`FE_DIALOG_RESULT_OFFSET`] to 0 in one instruction, which is what
/// establishes that the two fields are adjacent and in this order.
pub const FE_DIALOG_PHASE_OFFSET: usize = 0x30;

/// Result byte within a common-window substate. `+0x31`.
///
/// `0` undecided, `1` the back/cancel answer, `2` the confirm answer. **This is the byte a button
/// press writes**, and the dispatch at `0x140105200` reads it, calls vtable slot 8 for `1` or slot
/// 9 for `2`, and sets the phase to 2. Writing it is how this mod presses the button.
pub const FE_DIALOG_RESULT_OFFSET: usize = 0x31;

/// Option-count field within a common-window substate. `+0x12`, and it is **signed**.
///
/// Negative means a one-button acknowledgement box: `v1` takes a different show call for it, and
/// `v3` forces the result to [`FE_DIALOG_RESULT_CANCEL`] on any press without ever consulting
/// which option is highlighted. Non-negative means a real choice, where the game produces
/// [`FE_DIALOG_RESULT_CONFIRM`] instead. Read at `0x1400fd3ba` in enter and `0x1401051ba` in
/// update, both as `cmp WORD PTR [.. +0x12], 0` followed by a signed branch.
///
/// This is what makes a synthesised answer match the game's own rather than merely resemble it:
/// the value to write is a function of this field, not a constant to pick.
pub const FE_DIALOG_OPTIONS_OFFSET: usize = 0x12;

/// [`FE_DIALOG_PHASE_OFFSET`] while the box is up and waiting for a button.
///
/// The only phase in which writing a result does anything: `v3` dispatches from phase 1 and
/// returns immediately in every other phase.
pub const FE_DIALOG_PHASE_WAITING: u8 = 1;

/// [`FE_DIALOG_RESULT_OFFSET`] before anything has been decided.
pub const FE_DIALOG_RESULT_NONE: u8 = 0;

/// The back/cancel answer, and the ONLY answer a one-button box can produce.
pub const FE_DIALOG_RESULT_CANCEL: u8 = 1;

/// The confirm answer, produced when a press lands on a box with real options.
pub const FE_DIALOG_RESULT_CONFIRM: u8 = 2;

/// Vtable slot the dispatch calls for [`FE_DIALOG_RESULT_CANCEL`]. `call [rax+0x40]` at
/// `0x140105212`, and `0x40 / 8 == 8`.
pub const FE_DIALOG_SLOT_ON_CANCEL: usize = 8;

/// Vtable slot the dispatch calls for [`FE_DIALOG_RESULT_CONFIRM`]. `call [rax+0x48]` at
/// `0x140105217`.
pub const FE_DIALOG_SLOT_ON_CONFIRM: usize = 9;

/// `FeSubStateCommonWindowBase`'s own slot-8 handler: `ret 0`, and nothing else. RVA `0x000f89d0`.
///
/// Recorded so a dialog's handlers can be checked to be inert **at runtime**, from its own vtable,
/// before this mod answers it. That check is the difference between "the allowlist below is
/// believed to be safe" and "this build's bytes say answering does nothing but close the box".
pub const FE_DIALOG_INERT_ON_CANCEL: u32 = 0x000f_89d0;

/// `FeSubStateCommonWindowBase`'s own slot-9 handler: `ret 0`. RVA `0x000f89c0`.
pub const FE_DIALOG_INERT_ON_CONFIRM: u32 = 0x000f_89c0;

/// `FeSubStateTitleOnlineCheckFailWarn`'s vtable. RVA `0x010bd7d8`, VA `0x1410bd7d8`.
///
/// The network-check failure box. Its `v1` and its slot-11 message getter both live in
/// `..\..\Source\Frontend\Operator\Title\FeSubStateServerFailWarn.cpp` -- the source path is still
/// in the image at `0x1410bd8b0` -- and its enter formats that path together with error code
/// `0x35b62` into the message it shows. Both its slot-8 and slot-9 handlers are the inert `ret 0`
/// above, so answering it closes it and does nothing else.
pub const FE_DIALOG_VTABLE_ONLINE_CHECK_FAIL_WARN: u32 = 0x010b_d7d8;

/// `FeSubStateTitleInformationFailWarn`'s vtable. RVA `0x010bd848`, VA `0x1410bd848`.
///
/// The sibling of [`FE_DIALOG_VTABLE_ONLINE_CHECK_FAIL_WARN`] from the same source file, formatting
/// error code `0x33453`. Its handlers are inert in exactly the same way.
pub const FE_DIALOG_VTABLE_INFORMATION_FAIL_WARN: u32 = 0x010b_d848;

/// `FeSubStateOfflineModeWindow`'s vtable. RVA `0x010bd388`, VA `0x1410bd388`.
///
/// The "playing offline" notice. Uses the shared `v1`, and overrides only the slot-11 message
/// getter (`0x1400f9800`) to pick text `0x67` or `0x65` from category `0x19` depending on the flag
/// at `[0x14160de10]+0x56b`. Handlers inert.
pub const FE_DIALOG_VTABLE_OFFLINE_MODE_WINDOW: u32 = 0x010b_d388;

/// **NOT in the allowlist, and recorded to say why.** `FeSubStateTitleDeleteProfile`'s vtable,
/// RVA `0x010bd6c8`.
///
/// It shares [`FE_DIALOG_UPDATE`] with the three above, so a mod that answered "every common
/// window" would answer this one too. It **overrides slot 8** with a real body at `0x1400fcf30`,
/// which reaches into the save-data object at `[[0x1416148f0]+0xa8]+0xd8`. Whatever that does, it
/// is not nothing, and it is reached only when a player deliberately chooses to delete a profile.
///
/// The runtime handler check against [`FE_DIALOG_INERT_ON_CANCEL`] rejects it on its own, without
/// consulting this constant -- which is the point. This entry exists so the exclusion is written
/// down rather than merely emergent.
pub const FE_DIALOG_VTABLE_DELETE_PROFILE_DO_NOT_ANSWER: u32 = 0x010b_d6c8;
