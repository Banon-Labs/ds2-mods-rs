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

/// Scene-reference offset within `FeSubStateTitleLogo`. `+0x18`.
///
/// **Nulling this is what stops the logo animating**, and it is the game's own path rather than a
/// new one. `FeSubStateTitleLogo::v1` opens `mov rcx,[rcx+0x18]; test rcx,rcx; je` and the
/// not-taken branch at `0x1400fd9fb` is the whole of the shipped "there is no logo to show" case:
/// write phase 4, return. It plays no sequence and opens nothing.
///
/// The previous version of this crate let the original `enter` run and then wrote phase 4 after
/// it. That advances the flow but the sequence `enter` already started
/// (`0x140afdb80(scene, 0x66, ...)`, or `0x67` on the skip path) keeps playing, which is exactly
/// the logo animation that remained visible.
///
/// # Why nulling it is balanced at both ends
///
/// `FeSubStateTitleLogo::v2` (leave, `0x1400fe830`) is guarded on **this same pointer**:
/// `mov rcx,[rcx+0x18]; test rcx,rcx; je` and it closes only when non-null. So null at `enter` and
/// null at `leave` is precisely the pair the shipped path produces -- neither opens nor closes.
/// Restoring the pointer afterwards would instead produce a close with no matching open, which is
/// the unbalance this avoids.
///
/// **`enter` does not create the scene, it reads one already there**, so nulling the substate's
/// copy does not orphan an allocation this crate made; it declines to start and stop something
/// another object owns.
///
/// This trick does NOT transfer to the other two boot screens, and their `leave` implementations
/// are why: `FeSubStateWarningNoCopy::v2` (`0x1400febb0`) guards on a **global** at
/// `[0x14160de10]+0xf0` rather than on the substate, and `FeSubStateTitleUserPolicy::v2`
/// (`0x1400f96b0`) guards on **phase == 1**. Three classes, three different guards; each one had
/// to be read.
pub const FE_SUBSTATE_TITLE_LOGO_SCENE_OFFSET: usize = 0x18;

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

/// `FeSubStateCommonWindowBase::v1` (enter). RVA `0x00104db0`, VA `0x140104db0`.
///
/// **The one place a title message box is actually created**, and therefore the only place it can
/// be prevented rather than dismissed. It picks one of two show calls off the sign of
/// [`FE_DIALOG_OPTIONS_OFFSET`] -- `0x1404fe2a0` for a one-button box, `0x1404fe1c0` for a choice
/// -- and ends `mov DWORD PTR [rdi+0x18],0` / `mov WORD PTR [rdi+0x30],1`.
///
/// EVERY dialog class reaches it, including the two that override `v1`:
/// `FeSubStateTitleOnlineCheckFailWarn::v1` formats its message and then `call 0x140104db0` at
/// `0x1400fd471`, and `FeSubStateTitleInformationFailWarn::v1` does the same. So a single detour
/// here covers all six, where a detour on each class's own `v1` would need four.
///
/// # Why skipping it does not leak a window
///
/// `leave` (`0x1401050a0`) closes the window **only when the phase is 1**:
/// `cmp BYTE PTR [rcx+0x30],1` / `jne`, and the not-taken path merely zeroes the phase. So an
/// `enter` that never ran and never opened anything pairs with a `leave` that never closes
/// anything. That conditional is what makes suppression sound here and is exactly what
/// `ds2-intro-skip` did NOT have available -- its `leave` closes unconditionally, which is why
/// that crate lets the original `enter` run and only rewrites the phase afterwards.
///
/// Not Arxan-redirected; `.pdata` gives it RVA `0x104db0`-`0x104df4`, and its first instruction
/// `48 89 5c 24 20` is a whole five bytes.
pub const FE_DIALOG_ENTER: u32 = 0x0010_4db0;

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

/// [`FE_DIALOG_PHASE_OFFSET`] once the box has closed on a [`FE_DIALOG_RESULT_CANCEL`].
///
/// The update computes the closed phase as `sete al` on `result == 2` then `add al, 3`
/// (`0x14010518b`), so a cancel closes to `3` and a confirm to `4`. A one-button box can only ever
/// produce a cancel, which makes `3` **the only terminal phase reachable for one** -- writing it
/// is reproducing the single outcome the game has, not choosing between two.
pub const FE_DIALOG_PHASE_CLOSED_CANCEL: u8 = 3;

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

/// `FeSubStateCommonWindow`'s vtable. RVA `0x010bcff8`, VA `0x1410bcff8`.
///
/// **The dialog that actually appears at boot**, measured: a first run with only the three
/// `FailWarn`/`OfflineMode` vtables allowlisted logged
/// `seen screen=<not-allowlisted> vtable=0x00000001410bcff8` and nothing else, while the three
/// that *were* allowlisted never fired. The generic-sounding name is why it was left out at
/// first, and reading it is what put it back in.
///
/// # Why answering the "generic" window is nonetheless safe here
///
/// Three facts, each read from the image rather than assumed:
///
/// 1. **There is exactly one of these objects in the game.** `scripts/ds2-xrefs.py 0x1410bcff8`
///    finds a single code reference in the whole image, the `lea r13` at `0x1400f75c1`, and it is
///    inside `FeStateTitle::v6` (`0x1400f72e0`) -- the routine that builds the title's substate
///    table, an 88-slot array at `[state+8]` with its count at `[state+0x2c8]`. So this class is
///    not "the generic box used all over the game"; it is one member of the title flow, and no
///    in-game prompt can be an instance of it.
/// 2. **It is a one-button acknowledgement box, by construction.** Its constructor
///    `0x140104c00` executes `or eax,0xffffffff` then `mov WORD PTR [rcx+0x12],ax`, so
///    [`FE_DIALOG_OPTIONS_OFFSET`] is hardcoded to `-1`. The update's input path can therefore
///    only ever produce [`FE_DIALOG_RESULT_CANCEL`] for it -- there is no second answer to get
///    wrong, and no choice being made on the player's behalf.
/// 3. **Its handlers are inert**, like the others: slots 8 and 9 are the base `ret 0` stubs.
///
/// Its message comes from category `0x19` id `0x1adc0` (`0x1400f7590`), its caption id is `0x20`
/// and its kind field at `+0x0c` is `6`.
pub const FE_DIALOG_VTABLE_COMMON_WINDOW: u32 = 0x010b_cff8;

/// Kind field within a common-window substate. `+0x0c`, set from the constructor's second
/// argument. Logged as a diagnostic; nothing branches on it here.
pub const FE_DIALOG_KIND_OFFSET: usize = 0x0c;

/// Caption/message id within a common-window substate. `+0x10`, a WORD, set from the
/// constructor's third argument and republished by `v5` at `0x140104f69`.
pub const FE_DIALOG_CAPTION_OFFSET: usize = 0x10;

/// Elapsed-time accumulator within a common-window substate. `+0x18`, a float.
///
/// `enter` zeroes it (`mov DWORD PTR [rdi+0x18],0` at `0x140104e85`) and the update accumulates the
/// frame delta into it (`addss xmm1,[rcx+0x18]`). Only read while the box is open.
pub const FE_DIALOG_ELAPSED_OFFSET: usize = 0x18;

/// Auto-close timeout within a common-window substate. `+0x14`, a float, `0` meaning none.
///
/// Read by the update at `0x1401051e5`. A dialog with a positive value here closes itself with no
/// press at all, which is what makes "close without a press" a shipped behaviour rather than one
/// this mod invented. Logged as a diagnostic.
pub const FE_DIALOG_TIMEOUT_OFFSET: usize = 0x14;

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

// ============================================================================================
// THE TWO REMAINING STOPS BETWEEN BOOT AND THE MENU (`ds2-mods-rs-j3b`)
//
// Suppressing the notice boxes still does not hand the player a menu. Two things are left, and
// they are different in kind from each other and from the notices:
//
//   * a PRESS ANY BUTTON gate, which waits on input and is the only one of the three that a
//     player must actually act on;
//   * PROCESS WINDOWS -- "please wait" boxes that resolve on their own, but not before a minimum
//     display time has elapsed. These wrap real asynchronous work and MUST NOT be skipped; only
//     the artificial minimum is worth removing.
// ============================================================================================

