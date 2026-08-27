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

/// **Plays sequence `0x67` on `FeSceneTitle`, and does NOT stop the text animating.** RVA
/// `0x000f3820`. Kept as a recorded negative result, not used.
///
/// Original reasoning, which was sound as far as it went: `FeSubStateTitleMain::v1` calls
/// `0x1400f3e30` (`0x1400fda54`), which plays sequence **`0x66`** on `[scene+8]` -- the title text
/// animating in -- and nothing in the phase machine stops it. `0x1400f3820` plays **`0x67`** on the
/// same object, the settled state that [`FE_TITLE_MAIN_SEQUENCE_GATE`] waits to observe. Playing it
/// should therefore have put the text at its final position on the first frame.
///
/// **It did not.** Called once on the first `FeSubStateTitleMain` update, in a live run, the text
/// animated in exactly as before. Unlike
/// [`FE_SEQUENCE_NOT_A_FINISH_DO_NOT_USE`] the function does do what its name says -- its body is
/// unambiguous -- so the wrong assumption is elsewhere: either `0x66` continues in parallel rather
/// than being replaced, or `0x67` carries its own entry animation. That is the open question.
///
/// The four sequence ids used across the Fe scenes are `0x65`, `0x66`, `0x67` and `0x68`, read from
/// the 91 call sites of the sequence-play forwarder `0x140afdb80`, with `0x66`/`0x68` reading as
/// the in and out transitions and `0x67` as the settled state -- corroborated by
/// `FeSubStateTitleLogo` using the same set.
pub const FE_SCENE_TITLE_PLAY_IDLE_INEFFECTIVE: u32 = 0x000f_3820;