/// The press-any-button poll. RVA `0x000ff420`, VA `0x1400ff420`.
///
/// **It has exactly one caller in the entire image**: `0x1400fee6b`, inside
/// `FeSubStateTitleMain::v3` (`0x1400fed90`), which is the "PRESS ANY BUTTON" screen's update.
/// Counted by scanning every `e8 rel32` in the image for this target and attributing each hit to
/// its `.pdata` owner. That is what makes detouring it narrow: it is a private helper of one gate,
/// not shared input plumbing, so forcing its result cannot reach anything else in the game.
///
/// It ignores its argument entirely and reads globals -- the singleton at `0x1416751f8`, whose
/// `+0x60` object it passes to `0x140af3f30`, then bit 16 and bit 4 of the returned `+0x10` word,
/// falling back to `[[+0x60]+8]+0x34 & 1`. That is the same "a button was pressed" state word
/// `FeSubStateTitleLogo`'s skip path tests; see `docs/DS2-TITLE-FLOW.md`.
///
/// # Why forcing it true is the right cut
///
/// `FeSubStateTitleMain::v3`'s phase-1 branch runs three things in order: it ticks the title scene,
/// waits for [`FE_TITLE_MAIN_SEQUENCE_GATE`] to report the title sequence is up, and only then
/// consults this poll. Forcing this one true leaves the sequence gate intact -- the title screen
/// still initialises normally -- and then runs the **whole** of the game's own phase-1 body, which
/// is what prepares the top menu. Forcing the substate's terminal phase instead would skip that
/// setup, which is why this is hooked and the phase is not.
///
/// Not Arxan-redirected. `.pdata` gives it RVA `0x0ff420`-`0x0ff465`; its first two instructions
/// are `48 83 ec 28` and `48 8b 49 08`, eight relocatable bytes with no branch targeting them.
pub const FE_TITLE_MAIN_PRESS_ANY_BUTTON: u32 = 0x000f_f420;

/// The gate that must still pass before the press poll is consulted. RVA `0x000f37f0`.
///
/// Returns true when the title scene's currently-playing sequence is `0x67`, via
/// `0x140afdb30(scene, 0x67)` -- which compares the active sequence id against its argument.
/// Recorded so it is clear that [`FE_TITLE_MAIN_PRESS_ANY_BUTTON`] is NOT the only condition on
/// that branch, and that this mod deliberately leaves the other one alone.
pub const FE_TITLE_MAIN_SEQUENCE_GATE: u32 = 0x000f_37f0;

/// `FeSubStateProcessWindowBase::v1` (enter). RVA `0x00104ed0`, VA `0x140104ed0`.
///
/// The "please wait" window shared by six classes -- `FeSubStateProcessWindowBase`,
/// `FeSubStateProcessWindowSimple`, `FeSubStateTitleOnlineCheck`,
/// `FeSubStateTitleGameServerLogin`, `FeSubStateTitleSaveSystemData` and
/// `FeSubStateTitleLoadProfile` -- found by scanning every RTTI vtable for this value in slot 1.
///
/// ```text
/// result = this->vtable[8]();          // STARTS THE ASYNCHRONOUS WORK
/// if (result >= 0) { this->phase = 3; return; }   // nothing to do; no window at all
/// show_process_window(ui, this->caption, 0, 1);
/// this->timer = 0;
/// this->phase = 1;
/// ```
///
/// **THIS ONE MUST NOT BE SUPPRESSED.** Slot 8 starts real work -- a network check, a server
/// login, a system-data save, a profile load -- and the update then waits on slot 10 for it to
/// finish. Skipping the substate would skip the wait, not just the window. That is the whole
/// reason this is treated differently from [`FE_DIALOG_ENTER`], where nothing was pending.
///
/// Not Arxan-redirected; `.pdata` RVA `0x104ed0`-`0x104f24`, prologue `40 53 48 83 ec 20`.
pub const FE_PROCESS_WINDOW_ENTER: u32 = 0x0010_4ed0;

/// Minimum display duration within a process-window substate. `+0x10`, a float, in seconds.
///
/// `FeSubStateProcessWindowBase::v3` (`0x140105270`) phase 1 reads it as
/// `addss xmm1,[rcx+0x14]` / `comiss xmm1,[rcx+0x10]` / `jb` -- so while the timer is BELOW this
/// value the window stays up no matter what, and only once it is reached does the update consult
/// slot 10 to ask whether the work is actually done. Set by the constructor at `0x140104c77`
/// (`movss [rcx+0x10],xmm3`) from its third argument.
///
/// **Zeroing it removes the artificial floor and nothing else.** The slot-10 wait is untouched, so
/// the window still stays up for exactly as long as the operation really takes -- it can no longer
/// linger after the work is finished, and it cannot outrun it either. That is why this is the cut
/// rather than anything that touches the phase.
pub const FE_PROCESS_WINDOW_MIN_DURATION_OFFSET: usize = 0x10;

/// Elapsed timer within a process-window substate. `+0x14`, a float, zeroed by `enter`.
pub const FE_PROCESS_WINDOW_TIMER_OFFSET: usize = 0x14;

/// Phase field within a process-window substate. `+0x20`, a **DWORD** -- not the byte the
/// common-window substates use at `+0x30`. Different base class, different layout.
///
/// `1` while the window is up, `2` while it closes, `3` once finished. `enter` sets it to `3`
/// directly when slot 8 reports there was nothing to do, which is the game's own "no window"
/// path.
pub const FE_PROCESS_WINDOW_PHASE_OFFSET: usize = 0x20;

/// [`FE_PROCESS_WINDOW_PHASE_OFFSET`] while the window is up and the work is outstanding. The
/// only phase in which the minimum duration is read, and therefore the only one in which zeroing
/// it does anything.
pub const FE_PROCESS_WINDOW_PHASE_SHOWING: i32 = 1;

/// Kind field within a process-window substate. `+0x0c`, set by the constructor at `0x140104c87`.
/// Logged as a diagnostic so the boot windows can be told apart in a log.
pub const FE_PROCESS_WINDOW_KIND_OFFSET: usize = 0x0c;

// --------------------------------------------------------------------------------------------
// Suppressing the process window outright, and the title screen's activation animation.
//
// Both go one step further than the two constants above, and both are recorded separately from
// them so either can be switched off on its own.
// --------------------------------------------------------------------------------------------

/// Vtable slot a process window calls from `enter` to START its asynchronous work.
/// `call [rax+0x40]` at `0x140104edc`, and `0x40 / 8 == 8`.
///
/// **This call is the reason a process window cannot simply be suppressed.** It returns a status
/// into [`FE_PROCESS_WINDOW_RESULT_OFFSET`]; a negative value means work is outstanding and the
/// window is shown, a non-negative one means there was nothing to do and the substate goes
/// straight to [`FE_PROCESS_WINDOW_PHASE_DONE`] with no window at all. Any code that hides the
/// window must still make this call and still honour that branch.
pub const FE_PROCESS_WINDOW_SLOT_BEGIN: usize = 8;

/// Where a process window stores the status returned by [`FE_PROCESS_WINDOW_SLOT_BEGIN`]. `+0x24`.
/// Written by `enter` at `0x140104edf` and again by the update at `0x1401052b2`.
pub const FE_PROCESS_WINDOW_RESULT_OFFSET: usize = 0x24;

/// [`FE_PROCESS_WINDOW_PHASE_OFFSET`] once the substate is finished. `3`.
///
/// `enter` writes it directly at `0x140104f17` when there was no work to do -- the game's own
/// no-window path, and the precedent for reaching this phase without ever showing anything.
pub const FE_PROCESS_WINDOW_PHASE_DONE: i32 = 3;

/// `FeSubStateTitleMain::v3` (update). RVA `0x000fed90`, VA `0x1400fed90`.
///
/// The PRESS ANY BUTTON screen's per-frame logic, switching on a phase at
/// [`FE_TITLE_MAIN_PHASE_OFFSET`]:
///
/// | phase | what it does |
/// | --- | --- |
/// | 1 | ticks the scene, waits for [`FE_TITLE_MAIN_SEQUENCE_GATE`], then for a press. On a press it runs the whole top-menu setup and leaves phase 2 (or 3). With no press for long enough it goes to phase **5** -- the attract-mode prologue movie. |
/// | 2 | pure wait on a sequence handle at `+0x38`; sets phase 3 and does nothing else |
/// | 3 | waits for the same sequence to finish, calls `0x140afe8a0` on it, sets phase 4 |
/// | 4 | terminal |
///
/// **Phases 2 and 3 are the activation animation** -- the flourish a player sees after pressing.
/// Phase 1's body is the part that matters, and it has already run by the time either is reached,
/// which is what makes forcing phase 4 from 2 or 3 a skip of the animation rather than of the
/// setup. Compare `ds2-intro-skip`, which forces a terminal phase from `enter`; that is not
/// available here, because from `enter` the setup has not happened yet.
///
/// Not Arxan-redirected. Prologue `48 89 5c 24 18` -- five bytes exactly.
pub const FE_TITLE_MAIN_UPDATE: u32 = 0x000f_ed90;

/// Phase field within `FeSubStateTitleMain`. `+0x10`, a DWORD.
///
/// Read as `mov ecx,[rcx+0x10]` at `0x1400fedb2` before its `dec`/`je` chain. Same offset as the
/// boot screens' phase, and a different one from either window family -- which is why each class's
/// offset in this file is derived from that class's own code and never shared by analogy.
pub const FE_TITLE_MAIN_PHASE_OFFSET: usize = 0x10;

/// The first activation-animation phase. Written by phase 1's tail at `0x1400feee8`.
pub const FE_TITLE_MAIN_PHASE_ANIMATING: i32 = 2;

/// The second activation-animation phase. Phase 1's tail can jump straight here at `0x1400feefa`
/// when the object it just built reports `[+8] == 0`, so BOTH values mean "the setup is done and
/// only the flourish is left".
pub const FE_TITLE_MAIN_PHASE_ANIMATING_LATE: i32 = 3;

/// Terminal phase for `FeSubStateTitleMain`. Written by phase 3 at `0x1400fedf7`.
pub const FE_TITLE_MAIN_PHASE_DONE: i32 = 4;

/// `show_process_window`. RVA `0x004fe760`, VA `0x1404fe760`.
///
/// **The single function that draws a "please wait" box**, and the only place all of them meet.
/// Hooking it is what makes hiding them general, and the alternative was shown to be a losing
/// game: the seven call sites are spread across different vtable slots of different classes, and
/// `FeSubStateTitleInformation` -- the "Retrieving Information" box -- shows its window from its
/// `update` (`v3`, via the continuation chunk at `0x1400ff98e`) rather than from `enter`, so no
/// amount of hooking `enter` reaches it.
///
/// # Its signature, established rather than assumed
///
/// Four register arguments and **no stack arguments**: the body reads nothing above its own frame.
/// It keeps RCX and forwards RDX, R8 and R9 untouched into `0x1405105f0`, which is why a detour
/// must carry all four even though the function appears to use only the first. All seven call
/// sites were checked and set exactly these four -- six do `mov r9b,1; xor r8d,r8d`, and
/// `0x1401088ae` does the mirror `xor r9d,r9d; mov r8b,1`. None writes a fifth at `[rsp+0x20]`.
/// Forwarding them as raw 64-bit registers reproduces even the upper bits the callers leave
/// undefined.
///
/// # Returning zero is the function's own no-op answer
///
/// It opens with `mov rcx,[rbx+0xf0]; test rcx,rcx; jne`, and the not-taken path is
/// `xor eax,eax; ret` -- "there is no window manager, so nothing was shown". **No caller uses the
/// return value**; all seven ignore EAX and immediately write their own phase field. So a detour
/// that returns 0 without drawing is indistinguishable from the shipped path where there was
/// nothing to draw on.
pub const FE_SHOW_PROCESS_WINDOW: u32 = 0x004f_e760;

/// The byte that is nonzero while `FeOperatorTitle` is running. RVA `0x01614804`, VA
/// `0x141614804`.
///
/// Written `1` by `FeOperatorTitle::v2` at `0x1400ef045` and `0` by `FeOperatorTitle::v3` at
/// `0x1400ef123`, which are the operator's setup and teardown. The game reads it itself at
/// `0x140342251` (`cmp BYTE PTR [rip+...],0`), so it is a real state flag rather than a
/// write-only leftover -- `scripts/ds2-xrefs.py` finds no other genuine reference.
///
/// **This is what scopes the process-window hiding to the title flow.** Hiding every process
/// window in the game would take the "Saving..." indicator with it, which is exactly the kind of
/// thing a player is entitled to see. Gating on the game's own "am I in the title flow" flag keeps
/// the change to the boot sequence and leaves gameplay alone, without this mod having to invent a
/// notion of "still booting" or time-box one.
pub const FE_OPERATOR_TITLE_ACTIVE: u32 = 0x0161_4804;

/// Sequence handle within `FeSubStateTitleMain`. `+0x38`.
///
/// Set up by phase 1's press-taken body (`0x1400feec8`, `lea rcx,[rbx+0x38]; call 0x14005a8e0`),
/// which starts the title-text sequence, and then waited on by phases 2 and 3.
pub const FE_TITLE_MAIN_SEQUENCE_HANDLE_OFFSET: usize = 0x38;

/// **NOT a "finish sequence" call, despite how its call sites read.** RVA `0x00afe8a0`.
///
/// `FeSubStateTitleMain::v3` phase 1 calls it on the substate's `+0x18` handle the moment a press
/// is taken, and phase 3 calls it on `+0x38` immediately before writing the terminal phase. Both
/// placements make it look like "stop the animation, we are done" -- and that reading was wrong.
///
/// Its body tail-calls `0x1409d5610`, which compares `[handle]` against the global at `0x14166df98`
/// and, when they differ, builds a record tagged `0x4d4f4d53` ("SMOM") and hands it to
/// `0x1409ebea0`. That is handle validation or telemetry, not playback control.
///
/// Recorded as an exclusion rather than deleted because the mistake is re-derivable: anyone reading
/// phases 1 and 3 will reach the same wrong conclusion from the call sites alone. It was caught by
/// a live run where it returned success and the title text animated in exactly as before.
pub const FE_SEQUENCE_NOT_A_FINISH_DO_NOT_USE: u32 = 0x00af_e8a0;

// --------------------------------------------------------------------------------------------
// NAMING THE UI ANIMATION PLAYER (`ds2-mods-rs-j3b`, still open)
//
// The title text animates in and the lever for it is not in the title substate. Both routes into
// the sequence system are thin forwarders that end in a virtual call, and no `Fe*Sequence` class
// exists among the 5269 RTTI names to identify the callee from:
//
//   0x140afdb80(scene, id, ..)  ->  rcx = [scene+0x28]      ; jmp 0x140b50860
//   0x140b50860                 ->  rcx = [rcx+0x30]        ; jmp [[rcx] + 0xc0]
//
// So the class is resolved the other way round: read the live vptr at the end of that chain and
// match it against the RTTI vtable map. That is a measurement, and it replaces the inference that
// produced FE_SEQUENCE_NOT_A_FINISH_DO_NOT_USE above.
// --------------------------------------------------------------------------------------------

/// The global holding the title-flow object table. RVA `0x0160de10`, VA `0x14160de10`.
///
/// Already load-bearing elsewhere in the boot flow: `FeSubStateTitleLogo`'s skip path writes its
/// `+0x568`, `FeSubStateTitleInitBranch` writes its `+0x564`, and `FeSubStateOfflineModeWindow`
/// reads its `+0x56b`.
pub const FE_TITLE_GLOBALS: u32 = 0x0160_de10;

/// Offset of the title scene within [`FE_TITLE_GLOBALS`]. `+0x80`.
///
/// `FeSubStateTitleMain::v3` loads it at `0x1400fedc9` (`mov rcx,[rax+0x80]`) and ticks it through
/// a virtual before consulting either of its gates.
pub const FE_TITLE_SCENE_OFFSET: usize = 0x80;

/// First hop of the sequence-player chain: `[scene + 0x28]`, read by `0x140afdb80`.
pub const FE_SEQUENCE_PLAYER_HOP1: usize = 0x28;

/// Second hop: `[hop1 + 0x30]`, read by `0x140b50860`, whose vtable is then dispatched through.
pub const FE_SEQUENCE_PLAYER_HOP2: usize = 0x30;

/// The vtable slot `0x140b50860` dispatches to: `jmp [rax+0xc0]`, and `0xc0 / 8 == 24`.
pub const FE_SEQUENCE_PLAYER_PLAY_SLOT: usize = 24;

/// Put `FeSceneTitle` into its settled state by playing sequence `0x67`. RVA `0x000f3820`.
///
/// `FeSubStateTitleMain::v1` calls `0x1400f3e30` (`0x1400fda54`), which plays sequence **`0x66`**
/// on `[scene+8]` -- the "DARK SOULS II SCHOLAR OF THE FIRST SIN" text animating in -- and nothing
/// in the phase machine stops it. `0x1400f3820` plays **`0x67`** on the same object, the settled
/// state, and it is exactly the sequence [`FE_TITLE_MAIN_SEQUENCE_GATE`] waits to observe before a
/// press is accepted:
///
/// ```text
/// if ([scene+0xf1] != 0) return;
/// rcx = [scene+8];
/// if (!rcx) return;
/// [rcx+0x18]--;
/// play(rcx, 0x67, 0, 0.0f);
/// ```
///
/// # Why this rather than forcing the gate alone
///
/// Forcing [`FE_TITLE_MAIN_SEQUENCE_GATE`] makes the gate report a state the scene is not in: the
/// press is taken early while `0x66` keeps animating underneath. Playing `0x67` puts the scene in
/// the state the gate is waiting for, so the flow reaches an interactive menu **as soon as the data
/// is available rather than pacing itself to an animation** -- which is the behaviour this is kept
/// for, confirmed in-game.
///
/// The four sequence ids used across the Fe scenes are `0x65`, `0x66`, `0x67` and `0x68`, read from
/// the 91 call sites of the play forwarder `0x140afdb80`, with `0x66`/`0x68` the in and out
/// transitions and `0x67` the settled state -- corroborated by `FeSubStateTitleLogo` using the same
/// set.
///
/// **Open:** the title text is still seen animating. Whether that is `0x66` continuing in parallel,
/// `0x67` carrying its own entry animation, or a different object entirely is unresolved; see
/// `docs/DS2-TITLE-FLOW.md`. That is a question about the remaining animation, NOT a reason to drop
/// this call, whose effect on when the menu becomes usable is real.
pub const FE_SCENE_TITLE_PLAY_IDLE: u32 = 0x000f_3820;

// --------------------------------------------------------------------------------------------
// The title top menu. `docs/DS2-TITLE-FLOW.md` carries the trace these came from.
//
// The menu is a fixed vector of six rows, rebuilt from scratch on demand. Nothing is ever
// inserted or removed -- `0x1400f4250` appends the same six descriptors on every path, and the
// only per-row variable is one byte. That byte decides two independent things, and separating
// them is what the constants below exist for.
// --------------------------------------------------------------------------------------------

/// `FeGroupTitleTopMenu`'s enable-and-style pass. RVA `0x000f5000`, VA `0x1400f5000`.
///
/// Called from `FeGroupTitleTopMenu::v25` (`0x1400f4df0`) with the group and the freshly built
/// descriptor list. Read from the disassembly rather than the decompiler, which drops the third
/// argument on one of the two branches:
///
/// ```text
/// for i in 0 .. list[+0x158]:
///     cell = FE_TOP_MENU_CELL_FOR_INDEX(group, i)      # null is skipped
///     desc = list + align + i*FE_TOP_MENU_ROW_STRIDE
///     tmp  = 0x140026790(group+0x100, &scratch, desc)  # RCX/RDX/R8 set BEFORE the branch
///     if desc[+0x34] != 0:
///         tmp[+0x40]->vtable[0](tmp+0x40, 0x67, 0, 0.0)
///         cell[+8] = 3 + (i == group[+0x28])
///     else:
///         cell[+8] = 2
///         tmp[+0x40]->vtable[0](tmp+0x40, 0x7a, 0, 0.0)
/// ```
///
/// **The two effects of "disabled" are separate writes.** `cell[+8] = 2` is what removes the row
/// from cursor navigation; the sequence swap is the entire visual difference. Nothing in the image
/// reads `cell[+8] == 2` to decide how to draw -- the only readers compare against 3 and 4 -- so
/// the appearance of an unavailable row is decided solely by sequence `0x7a`.
///
/// Not Arxan-redirected: `48 8b c4 48 89 58 18` is an ordinary MSVC prologue, seven bytes before
/// the first instruction boundary past five.
pub const FE_TOP_MENU_APPLY_STATES: u32 = 0x000f_5000;

/// `FeSubStateTitleTopMenu::v3` (update). RVA `0x000ff300`, VA `0x1400ff300`.
///
/// Runs every frame while the top menu is the active substate: it ticks the scene through a
/// virtual, copies `[scene+0xe8]` into its own phase, and clears a save-data flag on phase 4. It
/// is the only per-frame function that is specific to this menu, which is what makes it the place
/// to re-assert per-row state without reaching groups this mod has no business touching.
///
/// Takes the frame delta in XMM1 like the rest of the family, even though this member never reads
/// it. A detour that declares only `this` is free to clobber XMM1.
///
/// Not Arxan-redirected; prologue `48 89 5c 24 08`, five bytes exactly.
pub const FE_TOP_MENU_UPDATE: u32 = 0x000f_f300;

/// `FexGroupList<FeGroupGrid>`'s cell-by-index lookup. RVA `0x00108060`, VA `0x140108060`.
///
/// `longlong lookup(group, int index)` -- walks the group's cell list through two of its own
/// virtuals and returns the cell whose `[+0x10]` equals `index`, or null. Called, never patched,
/// so its Arxan status does not arise.
pub const FE_TOP_MENU_CELL_FOR_INDEX: u32 = 0x0010_8060;

/// Offset of `FeGroupTitleTopMenu` within `FeSceneTitle`. `+0xb8`.
///
/// Written by the scene's own builder at `0x1400f4950`: `*(longlong **)(param_1 + 0xb8) = plVar3`,
/// immediately after constructing the group with `0x1400f3250`.
pub const FE_TOP_MENU_GROUP_OFFSET: usize = 0xb8;

/// Stride of one row descriptor in the top-menu list. `0x38`.
pub const FE_TOP_MENU_ROW_STRIDE: usize = 0x38;

/// The row's action id, within a descriptor. `+0x30`, a DWORD, values 1 to 6.
///
/// Read by the activate handler at `0x1400f4a8d` as `[buffer + align + cursor*0x38]`.
pub const FE_TOP_MENU_ROW_ACTION_OFFSET: usize = 0x30;

/// The row's enabled flag, within a descriptor. `+0x34`, one byte.
///
/// Tested at `0x1400f5063`. Its six values, in row order, are the whole of the menu's variability:
/// `1`, has-a-save, online-available, `!online-available`, `1`, `1`.
pub const FE_TOP_MENU_ROW_ENABLED_OFFSET: usize = 0x34;

/// Row count within the descriptor list. `+0x158`.
///
/// Written as a QWORD by the builder and read as a DWORD by the styling pass, which is why it is
/// read as a 32-bit value here.
pub const FE_TOP_MENU_LIST_COUNT_OFFSET: usize = 0x158;

/// Capacity of the descriptor list. `6`, and also the count on every path.
///
/// The builder's own `DLFixedVector` bound: appending a seventh row calls
/// `DLKR::DLBackAllocator::panic` with `"out of memory."`. Used as a sanity bound on a count read
/// out of game memory, so a garbage read cannot drive a loop.
pub const FE_TOP_MENU_ROW_CAPACITY: usize = 6;

/// The cell's state field. `+0x8`, a DWORD, on `FeObjectButtonEx`.
pub const FE_BUTTON_STATE_OFFSET: usize = 0x8;

/// Cell state meaning "unavailable". `2`.
///
/// **This is what removes a row from cursor navigation.** `FeObjectButtonEx::v16` at
/// `0x14004c5c0` is the predicate the navigation search at `0x140107b40` calls on every candidate
/// before accepting it, on all six of its direction branches and again at the shared accept point
/// `0x140107fb0`:
///
/// ```text
/// vtable[3]() == 1 && [rcx+8] == 3
/// ```
///
/// So only state 3 is selectable. The activate handler `0x1400f4a60` does no enable check of its
/// own, which is exactly why this one has to hold.
pub const FE_BUTTON_STATE_UNAVAILABLE: i32 = 2;

/// Cell state meaning "available, not under the cursor". `3`.
pub const FE_BUTTON_STATE_NORMAL: i32 = 3;

/// Cell state meaning "under the cursor". `4`. `FeObjectButtonEx::v1` (`0x14004c5b0`) is
/// `cmp [rcx+8],4; sete al; ret`.
pub const FE_BUTTON_STATE_CURSOR: i32 = 4;

/// Which top-menu rows may be forced visible when the game would hide them. **Row 1 only.**
///
/// The six enable expressions are `1`, has-a-save, online, `!online`, `1`, `1`. Rows 0, 4 and 5 are
/// literal `1` and can never be hidden, so forcing them is meaningless. That leaves three rows the
/// game can delete, and exactly one of them is worth keeping on screen:
///
/// * **row 1, LOAD GAME** -- deleted until a save exists, and it owns its screen slot outright.
///
/// **Rows 2 and 3 -- INFORMATION and GO ONLINE -- are deliberately out, and the reason is not that
/// they look wrong when forced. It is that the pair needs no help.** Their enable bytes are
/// `online` and `!online`, computed from one another at `0x1400f4344`, so their XOR is true at
/// every instant: one of the two is always live, and the slot they share is never empty on its own.
/// The game swaps which one occupies it when `FeSubStateTitleOnlineCheck` sets the flag. Forcing
/// either is not "showing a row the game hides", it is putting a second label in an occupied slot,
/// which is exactly what the all-six run drew on top of itself. The grid coordinates are not the
/// reason -- a run logged them as `0:(0,0) 1:(0,1) 2:(0,2) 3:(0,3) 4:(0,4) 5:(0,5)`, six distinct
/// positions -- so the collision is in the layout, which places the mutually-exclusive pair
/// together.
///
/// A forced row 2 also draws WRONG, and the mechanism is worth recording because it governs any
/// future attempt to restyle a row. A sequence play is vtable slot `0xc0`, and it is not uniform:
/// `FeComponentObject` (`0x1411ddfa8`, `0x140b6a980`) forwards to every child, `FeComponentSprite`
/// (`0x1411de318`, `0x140b6c4f0`) is the only leaf class that acts on it, and `FeComponentTextField`
/// (`0x1411de698`), `FeComponentTextureShape`, `FeComponentMaskShape`, `FeComponentTextureMask` and
/// `FeComponentLinked` all inherit `FeComponentBase`'s, which is `return;` (`0x140b6a970`). **A text
/// field never responds to a sequence play at all** -- it follows an ancestor sprite. Inside a
/// sprite the id is looked up in that sprite's own table (resource at `+0x48`, table at `+0x18` of
/// that, entries at `+0x00`, `u16` count at `+0x08`, `0x10`-byte entries of `{i32 id, u16 start}`)
/// and **a miss falls through to `RET`** -- a silent no-op leaving that sprite where `0x7a` parked
/// it. A row holds more than one sprite, so it can half-move: the plate reaches the faded frame and
/// the caption's sprite never moves. That is the empty plate, and it is a per-SPRITE fact, not a
/// per-row one.
///
/// Which sprites carry a `0x6c` entry is layout data in `GameDataEbl.bdt` and is NOT established.
/// The measurement that looks like it settles this does not: `FeComponentObject`'s bracket getters
/// at slots `0x58`/`0x60` (`0x140b6a4f0`, `0x140b6a490`) tail-jump to the FIRST child only and do
/// not aggregate, so a `0x6c start=89 end=103` read off a row element describes the first sprite on
/// its leftmost spine and nothing else in the tree.
///
/// No game code writes any of these captions: each row's element id occurs exactly ONCE in the
/// whole image, in the row builder (row 2's `17010022` at `0x1400f4462`). The content is authored
/// in the layout and reached by the path the builder assembles.
///
/// **Measured, and the first version got this wrong.** Forcing all six drew two rows on top of each
/// other on screen. The grid coordinates are not the reason -- a run logged them as
/// `0:(0,0) 1:(0,1) 2:(0,2) 3:(0,3) 4:(0,4) 5:(0,5)`, six distinct positions -- so the collision is
/// in the layout resource, which evidently has no designed place for rows the game never shows
/// together.
pub const FE_TOP_MENU_FORCE_SHOWN_ROWS: u32 = 1 << 1;

/// `FrontendEx::SceneObjProxy`'s vtable slot 0: the proxy's own path-to-element resolver.
///
/// **This is the only verified way to get a row's layout element, and guessing cost this project a
/// round of false conclusions.** A probe read `cell+0x40`, fell back to `cell+0x30`, and called the
/// result "the row's element". It is not: all six rows came back with the same pointer, the same
/// sequence table and the same owner, which is one shared object read six times -- and a tree walk
/// from it ran off into unrelated memory past depth 5.
///
/// The real route is the one the game uses. `ComponentPositionProperty::get` (`0x14001e4d0`) does
/// `RCX=[this+8]` (the proxy back-pointer), `CALL [[RCX]]`, then `TEST RAX,RAX` -- so slot 0 takes
/// the proxy and returns the element or null. Inside, `0x140027ce0` matches the path copied to
/// `+0x60` by the binder against the scene at `+0x58` through `0x140afdad0`.
pub const FE_SCENE_OBJ_PROXY_ELEMENT_SLOT: usize = 0;

/// `FeComponentSprite`'s vtable. RVA `0x011de318`, VA `0x1411de318`. From MSVC RTTI.
///
/// The only leaf component class that acts on a sequence play. `FeComponentObject` (`0x011ddfa8`)
/// forwards to every child; `FeComponentBase`'s slot `0xc0` (`0x140b6a970`) is `return;` and
/// `FeComponentTextField`, `FeComponentTextureShape`, `FeComponentMaskShape`,
/// `FeComponentTextureMask` and `FeComponentLinked` all inherit that. So a text field never
/// responds to a play and follows an ancestor sprite instead.
pub const FE_COMPONENT_SPRITE_VTABLE: u32 = 0x011d_e318;

/// A component's first child. `+0x38`. Siblings chain through [`FE_COMPONENT_SIBLING_OFFSET`].
///
/// Read from `FeComponentObject`'s play at `0x140b6a98f` (`MOV RBX,[RCX+0x38]`).
pub const FE_COMPONENT_CHILD_OFFSET: usize = 0x38;

/// A component's next sibling. `+0x28`. Read at `0x140b6a9c5` (`MOV RBX,[RBX+0x28]`).
pub const FE_COMPONENT_SIBLING_OFFSET: usize = 0x28;

/// A `FeComponentSprite`'s animation resource. `+0x48`. Its sequence table is at `+0x18` of that.
///
/// From `0x140b6c4fa`: `MOV RAX,[RCX+0x48]; MOV RCX,[RAX+0x18]`. A null table means every play on
/// this sprite is a no-op.
pub const FE_SPRITE_RESOURCE_OFFSET: usize = 0x48;

/// The sequence table within a sprite's animation resource. `+0x18`.
pub const FE_SPRITE_TABLE_OFFSET: usize = 0x18;

/// A sequence table's entry array (`+0x00`) and its `u16` entry count (`+0x08`).
///
/// Entries are `0x10` bytes: an `i32` id at `+0` and a `u16` start frame at `+4`. The play scans
/// them linearly and **falls through to `RET` on a miss** (`0x140b6c50c`, `0x140b6c532`), leaving
/// the sprite exactly where it was -- which is how half a row ends up posed and the rest blank.
pub const FE_SPRITE_TABLE_ENTRIES_OFFSET: usize = 0x00;
/// See [`FE_SPRITE_TABLE_ENTRIES_OFFSET`].
pub const FE_SPRITE_TABLE_COUNT_OFFSET: usize = 0x08;
/// See [`FE_SPRITE_TABLE_ENTRIES_OFFSET`].
pub const FE_SPRITE_TABLE_ENTRY_STRIDE: usize = 0x10;
/// See [`FE_SPRITE_TABLE_ENTRIES_OFFSET`].
pub const FE_SPRITE_TABLE_ENTRY_START_OFFSET: usize = 0x04;

/// A `FeComponentSprite`'s current playback position, a `f32`. `+0x40`.
///
/// Written by the play at `0x140b6c57b` (`MOVSS [RBX+0x40],XMM6`) as `start(sequence) + offset`.
/// Reading it back is how "did this sprite actually follow the sequence" is answered without
/// looking at the screen.
pub const FE_SPRITE_POSITION_OFFSET: usize = 0x40;

/// Rows 2 and 3, the pair the game guarantees is never shown together.
///
/// `0x1400f4344` computes row 3's enable byte as `row2.enabled == 0`, so their XOR is true at every
/// instant. Adding either to [`FE_TOP_MENU_FORCE_SHOWN_ROWS`] therefore does not reveal a hidden
/// row -- it forces the dead half of a mutually exclusive pair on screen next to the live half.
/// That was done, it drew both INFORMATION and GO ONLINE at once, and
/// `FE_TOP_MENU_PAIR_MUTUALLY_EXCLUSIVE` exists so the next attempt is caught by a log line instead
/// of by someone looking at the screen.
pub const FE_TOP_MENU_PAIR_MUTUALLY_EXCLUSIVE: u32 = (1 << 2) | (1 << 3);

/// `FeObjectButtonEx`'s flags word. `+0x14`, a DWORD.
///
/// Read at `0x14010ad62` (`mov eax,[rbx+0x14]`) in the button's own styling method, and tested for
/// a skip bit at `0x14010ad35` (`test BYTE PTR [rcx+0x14],0x40`) before anything else happens.
pub const FE_BUTTON_FLAGS_OFFSET: usize = 0x14;

/// The flag bit that makes a button draw its alternate look. `0x400`. **DEAD END, DO NOT USE.**
///
/// Setting it on a title-menu row changes nothing, and the reason is one field over. The styling
/// method opens `test BYTE PTR [rcx+0x14],0x40; jne <exit>` at `0x14010ad35` -- **bit 6 is a skip
/// bit**, and a live run logged these buttons' flags as `0x00000040`, so the method returns before
/// it ever reaches the branch this bit selects. `FeObjectButtonEx` draws nothing of its own here;
/// the row's appearance comes entirely from the menu's own pass through the layout element.
///
/// Kept documented rather than deleted so the next attempt does not rediscover it. The mechanics
/// below are accurate; they are simply unreachable for this menu.
///
/// `FeObjectButtonEx`'s styling method branches on `test eax,0x401` at `0x14010ad65`: with neither
/// bit it plays sequence `0x6c`, with either it plays **`0x7e`** instead. **This is the only
/// alternate appearance the button class has**, and it is the nearest thing in the image to a
/// "drawn but not offered" look -- the menu's own styling pass never reaches it, because that pass
/// removes an unavailable row from the screen rather than restyling it.
///
/// Bit `0x400` rather than bit `0x1`, deliberately: both take the same branch, but bit `0x1` also
/// writes `[this+0x18] = 1` at `0x14010ad70`, which a second method (`0x14010af10`) reads to
/// suppress a different sequence. `0x400` changes the appearance and nothing else, which is the
/// smallest change that can produce the effect.
pub const FE_BUTTON_FLAG_ALTERNATE_LOOK: i32 = 0x400;

/// Bind a row descriptor to a `FrontendEx::SceneObjProxy`. RVA `0x00026790`, VA `0x140026790`.
///
/// `proxy* bind(group + 0x100, scratch, descriptor)`. It forwards to `0x140027880`, which installs
/// `SceneObjProxy::vftable`, stores the group at `+0x58` and copies the descriptor's label object
/// to `+0x60`. The menu's styling pass builds one per row per pass into a 144-byte stack scratch
/// and lets it die there.
pub const FE_BIND_SCENE_OBJ_PROXY: u32 = 0x0002_6790;

/// The `ComponentFrameCtrl` embedded in a `SceneObjProxy`. `+0x40`.
///
/// `0x14001e150` initialises the proxy's members in order: `ComponentPositionProperty` at `+0x10`,
/// `ComponentSizeProperty` at `+0x20`, `ComponentFrameCtrl` at `+0x40`. **There is no colour or
/// alpha property among them**, which is why dimming a row is not reachable through this object and
/// has to come from a sequence the layout itself defines.
///
/// Vtable slot 0 of the frame control is the sequence play, called as
/// `play(this, id, 0, 0.0f)` at `0x1400f5087` -- RCX, EDX, R8D, XMM3.
pub const FE_SCENE_OBJ_PROXY_FRAME_CTRL: usize = 0x40;

/// Build the top menu's six row descriptors. RVA `0x000f4250`, VA `0x1400f4250`.
///
/// `list* build(list)` -- fills a caller-supplied 352-byte buffer and returns it. Four call sites,
/// including the cell factory, so calling it again per state change is a rate the game already
/// exceeds on its own.
pub const FE_TOP_MENU_BUILD_ROWS: u32 = 0x000f_4250;

/// Sequence for a row that is available. `0x67`.
///
/// The id the menu's own styling pass plays on an available row at `0x1400f5080`.
pub const FE_TOP_MENU_SEQUENCE_AVAILABLE: i32 = 0x67;

/// Sequence for a row that is drawn but not selectable. `0x6c`. **DO NOT TRUST THIS NAME.**
///
/// A control run -- the game's own title menu, with `show_unavailable` off and none of this mod's
/// writes in it -- showed that this is not a "faded" look and was never needed. The game draws
/// every row, dims one it cannot offer, and swaps INFORMATION and GO ONLINE inside one shared slot.
/// Meanwhile `0x6c` is frames 89..103 of a shared timeline whose next marker is `0x7a` at 104, the
/// removal: it is the segment leading OUT. Playing it and letting it run walks the row off the
/// screen; holding its last frame poses the row invisible. Both were reported as "the row is
/// blank", and both were this constant doing what it actually means.
///
/// The measured markers, all six identical across all six rows:
/// `0x67@1  0x69@9  0x6a@15  0x6b@83  0x6c@89  0x7a@104`.
///
/// If a genuine dimmed look is ever wanted, `0x6b@83` and `0x6a@15` are the unexamined neighbours;
/// which frames render is an alpha curve in `GameDataEbl.bdt` and is not readable from the image.
///
/// Original note, kept because its trial method is sound even though its conclusion was not:
///
/// Sequence for a row that is drawn but not selectable. `0x6c`. **MEASURED ON SCREEN.**
///
/// This one could not be established statically and was not guessed. Sequence ids index a layout
/// resource inside `GameDataEbl.bdt`, so a run played a different candidate on each menu row at
/// once -- `0x6c`, `0x7e`, `0x69`, `0x6b`, `0x70`, with row 0 held at
/// [`FE_TOP_MENU_SEQUENCE_AVAILABLE`] as the control -- and `0x6c` was the one that came back
/// faded. The candidates were the layout's own vocabulary, taken from the sequences
/// `FeObjectButtonEx`'s methods play on their elements, rather than a sweep of the id space.
///
/// Two cheaper routes were tried first and both are dead, recorded so they are not retried:
/// [`FE_BUTTON_FLAG_ALTERNATE_LOOK`] is unreachable on these buttons, and the row's proxy carries
/// position, size and frame-control properties but no colour or alpha.
pub const FE_TOP_MENU_SEQUENCE_FADED: i32 = 0x6c;

/// `FexGroupList<FeGroupGrid>`'s "nothing here is selectable" pass. RVA `0x00106240`.
///
/// Vtable slot 3, and its whole body is a loop writing [`FE_BUTTON_STATE_UNAVAILABLE`] into every
/// cell in the list. Slot 5 (`0x140106290`) is its inverse, resetting `2 -> 3`.
///
/// **This is why the menu is drawn before it can be used**, and it is the reason no separate
/// "is it ready yet" flag has to be invented: the game already publishes that fact, per cell, in
/// the field the navigation predicate reads. A row that cannot be selected -- because its own
/// enable byte was false, or because this pass disabled the whole list while the title scene is
/// still animating -- is exactly a row in state 2.
pub const FE_TOP_MENU_DISABLE_ALL: u32 = 0x0010_6240;

/// The `GameManagerImp` pointer. RVA `0x016148f0`, VA `0x1416148f0`.
///
/// Read at `0x1400f432d` in the top-menu row builder, among many other places.
pub const GAME_MANAGER: u32 = 0x0161_48f0;

/// `SaveLoadSystem` within `GameManagerImp`. `+0xb8`.
///
/// `FeSubStateTitleLoadProfile`'s work starter reads it at `0x1400fc384`
/// (`mov rdi,[rax+0xb8]`) before handing it to the loader.
pub const GAME_MANAGER_SAVE_LOAD_SYSTEM: usize = 0xb8;

/// The `SaveLoadSystem` field that is non-zero while a request is in flight. `+0x8`.
///
/// The pump at `0x1402e6230` opens `mov eax,[rcx+8]; sub eax,2; test eax,0xfffffffd; jne <bail>`,
/// so it only does work when this is `2` or `4`, and every completion path writes `0`. A save or
/// profile load in progress is therefore a non-zero here, read out of the game's own field rather
/// than inferred from a clock.
///
/// **Why this and not the cell states.** The menu is drawn before it can be used, and the obvious
/// candidate for that was `FE_TOP_MENU_DISABLE_ALL` -- but a live run showed the cell states never
/// reading all-unavailable, only the two rows the enable bytes had already ruled out. So whatever
/// holds input during that window is above the cells, and the window measurably coincides with the
/// save arriving: the same run logged the row states flipping from `0b000110` to `0b001000` at the
/// moment the save landed.
pub const SAVE_LOAD_SYSTEM_REQUEST: usize = 0x8;

/// `FeGroupTitleTopMenu::TitleButtonLayout`'s cell factory. RVA `0x000f36b0`, VA `0x1400f36b0`.
///
/// `proxy* build(layout, proxy_out, coords)` -- `coords` is `int[2]`, column then row. It rebuilds
/// the descriptor list, and for an in-range `(0, row)` binds descriptor `row` into the caller's
/// proxy through [`FE_BIND_SCENE_OBJ_PROXY`]; anything out of range gets an empty cell from
/// `0x140027980` instead. The bound proxy is returned in RAX, so a detour has the row's layout
/// element in hand without rebuilding anything.
///
/// **This is the earliest moment a row exists**, which is what makes it the only place able to
/// decide what a row looks like on its very first frame. Everything else -- the styling pass, the
/// substate updates -- runs after the rows are already on screen.
///
/// Not Arxan-redirected; prologue `48 89 5c 24 08`, five bytes exactly.
pub const FE_TOP_MENU_BUILD_CELL: u32 = 0x000f_36b0;

/// A seek offset far past the end of any UI sequence. `1000.0` seconds.
///
/// **The fourth argument to a sequence play is a start offset, not a flag.** Read from
/// `FeObjectButtonEx`'s own styling method, which computes one rather than passing zero:
///
/// ```text
/// play(element, 0x68, 0, (now - start_of_85) * (span_of_68 / span_of_85))
/// ```
///
/// -- it starts sequence `0x68` at the point corresponding to how far `0x85` has already run. So a
/// sequence can be entered part-way, and passing `0.0` everywhere means "always play from the
/// beginning", which for a fade means watching it fade rather than seeing it faded.
///
/// How far into the fade to start it. `14.0`, the fade's own span.
///
/// **The fourth argument to a sequence play is an offset from that sequence's START frame**, read
/// out of `FeComponentSprite::v24` (`0x140b6c4f0`) rather than inferred:
///
/// ```text
/// entry     = table_lookup(this, sequence)      # 0x10 bytes per entry, id at +0, start at +4
/// param_4   = (float)entry.start + param_4      # <- the offset is RELATIVE
/// [this+0x40] = param_4                         # <- and this is the playback position
/// ```
///
/// The position is a plain float at `+0x40` of the sprite; `v9` is `movss xmm0,[rcx+0x40]; ret`.
///
/// With `0x6c` measured at frames 89 to 103, that makes the arithmetic exact, and explains four
/// on-screen results that looked contradictory:
///
/// | passed | lands on | seen |
/// | --- | --- | --- |
/// | `0.0` | 89, the fade's FIRST frame | fades, playing all 14 frames |
/// | `14.0` | 103, its last frame | faded immediately -- what is wanted |
/// | `100.0` | 189 | nothing: past the end of a 104-frame animation |
/// | `103.0` | 192 | nothing |
/// | `1000.0` | 1089 | nothing |
///
/// The earlier reading of this argument as an absolute timeline position is what made `103.0` look
/// like the obvious value; it is off by the sequence's own start every time.
pub const FE_TOP_MENU_SEQUENCE_FADED_SEEK: f32 = 14.0;

// ============================================================================================
// THE TITLE STATE MACHINE ITSELF -- `FeStateFlow`, its resident substate, and the id space.
//
// Everything above this line names a specific substate or a specific screen. These name the
// MACHINE, which is why they are grouped: they are what `ds2-boot-timeline` needs in order to
// instrument every step without knowing any step's name, and what a loading bar would be driven
// from. See `docs/DS2-BOOT-WORK.md` for the trace and the full boot chain.
// ============================================================================================

/// `FeStateFlow::update` -- the dispatcher that drives the resident substate. RVA `0x00104540`.
///
/// # How it was established
///
/// `FeOperatorTitle::v4`'s phase-4 branch calls it at `0x1400ef42b` with `RCX = [operator+0x38]`,
/// and the body is unambiguous about what that object is: it reads the resident substate from
/// `+0x10`, calls `[[resident]+0x18]` (`update`) with the frame delta, and on a transition calls
/// `[[resident]+0x30]` (`v6`, drop transitions), `[[resident]+0x10]` (`leave`), then
/// `[[next]+0x08]` (`enter`) and `[[next]+0x28]` (`v5`, publish transitions). It is not an Arxan
/// redirect -- `scripts/ds2-arxan-chain.py` terminates at hop 0 with the prologue
/// `40 53 48 83 ec 30` (`push rbx; sub rsp,0x30`) at the entry.
///
/// # Its signature is two arguments, and that is read rather than assumed
///
/// `this` in RCX and the frame delta in XMM1. Every other register the body uses it loads from the
/// object first -- `mov rdx,[rbx+0x28]`, `mov r8,[rbx+0x30]` -- so there is no third argument a
/// detour could clobber by using the register as scratch. The float rules out `ds2-hook`'s union,
/// whose shared signature is four integers.
pub const FE_STATE_FLOW_UPDATE: u32 = 0x0010_4540;

/// `FeSubStateBase::v6` -- "drop the transitions I published". RVA `0x001043a0`.
///
/// The flow calls this on the outgoing substate immediately before `leave`, on both of its
/// transition paths (`0x140104584` and `0x1401046a9`). **Checked against all 36 `FeSubState*`
/// vtables: not one overrides slot 6.** That is what makes this single address every departure in
/// the game, and it is the reason `ds2-boot-timeline` can see steps a per-class hook would miss --
/// the failure `ds2-dialog-skip` already hit once with `FeSubStateTitleInformation`, which shows
/// its wait window from `update` rather than `enter`.
///
/// Not an Arxan redirect: `scripts/ds2-arxan-chain.py` terminates at hop 0 with the prologue
/// `48 89 6c 24 18 57 41 56` at the entry.
///
/// Its arguments are `(this, transitions, context)`, all integer -- taken from the two call sites,
/// which both set RDX from `[flow+0x28]` and R8 from `[flow+0x30]`.
pub const FE_SUBSTATE_DROP_TRANSITIONS: u32 = 0x0010_43a0;

/// Offset of the resident substate pointer in `FeStateFlow`.
///
/// Read at `0x1401045e3` (`mov rcx,[rbx+0x10]`) before the resident's `update`, and written at
/// `0x1401046bf` (`mov [rbx+0x10],rdi`) when a transition is taken.
pub const FE_STATE_FLOW_RESIDENT_SUBSTATE_OFFSET: usize = 0x10;

/// Offset of the substate list in `FeStateFlow` -- the `TMenuStateBaseList<FeSubStateBase, 0x58>`
/// that `FeStateTitle::v6` fills with 64 substates.
///
/// Read at `0x140104658` (`mov rdx,[rbx+0x20]`), immediately before the loop that searches it for
/// the requested id.
pub const FE_STATE_FLOW_SUBSTATE_LIST_OFFSET: usize = 0x20;

/// Offset of the pending-request id in `FeStateFlow`: the substate an outside caller has asked the
/// flow to move to, or `-1` for none.
///
/// `FeOperatorTitle::v4` writes `0x17` here at `0x1400ef3e4` to return to the title screen. The
/// flow compares it against zero as a **signed** value at `0x140104633` and writes `-1` at
/// `0x140104649` once it has been consumed, which is what fixes the type as `i32`.
pub const FE_STATE_FLOW_PENDING_ID_OFFSET: usize = 0x48;

/// Offset of the entry count in the substate list. Entries themselves start at `+0x08`.
///
/// Read at `0x140104661` (`movsxd r10,[rdx+0x2c8]`) to bound the transition search, and
/// incremented by `FeStateTitle::v6` once per substate it appends.
pub const FE_SUBSTATE_LIST_COUNT_OFFSET: usize = 0x2c8;

/// Offset of a substate's own id.
///
/// **This is the game's id, not a label this repo invented.** `FeStateFlow`'s transition search
/// compares this exact field against the requested id at `0x14010467f`
/// (`cmp [rdi+0xc],esi`), and every substate constructor writes it -- as a literal
/// (`FeSubStateTitleMain` writes `0x17`) or from `EDX` at the call site (the four
/// `FeSubStateTitleLogo` instances get `0x13` through `0x16`).
///
/// It is the same field `FE_DIALOG_KIND_OFFSET` and `FE_PROCESS_WINDOW_KIND_OFFSET` already name
/// at `0x0c` for their own classes, and the `kind=57` / `kind=70` those two log on a real boot are
/// `0x39 FeSubStateTitleGameServerLogin` and the message box built beside
/// `0x44 FeSubStateTitleInformation` -- which is the runtime evidence that this id space is right.
pub const FE_SUBSTATE_ID_OFFSET: usize = 0x0c;

/// Id of `FeSubStateTitleTopMenu` -- the end of the boot chain, and the screen "Continue" is on.
///
/// Written as the literal `0x47` by its constructor at `0x1400fd65d`
/// (`mov QWORD PTR [rcx+0xc],0x47`), and corroborated by `FeSubStateTitleOptionGame`'s transition
/// table, whose phase-4 edge names `0x47` as its destination.
pub const FE_SUBSTATE_ID_TITLE_TOP_MENU: u32 = 0x47;

// ============================================================================================
// THE TWO STATE WORDS BEHIND THE ONE-SECOND FLOORS (`ds2-mods-rs-wxl`).
//
// 0x05 SteamLoadSystemData and 0x44 Information each take ~1.01s, reproducibly to within 2ms
// across runs, which is a clock rather than work. Their own `update` functions contain no
// threshold, and no sleep or wait import is called from anywhere in `SaveLoad2`. So the question
// is which side of the boundary the second is spent on: the substate polling a service that
// finished long ago, or the service genuinely taking a second.
//
// These are the fields that answer it. Both are read once per frame while their substate is
// resident, and only a CHANGE is logged.
// ============================================================================================

/// `GameManagerImp`, the root singleton most engine services hang off. RVA `0x016148f0`.
///
/// Every process-window substate reaches its backend through this: `[+0xb8]` is the storage
/// service, `[+0x22f0]` the network service, `[+0x22e0]` the window system, `[+0xa8]` the object
/// holding the savedata block at its own `+0xd8`.
pub const GAME_MANAGER_IMP: u32 = 0x0161_48f0;

/// Offset of `SaveLoadSystem` in [`GAME_MANAGER_IMP`].
///
/// Read at `0x1400fc3f7` (`mov rbp,[rdx+0xb8]`) in `SaveSystemData`'s enter and at `0x1400fb004`
/// in `SteamLoadSystemData`'s, among others.
pub const SAVE_LOAD_SYSTEM_OFFSET: usize = 0xb8;

/// `SaveLoadSystem`'s request state word.
///
/// **This is the interlock.** Every start entry point refuses while it is non-zero
/// (`0x1402e72c0` and `0x1402e7170` both open `if ([this+0x08] != 0 || [this+0x0c] != 0) return
/// false`), and both pollers gate on it: `0x1402e6230` accepts `{2, 4}`, `0x1402e67f0` tests
/// `bt 0x6a` for `{1, 3, 5, 6}`. If it flips out of "working" long before
/// `0x05 SteamLoadSystemData` advances, the floor is in the substate; if it stays put for the full
/// second, the floor is below, in `SaveLoad2`.
pub const SAVE_LOAD_SYSTEM_STATE_OFFSET: usize = 0x08;

/// The second half of the same interlock, checked alongside [`SAVE_LOAD_SYSTEM_STATE_OFFSET`].
pub const SAVE_LOAD_SYSTEM_SUBSTATE_OFFSET: usize = 0x0c;

/// The title context singleton. RVA `0x0160de10`.
///
/// `[+0x80]` is `FeSceneTitle`, `[+0xa0]` the information job below, `[+0x568]` the skip flag the
/// boot screens share, `[+0x54c]`/`[+0x558]`/`[+0x55c]`/`[+0x560]` result codes the substates
/// publish.
pub const FE_TITLE_CONTEXT: u32 = 0x0160_de10;

/// Offset of the information-download job in [`FE_TITLE_CONTEXT`].
///
/// Read at `0x1400ff787` (`mov rbx,[rax+0xa0]`) in `FeSubStateTitleInformation::v3`'s phase-4
/// branch, which ticks it through `[[job]+0x20]` and then tests the field below.
pub const FE_TITLE_INFORMATION_JOB_OFFSET: usize = 0xa0;

/// The information job's own state, the value `0x44 Information` is waiting on.
///
/// Read at `0x1400ff797` (`mov eax,[rbx+0x18]`) and compared against `5` then `6`; either sends
/// the substate to its terminal phase. Watching it says whether the job finishes early and the
/// substate sits on the result, or the job itself takes the second.
pub const FE_INFORMATION_JOB_STATE_OFFSET: usize = 0x18;

// ============================================================================================
// THE ONE-SECOND FLOORS, LOCATED (`ds2-mods-rs-wxl`). ~1.86s of a 6.7s boot.
//
// Two substates that are NOT `FeSubStateProcessWindowBase` subclasses -- so `ds2-dialog-skip`'s
// min-duration zeroing never reached them -- each hold their own elapsed timer and compare it
// against the SAME float, `0x1410ac698`, which is `1.0f`.
//
// Measured, run 6: `0x05` reaches phase 4 at t=4115.9ms and does not leave it until t=4994.9ms --
// 879ms. `0x44` reaches phase 2 at t=5676.4ms and does not leave until t=6661.4ms -- 985ms. In
// both cases the work they were waiting for had already finished.
//
// DO NOT PATCH THE CONSTANT. `0x1410ac698` has 2042 RIP-relative references from 1548 functions:
// it is MSVC's pooled `1.0f` literal for the whole image, not a tunable belonging to these two.
// The fix is to advance each substate's OWN elapsed field so the game's own comparison passes,
// which is the same shape as the existing min-duration zeroing and leaves both the comparison and
// the transition the game's.
// ============================================================================================

/// `FeSubStateTitleSteamLoadSystemData::v1` -- its `enter`. RVA `0x000faff0`.
///
/// Starts the system-data load through `SaveLoadSystem` (`0x1402e72c0`), shows a process window,
/// and sets phase 1. Measured: the storage work is finished 88ms later; the substate then spends
/// 879ms in phase 4 waiting on the floor below.
pub const FE_SUBSTATE_STEAM_LOAD_SYSTEM_DATA_ENTER: u32 = 0x000f_aff0;

/// Its elapsed timer, a `f32`.
///
/// Zeroed by the constructor at `0x1400fab66`. Accumulated in phases 1 and 2 without ever being
/// compared, and compared in **phase 4** at `0x1400fc13e`:
/// `addss xmm6,[rdi+0x18]; comiss xmm6,[0x1410ac698]; jb return`.
pub const FE_SUBSTATE_STEAM_LOAD_SYSTEM_DATA_ELAPSED_OFFSET: usize = 0x18;

/// `FeSubStateTitleInformation::v1` -- its `enter`. RVA `0x000ff570`.
pub const FE_SUBSTATE_TITLE_INFORMATION_ENTER: u32 = 0x000f_f570;

/// Its elapsed timer, a `f32`, at `+0x5a24` of a `0x5a30`-byte object.
///
/// Compared in **phase 2** at `0x1400ff9b7`, and reset to zero on the way out at `0x1400ff9d7`.
///
/// **Phase 2 waits on two things, and only one of them is the floor**:
/// ```text
/// elapsed += delta
/// if ([r14]->vtable[0x28]()) return;         // the job is still running -- REAL work
/// if (elapsed < 1.0f)        return;         // the floor
/// close the window; phase = 3
/// ```
/// Advancing the timer therefore removes the floor and leaves the job wait entirely intact, which
/// is the whole reason this is safe to do.
pub const FE_SUBSTATE_TITLE_INFORMATION_ELAPSED_OFFSET: usize = 0x5a24;

/// What to write into either elapsed field so the game's own `comiss` passes on the first frame.
///
/// `1.0f` exactly would leave `comiss` at equality, and the branch is `jb` -- below, not
/// below-or-equal -- so equality already passes. A slightly larger value is used anyway so that a
/// frame delta being added before the comparison cannot matter, and so the intent is legible: this
/// is "the minimum display time has elapsed", not "the timer is exactly at the threshold".
pub const FE_SUBSTATE_FLOOR_ELAPSED: f32 = 2.0;
