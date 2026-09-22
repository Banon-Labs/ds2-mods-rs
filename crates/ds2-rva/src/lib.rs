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
/// [`FE_DIALOG_CONFIRM_DEST_OFFSET`] -- `0x1404fe2a0` for a one-button box, `0x1404fe1c0` for a choice
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

/// **Destination substate id for the CONFIRM edge**, or `-1` when the box has no confirm edge.
/// `+0x12`, a signed WORD.
///
/// This was recorded as an "option count" and that was wrong in a way worth spelling out, because
/// the wrong reading is behaviourally right and therefore does not announce itself. `v5`
/// (`0x140104f30`) publishes the second transition only when this field is non-negative:
///
/// ```text
/// cmp   WORD PTR [rdi+0x12], 0
/// jl    done                         ; negative -> NO confirm edge at all
/// movsx ecx, WORD PTR [rdi+0x12]     ; <- this field
/// mov   [rax+0x08], ecx              ; destination substate
/// mov   BYTE PTR [rax+0x20], 4       ; ...when the phase is 4, the confirm-closed phase
/// ```
///
/// So "negative means a one-button acknowledgement box" is a true *consequence* -- a box with no
/// confirm destination has nothing for a second button to do -- and every sign test the old doc
/// cited is real. But the magnitude is not a count. The constructor at `0x140104c00` seeds it to
/// `-1` (`or eax,-1; mov WORD PTR [rcx+0x12], ax`) and whoever raises a box overwrites it.
///
/// **The practical consequence:** a log line reading `options=42` never meant forty-two options.
/// It meant destination `0x2a`, [`FE_SUBSTATE_ID_OFFLINE_MODE_WINDOW`].
pub const FE_DIALOG_CONFIRM_DEST_OFFSET: usize = 0x12;

/// [`FE_DIALOG_PHASE_OFFSET`] once the box has closed on a [`FE_DIALOG_RESULT_CONFIRM`].
///
/// The pair of [`FE_DIALOG_PHASE_CLOSED_CANCEL`]; `v5` watches for this value to take the
/// [`FE_DIALOG_CONFIRM_DEST_OFFSET`] edge.
pub const FE_DIALOG_PHASE_CLOSED_CONFIRM: u8 = 4;

/// `FeSubStateOfflineModeWindow`'s substate id: `0x2a`.
///
/// **Do not reason about this box from its button labels.** The two-option box the game raises
/// when the server login fails says, in its own text, `Select "OK" to attempt to log in again` and
/// `Select "CANCEL" to start the game in offline mode` -- so the obvious move is to write
/// [`FE_DIALOG_RESULT_CANCEL`]. That is the wrong answer, and it is wrong in the worst possible
/// direction for a mod whose purpose is playing offline.
///
/// Read live out of the object (`kind=0x3e`) on a running game:
///
/// ```text
/// CANCEL_dest = 0x39   FeSubStateTitleGameServerLogin   <- retries the login
/// CONFIRM_dest = 0x2a  FeSubStateOfflineModeWindow      <- plays offline
/// ```
///
/// So the edge that goes offline is the **confirm** edge. Corroborated twice over: the run that
/// produced those numbers had `result=2` in the object and logged
/// `suppressed screen=offline-mode-window kind=42` as the very next line, and eight other boxes in
/// the same substate table carry `0x2a` as their cancel destination.
///
/// Anything answering that box should select its edge by comparing these destination ids against
/// this constant, never by picking a result value from what the buttons are called.
pub const FE_SUBSTATE_ID_OFFLINE_MODE_WINDOW: i16 = 0x2a;

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
///    [`FE_DIALOG_CONFIRM_DEST_OFFSET`] is hardcoded to `-1`. The update's input path can therefore
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

/// **Destination substate id for the CANCEL edge.** `+0x10`, a signed WORD.
///
/// This was recorded as a "caption/message id" and that was wrong. It is a substate id, and `v5`
/// (`0x140104f30`) publishes it as the destination of a transition:
///
/// ```text
/// movsx edx, WORD PTR [rdi+0x10]     ; <- this field
/// mov   [rax+0x18], &this[0x30]      ; watch the PHASE
/// mov   [rax+0x08], edx              ; destination substate
/// mov   BYTE PTR [rax+0x20], 3       ; ...when the phase is 3, the cancel-closed phase
/// ```
///
/// The constructor at `0x140104c00` sets it from its third argument (`mov WORD PTR [rcx+0x10],
/// r8w`). The old reading came from the boot notices, where that argument is `0x20` -- which looks
/// exactly like a caption id and is in fact substate `0x20`, `FeSubStateTitleSteamNetworkCheck`.
///
/// Confirmed against 22 live instances read out of `/proc/<pid>/mem` on a running game: every one
/// holds a plausible substate id, several of them `0x2a`
/// ([`FE_SUBSTATE_ID_OFFLINE_MODE_WINDOW`]), and the one that had fired held `0x39`
/// (`FeSubStateTitleGameServerLogin`).
pub const FE_DIALOG_CANCEL_DEST_OFFSET: usize = 0x10;

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
///
/// **"Leaf" is right about sequence plays and wrong about the tree, and the difference crashed a
/// walk.** `FeComponentSprite::findByIdPath` (`0x140b6bec0`) holds children a third way -- not the
/// `+0x38` linked list `FeComponentObject` and `FeComponentScene` use, but the DISPLAY LIST:
///
/// ```asm
/// mov   rbx, [rcx+0x70]          ; the list
/// cmp   dx,  [rcx+0x66]          ; against the live child count
/// cmp   [rbx+0xc], eax           ; this entry's key against the id being looked for
/// mov   rcx, [rbx]               ; the child
/// add   rbx, 0x10                ; next entry
/// ```
///
/// That is the same list [`FLO_DEFINITION_CHILD_COUNT_OFFSET`] bounds and `FUN_140b6bd80` fills,
/// so **the container built from the quit tab's definition is one of these** -- which is why
/// raising that definition's child count grew it. The genuine tree leaves are the classes whose
/// `findByIdPath` is `xor eax,eax; ret` at [`FE_COMPONENT_LEAF_FIND_BY_ID_PATH`].
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
// THE ONE-SECOND FLOORS, LOCATED (`ds2-mods-rs-wxl`). Predicted ~1.86s; MEASURED 875ms.
//
// Lifting both floors moved only `0x05`. `0x44` turned out to be sitting on a download job that
// always fails, not on this timer -- `ds2-mods-rs-umo` -- and the addresses below are what proved
// it. They are correct as read; it was the price on them that was wrong.
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

/// The import thunk `DarkSoulsII.exe` calls `KERNEL32!Sleep` through. RVA `0x01aae314`.
///
/// # Why an IAT slot rather than the function
///
/// The engine block -- 3.06s between input initialisation and the title flow -- reproduces to
/// 0.67%, reads nothing from disk after an early burst, and never exceeds half of one core. That
/// combination is the signature of sleeping, not of working, and the binary has a candidate: a
/// `PeekMessageW` / `Sleep(1)` / check-a-flag loop at `0x140fecdd6` that spins until `[rbx+0x11c]`
/// clears. Three seconds of that is about three thousand iterations.
///
/// Counting the game's `Sleep` calls tests it directly, and patching **the IAT slot** rather than
/// `KERNEL32!Sleep` itself is what keeps the test cheap and honest: it is a pointer write in
/// `.idata`, so no code is modified, Arxan's `.text` integrity checks have nothing to see, and only
/// this executable's calls are counted rather than every module in the process.
///
/// `Sleep` is `void Sleep(DWORD)` -- one integer argument, no stack arguments, no return value --
/// which is why this one can be fronted from Rust with an ordinary `extern "system"` function and
/// needs none of the naked-thunk machinery a ten-argument import like `D3D11CreateDevice` would.
///
/// Established by walking the import descriptors: `KERNEL32.dll`'s `FirstThunk` is at RVA
/// `0x1aade6c` and `Sleep` is the entry that lands here. There are **13** call sites in the image;
/// two of them are `Sleep(0)` yield loops and one is the `Sleep(1)` pump above.
pub const SLEEP_IAT_THUNK: u32 = 0x01aa_e314;

/// The frame limiter: "sleep the rest of this frame, or yield if we are already late".
/// RVA `0x00feb910`.
///
/// # What it does, read from its own body
///
/// ```text
/// now       = timeGetTime()
/// elapsed   = (now - [this+0x17c]) * 10000
/// [this+0x174] = moving average of elapsed and the previous frame time
/// [this+0x178] = elapsed
/// remaining = [this+0x170]
/// if (remaining <= 0) tail-call Sleep(0)          ; already late -- yield
/// else                tail-call Sleep(remaining / 1000)
/// ```
///
/// # Why it is worth counting
///
/// The 3.04s engine block asks for 9866 ms of sleep across 3631 `Sleep` calls, and 63% of all
/// `Sleep` calls in the boot pass `0` -- which is this function's late path. If the block is a loop
/// advancing a fixed number of steps per frame, then **calls to this function are frames**, and the
/// count will be the same on every run. That is the measurement that decides whether the engine
/// block can be attacked by pacing at all, and it must exist before anything tries.
///
/// Signature is `void(this)`: RCX in, no other argument read, and both exits are tail-jumps to
/// `Sleep` so nothing is returned. Not an Arxan redirect -- `scripts/ds2-arxan-chain.py` terminates
/// at hop 0 on its own prologue, `40 53 48 83 ec 20` (`push rbx; sub rsp,0x20`), which also gives
/// MinHook six clean bytes to relocate.
pub const FRAME_LIMITER: u32 = 0x00fe_b910;

/// The simpler sibling of [`FRAME_LIMITER`], and **measured never to be called during boot**.
///
/// RVA `0x00feb890`. Same tail -- sleep `[this+0x170]/1000`, or `Sleep(0)` when not positive --
/// but it skips the clock sample and the moving average entirely, writing a fixed `0x4c4b40` into
/// `[this+0x178]` instead. It was instrumented first on the strength of that shared tail and the
/// counter read **zero** across a whole boot, which is what moved the instrument to `0x00feb910`.
/// Recorded so the next person does not spend the same run finding out.
pub const FRAME_LIMITER_RESET: u32 = 0x00fe_b890;

// ---------------------------------------------------------------------------------------------
// THE NETWORK SERVICE AND ITS ONLINE FLAG
//
// One boolean decides whether DARK SOULS II believes it is online, and the whole of `ds2-offline`
// rests on the four facts below. All four were read out of `darksoulsii-deobf.bin`; no game was
// launched to establish any of them.
//
//   net = [[0x1416148f0] + 0x22f0]      GameManagerImp's network service
//   net + 0x3a                          the flag, ONE BYTE
//   0x140513600                         its getter, `movzx eax, byte [rcx+0x3a]; ret`
//   0x140513820                         its setter, `mov byte [rcx+0x3a], dl; ret`
//
// THE FLAG IS BORN ZERO. The service's constructor at `0x140512f30` -- identified by the vtable
// `0x1410d13e8` it installs at `[this]` -- writes `mov BYTE PTR [rbx+0x3a],0` at `0x140512f5a`,
// four instructions in. So "offline" is the state this object is CONSTRUCTED in, and every online
// run is one that left it. That is why the setter can be neutered rather than fought: neutering it
// does not impose a value, it prevents a departure from the game's own initial one.
// ---------------------------------------------------------------------------------------------

/// `NetService::isOnline`. RVA `0x00513600`, VA `0x140513600`.
///
/// The whole function is `movzx eax, BYTE PTR [rcx+0x3a]; ret` -- five bytes, and `docs/
/// DS2-TITLE-FLOW.md` already named it "the game's master online gate" from the other end, while
/// tracing which rows the top menu greys out.
///
/// **34 call sites**, found by scanning the image for `e8` displacements that resolve here
/// (`scripts/ds2-xrefs.py`). Every one of them is immediately followed by `test al,al` and a
/// branch, which is what makes forcing the return value a complete answer for its readers rather
/// than a partial one. Three of the 34 were disassembled to check that the polarity is what it
/// looks like:
///
/// * `FeSubStateTitleOnlineCheck::v8` (`0x1400f98c0`), the substate's own work starter: calls this,
///   and on a zero returns `false` **without starting anything**. So a forced zero does not fake
///   the online check -- it takes the shipped path where the check never runs.
/// * The top-menu builder at `0x1400f433b`: the result becomes `r14b`, which enables row 2 (server
///   information) and disables row 3 (go online), or the reverse.
/// * `0x1400fe739`, inside the same menu code, gating whether a transition is registered at all.
///
/// Not an Arxan redirect: `scripts/ds2-arxan-chain.py` terminates at hop 0 on the real prologue
/// `0f b6 41 3a c3`, followed by a `90` pad. Five bytes of body and a byte of padding is enough
/// room for the three-byte stub [`NET_IS_ONLINE_STUB`] writes and nothing has to be relocated.
pub const NET_IS_ONLINE: u32 = 0x0051_3600;

/// First byte of [`NET_IS_ONLINE`]'s body, `0f` -- the `movzx` opcode.
///
/// Passed to `ds2_hook::patch_3byte_stub` as its `expected_first` guard. If a future build moves
/// the function, this byte almost certainly differs and the patch aborts instead of landing in the
/// middle of some other function's instruction.
pub const NET_IS_ONLINE_EXPECTED_FIRST: u8 = 0x0f;

/// `xor eax,eax; ret` -- what [`NET_IS_ONLINE`] is replaced with. Reports offline to all 34 readers.
pub const NET_IS_ONLINE_STUB: [u8; 3] = [0x31, 0xc0, 0xc3];

/// `NetService::setOnline`. RVA `0x00513820`, VA `0x140513820`.
///
/// `mov BYTE PTR [rcx+0x3a], dl; ret` -- the exact write-side pair of [`NET_IS_ONLINE`], on the
/// same object at the same offset. Found from the other end rather than by searching for a setter:
/// `NetSvrManager`'s vtable slot `+0x60` (`0x140290040`) opens by calling this with `edx` zeroed,
/// and **`FeSubStateTitleSetOfflineMode::v1` (`0x1400f8f80`) is nothing but a tail-jump into that
/// slot**. So this is the write the game's own "play offline" substate performs.
///
/// Not an Arxan redirect; its own four-byte body plus `ret` sits at its entry, followed by `cc`
/// padding.
pub const NET_SET_ONLINE: u32 = 0x0051_3820;

/// First byte of [`NET_SET_ONLINE`]'s body, `88` -- the `mov r/m8, r8` opcode.
pub const NET_SET_ONLINE_EXPECTED_FIRST: u8 = 0x88;

/// `ret; nop; nop` -- what [`NET_SET_ONLINE`] is replaced with, making the setter inert.
///
/// **Not `xor eax,eax; ret`.** The setter returns nothing, so zeroing `eax` would be a lie about
/// its signature that happens to be harmless; `ret` says what is meant. The two `nop`s exist only
/// because `ds2_hook::patch_3byte_stub` writes three bytes, and they are never executed.
pub const NET_SET_ONLINE_STUB: [u8; 3] = [0xc3, 0x90, 0x90];

/// Byte offset of the online flag inside the network service object. Recorded for diagnostics --
/// nothing in this repo writes it directly.
///
/// Read three ways that agree: the getter's `[rcx+0x3a]`, the setter's `[rcx+0x3a]`, and the
/// constructor's `mov BYTE PTR [rbx+0x3a],0` at `0x140512f5a`.
pub const NET_ONLINE_FLAG_OFFSET: usize = 0x3a;

/// Offset of the network service in [`GAME_MANAGER_IMP`] -- the `this` every call to
/// [`NET_IS_ONLINE`] and [`NET_SET_ONLINE`] is made on.
///
/// The sibling of [`SAVE_LOAD_SYSTEM_OFFSET`], and named the same way. `GAME_MANAGER_IMP`'s own
/// doc comment already recorded `[+0x22f0]` as the network service from the boot-chain trace; this
/// is that offset given a name so the two call sites in `ds2-offline` do not spell it as a
/// literal. Every getter call site disassembled for [`NET_IS_ONLINE`] loads it the same way:
/// `mov rax,[0x1416148f0]; mov rcx,[rax+0x22f0]`.
pub const NET_SERVICE_OFFSET: usize = 0x22f0;

/// The force-offline byte at VA `0x14160de19`, and **it is not the switch it looks like**.
/// RVA `0x0160de19`.
///
/// `ds2-mods-rs-rk4` asked whether setting this removes the network boot chain. It does not.
/// It is read at **exactly one instruction in the whole image**, `0x1400f431f` inside the top-menu
/// builder:
///
/// ```text
/// cmp BYTE PTR [0x14160de19], 0
/// je  ask_the_gate          ; zero -> fall through to call 0x140513600
/// xor r14b, r14b            ; non-zero -> force "not online" and skip the call
/// ```
///
/// So it is a local override of one boolean in one function, and the boot chain -- which calls
/// [`NET_IS_ONLINE`] directly, on its own -- never sees it. Recorded here so the next reader does
/// not have to re-derive that it is a dead end; [`NET_IS_ONLINE`] is the read it was shadowing.
pub const NET_FORCE_OFFLINE_MENU_ONLY: u32 = 0x0160_de19;

// ============================================================================================
// THE SAVE-SLOT LOAD PATH. Which character slot a load actually used, and where the game keeps
// that answer. Read statically from `FeSubStateTitleLoadDataList::v3` at `0x1400fba10`, whose
// whole body is the decision; full trace in `docs/DS2-CONTINUE.md`.
//
// The chain the update walks, taken from its own instructions rather than from a struct:
//
//   mov rax,[0x14160de10]            ; FE_TITLE_CONTEXT
//   mov rdi,[rax+0x98]               ; FeGroupTitleDataList
//   movsxd rdx,[rax+0x564]           ; the selected slot
//   mov rax,[0x1416148f0]            ; GAME_MANAGER_IMP
//   mov rcx,[rax+0xa8]               ; GameDataManager
//   mov r8,[rcx+0xd8]                ; the slot array
//   cmp rdx,0xa / jae                ; ten slots, and a negative slot means none
//   imul rax,rax,0x1f0               ; stride
//   test BYTE PTR [rbx+0x1d9],0x1    ; occupied
//   mov edx,[rdi+0x28]               ; the group's confirmed action
// ============================================================================================

/// `FeSubStateTitleLoadDataList::v3` (update). RVA `0x000fba10`, VA `0x1400fba10`.
///
/// The single site worth instrumenting on the load path, because it is the only place the slot,
/// the action and the outgoing phase are all in scope at once. It runs per frame but does nothing
/// unless the substate's phase is 1, which its first three instructions establish:
/// `mov edx,[rcx+0x10]; dec edx; jne <return>`.
///
/// **Not an Arxan redirect.** `scripts/ds2-arxan-chain.py` reports `UNKNOWN` here only because its
/// prologue table does not carry `40 56` (`rex push rsi`); the entry is ordinary code, not the
/// five-byte `e9` a redirected entry holds.
pub const FE_SUBSTATE_LOAD_DATA_LIST_UPDATE: u32 = 0x000f_ba10;

/// The selected save slot, at `[`[`FE_TITLE_CONTEXT`]`] + 0x564`. Signed; `0..=9` selects, and
/// anything else means "none".
///
/// The Ghidra project names this field `_x564_slotNum` off the same evidence. It is read with
/// `movsxd` and immediately bounds-checked against `0xa`, so a mod writing it must respect both
/// the sign and the bound -- see [`SAVE_SLOT_COUNT`].
pub const FE_TITLE_CONTEXT_SLOT_NUM_OFFSET: usize = 0x564;

/// The `FeGroupTitleDataList` that owns the character list, at `[`[`FE_TITLE_CONTEXT`]`] + 0x98`.
pub const FE_TITLE_CONTEXT_DATA_LIST_GROUP_OFFSET: usize = 0x98;

/// The group's confirmed action, at `group + 0x28`. `1` backs out, `2` loads the selected slot.
///
/// Written by the group's own `vtable[4]`, which the update calls immediately before reading this.
/// `3` and `4` are also handled (they route to `0x56` and `0x5f`) but were not identified.
pub const FE_GROUP_DATA_LIST_ACTION_OFFSET: usize = 0x28;

/// The action value that means "load the selected slot".
pub const FE_DATA_LIST_ACTION_LOAD: i32 = 2;

/// Offset of `GameDataManager` in [`GAME_MANAGER_IMP`].
pub const GAME_DATA_MANAGER_OFFSET: usize = 0xa8;

/// Offset of the ten-slot save array inside `GameDataManager`.
pub const SAVE_SLOT_ARRAY_OFFSET: usize = 0xd8;

/// Stride of one save-slot record. `0x1F0`.
///
/// Confirmed far beyond this one function: the image holds 43 separate `imul reg,reg,0x1f0` sites
/// and nearly all of them are preceded by the same `cmp reg,0xa; jae` bound.
pub const SAVE_SLOT_STRIDE: usize = 0x1f0;

/// How many save slots the game indexes. Ten, enforced at every indexing site.
pub const SAVE_SLOT_COUNT: i32 = 10;

/// Flags byte within a save-slot record. `+0x1D9`.
///
/// **Runtime only.** It is zero in every record of the `id=4` section of a real `.sl2`, so it is
/// derived when the per-character entries are loaded rather than persisted. Reading a save file
/// cold to decide whether a slot exists must use entry content, not this byte.
pub const SAVE_SLOT_FLAGS_OFFSET: usize = 0x1d9;

/// [`SAVE_SLOT_FLAGS_OFFSET`] bit 0: the slot holds a character. The update nulls its record
/// pointer when this is clear.
pub const SAVE_SLOT_FLAG_OCCUPIED: u8 = 0x1;

/// [`SAVE_SLOT_FLAGS_OFFSET`] bit 1: the slot is excluded. Every one of the four action branches
/// returns without acting when this is set.
pub const SAVE_SLOT_FLAG_EXCLUDED: u8 = 0x2;

/// The ownership word within a save-slot record, `+0x1E8`, masked with
/// [`SAVE_SLOT_OWNERSHIP_MASK`].
///
/// On the load action the update passes `record[0x1e8] & 0x3f` to `0x140af6610`, and that result
/// picks phase 6 over phase 2 -- a different destination substate. **A continue flow must not skip
/// this call**: it is what refuses a character the running build cannot legitimately load.
pub const SAVE_SLOT_OWNERSHIP_OFFSET: usize = 0x1e8;

/// Mask applied to [`SAVE_SLOT_OWNERSHIP_OFFSET`] before the check. `0x3f`.
pub const SAVE_SLOT_OWNERSHIP_MASK: u32 = 0x3f;

/// `FeSubStateTitleLoadDataList`'s substate id: `0x55`. The character list.
pub const FE_SUBSTATE_ID_TITLE_LOAD_DATA_LIST: u32 = 0x55;

/// `FeSubStateTitleLoadProfile`'s substate id: `0x57`.
///
/// The destination of the load edge, registered by `FeSubStateTitleLoadDataList::v5`
/// (`0x1400fb1f0`) as the phase-2 transition. Identified through its constructor `0x1400faa10`,
/// which writes id `0x57` alongside vtable `0x1410bd658`.
pub const FE_SUBSTATE_ID_TITLE_LOAD_PROFILE: u32 = 0x57;

/// `FeSubStateTitleStartIngame`'s substate id: `0x6b`. The end of the load chain.
pub const FE_SUBSTATE_ID_TITLE_START_INGAME: u32 = 0x6b;

/// `FeSubStateTitleLoadDataList::v1` (enter). RVA `0x000fae80`, VA `0x1400fae80`.
///
/// Ghidra's project already names it `FUN_1400fae80_continueMenu`. It reads the list group from
/// `[`[`FE_TITLE_CONTEXT`]`]+0x98`, asks `0x1400f0f60` for the occupied slots, and on a non-empty
/// list calls `0x1400f1cb0(group, 1)` and sets its own phase to 1. On an empty list it sets phase
/// 3 and goes straight back to the top menu.
///
/// **The site to write a pre-selected slot at.** `0x1400f1cb0`'s `1` is a mode flag -- it lands in
/// `group+0x2c` and swaps the screen's text ids between load and delete -- so the cursor is not
/// chosen there. It is chosen downstream in `0x1400f1fa0`, and the list code both reads
/// [`FE_TITLE_CONTEXT_SLOT_NUM_OFFSET`] (`0x1400f2021`) and writes it back on a cursor move
/// (`0x1400f220e`, `0x1400f22d7`, `0x1400f238e`). Writing the field before this function runs
/// therefore reaches the list before it is built; writing it after would be overwritten.
///
/// Not an Arxan redirect: `scripts/ds2-arxan-chain.py` reports a clean prologue at the entry.
pub const FE_SUBSTATE_LOAD_DATA_LIST_ENTER: u32 = 0x000f_ae80;

/// The slot the game itself remembers, at `savedata + 0x1368` -- past the ten records, which end
/// at `0x1360`.
///
/// `FeSubStateTitleSteamLoadSystemData::v2` (`0x1400fb8d0`, substate `0x05`, early boot) copies it
/// into [`FE_TITLE_CONTEXT_SLOT_NUM_OFFSET`] when it is not negative, and `0x1400fdc82` copies it
/// back the other way. So the game has a remembered-slot mechanism of its own.
///
/// **It is not restored from the save file.** The `id=4` section of a real `.sl2` reads zero at
/// the corresponding offset even in a save written immediately after loading slot 1, so whatever
/// seeds this at boot, it is not the file. Recorded so the next reader does not re-derive that.
pub const SAVE_SLOT_CURRENT_INDEX_OFFSET: usize = 0x1368;

/// The call every accepting branch of `FeSubStateTitleLoadDataList::v3` makes before it writes a
/// phase. RVA `0x000f10e0`, VA `0x1400f10e0`, one argument: the list group.
///
/// `cmp qword [rcx+8],0; je ...` then a call through the object at `group+8`. Whatever it settles,
/// all four branches make it first, so a transition driven from outside must make it too.
///
/// Not an Arxan redirect; `ds2-arxan-chain.py` reports `UNKNOWN` only because `48 83 79` is
/// missing from its prologue table, as with `40 56` and `4c 8b 89`.
pub const FE_DATA_LIST_CLOSE: u32 = 0x000f_10e0;

/// Offset of the content/ownership context in [`GAME_MANAGER_IMP`]. A **pointer** field, read as
/// `mov rcx,[rax+0xc0]` at `0x1400fbb32`.
pub const GAME_MANAGER_CONTENT_CTX_OFFSET: usize = 0xc0;

/// Inside the content context, the object holding the owned-content masks. `+0x10`; a null here
/// means the gate passes.
pub const CONTENT_CTX_OWNED_OFFSET: usize = 0x10;

/// The two owned-content masks, OR'd together before the test. `+0x28` and `+0x30`.
///
/// The whole of `0x140af6610`, which the load branch calls and which this repo replicates with
/// pure reads rather than a call:
///
/// ```text
/// owned = (obj[0x30] | obj[0x28]) & required
/// refused = owned != required
/// ```
pub const CONTENT_OWNED_MASK_A: usize = 0x28;

/// The second owned-content mask. See [`CONTENT_OWNED_MASK_A`].
pub const CONTENT_OWNED_MASK_B: usize = 0x30;

/// The phase `FeSubStateTitleLoadDataList` writes to load the selected slot: `2`, which its own
/// transition table routes to [`FE_SUBSTATE_ID_TITLE_LOAD_PROFILE`].
pub const FE_DATA_LIST_PHASE_LOAD: i32 = 2;

/// The phase the character list writes when the player backs out: `3`, routed to
/// [`FE_SUBSTATE_ID_TITLE_TOP_MENU`].
///
/// One of the list's two ways of ending without loading anything, and so one of the two places a
/// shortcut that suppressed something for the duration of the load has to undo it.
pub const FE_DATA_LIST_PHASE_BACK: i32 = 3;

/// The phase written instead when the ownership gate refuses: `6`, routed to `0x5d`.
pub const FE_DATA_LIST_PHASE_REFUSED: i32 = 6;

/// `FeSubStateTitleTopMenu::v3` (update). RVA `0x000ff300`, VA `0x1400ff300`.
///
/// Three statements: poll the top-menu group at `[`[`FE_TITLE_CONTEXT`]`]+0x80`, copy the action
/// the group parked at `group+0xE8` into the substate's own phase, and zero
/// `savedata+0x136e` when that action is 4. **The phase and the action id are the same number**,
/// so the row-1 transition `FeSubStateTitleTopMenu::v5` registers for value
/// [`FE_TOP_MENU_ACTION_LOAD_GAME`] is taken by writing that value into the phase.
///
/// It rewrites the phase from the group on every frame, so a write made here survives exactly one
/// frame -- which is enough, because `FeStateFlow` evaluates transitions immediately after the
/// update returns. Writing the group's field instead would be cleared by the poll, the way the
/// character list's action field was.
///
/// Not an Arxan redirect: clean prologue at the entry.
pub const FE_SUBSTATE_TOP_MENU_UPDATE: u32 = 0x000f_f300;

/// The top menu's row-1 action, LOAD GAME, which routes to
/// [`FE_SUBSTATE_ID_TITLE_LOAD_DATA_LIST`]. `2`.
///
/// Registered unconditionally by the top menu's `v5`, unlike row 3's, so the transition exists
/// whether or not the row is drawn or selectable. Full row table in `docs/DS2-TITLE-FLOW.md`.
pub const FE_TOP_MENU_ACTION_LOAD_GAME: i32 = 2;

/// The top menu's resting phase, meaning "no row activated this frame". `0`.
pub const FE_TOP_MENU_PHASE_RESTING: i32 = 0;

// ---------------------------------------------------------------------------------------------
// Audio. The frontend never calls a named sound function, which is what made this hard to find:
// every earlier sweep looked for `call [rip+N]` into the FMOD IAT and found nothing. MSVC routes
// imports through `jmp [rip+N]` thunks, so the call sites are `call <thunk>` and an IAT-target
// scan misses all of them. Scanning for the thunks first turns up thirteen, and the levers the
// project had concluded were absent are all live.
//
// FMOD is NOT statically linked in this build. `fmodex64.dll` and `fmod_event64.dll` sit beside
// the exe and are imported by name.
// ---------------------------------------------------------------------------------------------

/// The one global holding a `MOFmodSoundManager*` (`DLMO`). RVA `0x0166dfa8`, VA `0x14166dfa8`.
///
/// Read at 17 sites and written at exactly one, `0x1409e5d00`, from the lazy accessor
/// `0x1409e5c90`: it allocates `0xce0` bytes, constructs with `0x1409da780`, and stores the
/// result. `0x1409ddbc0` is the fast path -- `mov rax,[this]; test rax,rax; je <init>; ret`.
///
/// The class is confirmed by RTTI rather than inferred: the vtable at `0x1411841b8` carries a
/// complete-object locator whose type descriptor names `.?AVMOFmodSoundManager@DLMO@@`, and both
/// functions below are slots in it.
pub const SOUND_MANAGER_SINGLETON: u32 = 0x0166_dfa8;

/// `MOFmodSoundManager` -> the master `FMOD::ChannelGroup*`. `0x9f8`.
///
/// **Written by the game's own init and read back by its own update**, which is what makes this
/// an identification rather than a guess:
///
/// * `MOFmodSoundManager::v6` (`0x1409ddbe0`, init) does
///   `lea rdx,[r15+0x9f8]; mov rcx,[r13]; call <System::getMasterChannelGroup>` at `0x1409df157`
///   -- so FMOD itself writes the master group into this field.
/// * `MOFmodSoundManager::v2` (`0x1409e0910`, the command-queue drain) does
///   `movss xmm1,[rdi+0x930]; mov rcx,[rdi+0x9f8]; call <ChannelGroup::setVolume>` at
///   `0x1409e0c8f` -- the image's **only** call to `ChannelGroup::setVolume`.
///
/// Both are methods of the same class on the same vtable, so the two `0x9f8` are the same field.
pub const SOUND_MANAGER_MASTER_GROUP_OFFSET: usize = 0x9f8;

/// `MOFmodSoundManager` -> the master volume the game itself last applied, `f32`. `0x930`.
///
/// The command drain stores its incoming float here (`movss [rdi+0x930],xmm1`) and reloads it
/// three instructions later to hand to `ChannelGroup::setVolume`. So it is not a cached copy of
/// something else -- it is the value the game means the master group to have, which is what makes
/// it the right thing to restore to. Reading it back beats writing `1.0`, which would silently
/// discard whatever the player set in the options menu.
pub const SOUND_MANAGER_MASTER_VOLUME_OFFSET: usize = 0x930;

/// IAT slot for `FMOD::ChannelGroup::setVolume(float)` in `fmodex64.dll`. RVA `0x01aae9b4`.
///
/// An import slot, not code: patching or reading it never touches `.text`, so Arxan's integrity
/// checks have nothing to see. Same property `ds2-offline` relies on for the WS2_32 slots.
///
/// Calling convention is MSVC `__thiscall` on x64: `rcx` is the `ChannelGroup*`, the float goes in
/// `xmm1`, and the return is an `FMOD_RESULT` (`0` == `FMOD_OK`).
pub const FMOD_CHANNEL_GROUP_SET_VOLUME_IAT: u32 = 0x01aa_e9b4;

/// `MOFmodSoundManager::v6`, audio init. RVA `0x009ddbe0`, VA `0x1409ddbe0`.
///
/// The function that *creates* [`SOUND_MANAGER_MASTER_GROUP_OFFSET`]: at `0x1409df157` it does
/// `lea rdx,[r15+0x9f8]` and hands that to `System::getMasterChannelGroup` as the out-parameter.
/// So on return from this function the master group exists and not one sound has played yet,
/// which makes it the earliest moment anything can be silenced.
///
/// Not an Arxan redirect: clean prologue at the entry.
pub const SOUND_MANAGER_INIT: u32 = 0x009d_dbe0;

/// `MOFmodSoundManager::v2`, the command-queue drain. RVA `0x009e0910`, VA `0x1409e0910`.
///
/// **The only code in the image that can change a channel group's volume.** Its `setVolume` call
/// at `0x1409e0c96` is the sole `ChannelGroup::setVolume` site, so anything holding the master
/// group at a chosen level only has to out-run this one function and nothing else.
///
/// Not an Arxan redirect: clean prologue at the entry.
pub const SOUND_MANAGER_COMMAND_DRAIN: u32 = 0x009e_0910;

/// `MOFmodSoundManager::v0`. RVA `0x009dfef0`, VA `0x1409dfef0`.
///
/// **NOT a per-frame pump, measured.** It contains the image's only `FMOD::EventSystem::update`
/// call (`0x1409e0080`), which FMOD documents as a once-a-frame requirement, and that made it look
/// like the frame pump. A detour on it fired essentially once in a 57-second run, during process
/// teardown -- so a mute re-asserted here is never asserted, and a restore requested here arrives
/// 51 seconds late. Kept as a named constant so the next reader does not repeat the inference.
///
/// Not an Arxan redirect. `scripts/ds2-arxan-chain.py` reported `UNKNOWN` until its prologue table
/// learned `48 8b c4` (`mov rax,rsp`); the entry is ordinary code, not a five-byte `e9` stub.
pub const SOUND_MANAGER_V0_NOT_A_FRAME_PUMP: u32 = 0x009d_fef0;

/// `FeSubStateTitleStartIngame::v1` (enter). RVA `0x000fde30`, VA `0x1400fde30`.
///
/// Slot 1 of vtable `0x1410bdbf8`, which RTTI names `FeSubStateTitleStartIngame`. This is the last
/// substate on the load chain -- the boundary the autocontinue shortcut ends at, and so the point
/// at which anything suppressed for the duration of that shortcut has to be given back.
///
/// Not an Arxan redirect: clean prologue at the entry.
pub const FE_SUBSTATE_START_INGAME_ENTER: u32 = 0x000f_de30;

// ---------------------------------------------------------------------------------------------
// The NOW LOADING screen, as a cover for the title flow.
//
// `FeOperatorNowLoading` is the operator behind `FeSceneNowLoading` / `FeGroupNowLoading` -- the
// full-screen loading page the game shows on every map transition. It is constructed once, during
// GameManagerImp's own init, and then sits idle until something makes it visible. That makes it
// available for the whole title flow, long before anything the title draws.
// ---------------------------------------------------------------------------------------------

/// `GameManagerImp` -> the frontend object that owns the operator table. `0x22e0`.
///
/// Written at `0x1401bcb79` (`mov [rdi+0x22e0],rsi`) in GameManagerImp's init, immediately after
/// `0x140500200` has populated the operator slots on that same object.
///
/// **Verified live** rather than only read: walking
/// `[`[`GAME_MANAGER_IMP`]`] + 0x22e0 + `[`FRONTEND_NOW_LOADING_OPERATOR_OFFSET`] in a running
/// game lands on an object whose vtable is `0x1410fa0c8`, which RTTI names
/// `FeOperatorNowLoading`. Note the container's own head reads as `DLKR::DLBackAllocator`, so it
/// embeds an allocator as its first member; the offset is what matters and the vtable at the end
/// of the walk is what confirms it.
pub const GAME_MANAGER_FRONTEND_ROOT_OFFSET: usize = 0x22e0;

/// That frontend object -> `FeOperatorNowLoading`. `0xc8`.
///
/// Filled by the lazy factory at `0x1405002de` (`mov [rbx+0xc8],rsi`) right after it allocates
/// `0x3c0` bytes and installs vtable `0x1410fa0c8`. The same pointer is mirrored into the
/// operator array at `+0x18`, which is how the frontend iterates operators.
pub const FRONTEND_NOW_LOADING_OPERATOR_OFFSET: usize = 0xc8;

/// That frontend object -> `FeOperatorTitle`. `0xd0`.
///
/// The neighbouring named slot to [`FRONTEND_NOW_LOADING_OPERATOR_OFFSET`], and read live at the
/// top menu: it holds an object whose vtable is `0x1410bc578`, which RTTI names
/// `FeOperatorTitle`. The same two operators are mirrored into the operator array at `+0x18`
/// (NowLoading) and `+0x30` (Title).
pub const FRONTEND_TITLE_OPERATOR_OFFSET: usize = 0xd0;

/// `FeOperatorBase` vtable slot 24 (`+0xc0`) -- show or hide one of an operator's screens.
///
/// `void slot24(this, u32 screen_id, bool show, float fade)`. Windows x64 puts those in `rcx`,
/// `edx`, `r8b` and `xmm3`, which is exactly what the game's own call site loads.
///
/// **This is read from a call site, not inferred from one.** `0x1405116f0` is the game's own
/// switch between the title and the loading screen, and it is a straight swap:
///
/// ```text
/// [param+2] == 1   Title.slot24(0x66, false, 0.0)   NowLoading.slot24(0x65, true,  0.0)
/// [param+2] != 1   Title.slot24(0x65, true,  0.0)   NowLoading.slot24(0x66, false, 0.0)
/// ```
///
/// So the ids are not "the loading screen" and "the title" -- both operators answer to both. The
/// id selects which of that operator's screens, and the operator supplies the content.
///
/// An earlier version of this constant named slot 4 and called it opacity, on the evidence that
/// the operator factory calls slot 4 twice with `0.0f` right after construction. That inference
/// was wrong: setting it to `1.0` at the title changed nothing on screen. Slot 4 is kept out of
/// this file entirely rather than left around to be believed again.
pub const FE_OPERATOR_SET_SCREEN_VTABLE_SLOT: usize = 24;

/// The screen id an operator is asked to SHOW when it takes over. `0x65`.
pub const FE_OPERATOR_SCREEN_ID_SHOW: u32 = 0x65;

/// The screen id an operator is asked to HIDE when it gives way. `0x66`.
pub const FE_OPERATOR_SCREEN_ID_HIDE: u32 = 0x66;

/// `FeOperatorTitle::v2`, the title operator's setup. RVA `0x000ef030`, VA `0x1400ef030`.
///
/// Thirty-two bytes: if `this+0x10` is non-null it runs two calls and writes `1` into
/// [`FE_OPERATOR_TITLE_ACTIVE`]. It is the moment the title frontend becomes live, which makes it
/// the earliest point at which covering the title is both possible and meaningful.
///
/// Not an Arxan redirect: clean prologue at the entry.
pub const FE_OPERATOR_TITLE_SETUP: u32 = 0x000e_f030;

/// `FeTitleContext` -> `FeGroupTitleTopMenu`. `0x80`.
///
/// Read by `FeSubStateTitleTopMenu::v3` on its first two instructions --
/// `mov rax,[0x14160de10]; mov rbx,[rax+0x80]` -- and then polled through its own vtable. Not to be
/// confused with [`FE_TOP_MENU_GROUP_OFFSET`], which is where the same group hangs off
/// `FeSceneTitle` rather than off the title context.
pub const FE_TITLE_CONTEXT_TOP_MENU_GROUP_OFFSET: usize = 0x80;

/// `FeGroupBase::close(group)`. RVA `0x000f18b0`, VA `0x1400f18b0`.
///
/// Nineteen instructions, and the whole of the frontend's "make this screen go away":
///
/// ```c
/// scene = group->_0x08;
/// if (scene && group->_0x30) {          // only when the group is open
///     play_sequence(scene, 0x68, 0, 0.0f);
///     scene->_0x18 += 1;
///     group->_0x30 = 0;
/// }
/// ```
///
/// Its mirror is the open at `0x1400f1cb0`, which plays sequence `0x66` and decrements the same
/// counter. `ds2-dialog-skip` already plays `0x67` on `FeSceneTitle` for the settled state, so
/// `0x66`/`0x67`/`0x68` are one family: open, settled, close.
///
/// **This is what the `0x65`/`0x66` at `0x1405116f0` really were** -- sequence ids handed to a
/// scene, not screen ids handed to an operator. Reading them as operator arguments produced a call
/// through vtable slot 24 of an eleven-slot vtable, which read string data and crashed the game.
///
/// Called, never patched, so its Arxan status does not arise -- but it is clean at its entry
/// anyway.
pub const FE_GROUP_CLOSE: u32 = 0x000f_18b0;

/// `FeGroupBase` -> the byte that is `1` while the group is open. `0x30`.
///
/// Set by the open at `0x1400f1cf2` and cleared by the close at `0x1400f18e1`, and tested at the
/// head of both, so each is idempotent on its own. That is what makes closing a group safe to do
/// from a per-frame detour: the second call does nothing.
pub const FE_GROUP_OPEN_FLAG_OFFSET: usize = 0x30;

/// `FeGroupTitleTopMenu::close(group)`. RVA `0x000f3590`, VA `0x1400f3590`.
///
/// **The top menu does not use [`FE_GROUP_CLOSE`].** It has its own pair, and
/// `FeSubStateTitleTopMenu` names both of them by using them:
///
/// * `v1` (enter, `0x1400fde90`): `mov rcx,[0x14160de10]; mov rcx,[rcx+0x80]; call 0x1400f3820`
/// * `v2` (leave, `0x1400feb50`): the same two loads, then `call 0x1400f3590`
///
/// So this is literally what the game calls when the top menu goes away, on the pointer it reads
/// from the same global at the same offset.
///
/// HOW THE WRONG ONE WAS CAUGHT, because it is a cheap trick worth repeating: the first attempt
/// called [`FE_GROUP_CLOSE`] on this group and logged [`FE_GROUP_OPEN_FLAG_OFFSET`] beside it. The
/// data list read `1` -- a clean boolean -- and was hidden. The top menu read `248`, which is not a
/// boolean, and stayed visible. Logging the field a call depends on is what turned "it didn't
/// work" into "that byte is not what I think it is, so this is the wrong class".
pub const FE_GROUP_TITLE_TOP_MENU_CLOSE: u32 = 0x000f_3590;

/// `FeSubStateTitleTopMenu::v1` (enter). RVA `0x000fde90`, VA `0x1400fde90`.
///
/// Loads the top-menu group from `[`[`FE_TITLE_CONTEXT`]`]+0x80` and calls `0x1400f3820`, the
/// group's **open**. Its mirror is `v2` (leave, `0x1400feb50`), which calls
/// [`FE_GROUP_TITLE_TOP_MENU_CLOSE`].
///
/// **DO NOT CLOSE THE GROUP FROM HERE.** Measured: closing it immediately after this `enter`
/// returns took boot-to-top-menu from 4.3s to 29.9s and then killed the process at the top menu,
/// before the character list was ever reached. The substate's update polls this group through its
/// own vtable on every frame, and closing it the same frame it opened leaves that poll working on
/// a group that has been shut. Closing from the update instead is stable across runs.
///
/// Kept as a constant because it names the open/close pair, not because anything hooks it.
///
/// Not an Arxan redirect: clean prologue at the entry.
pub const FE_SUBSTATE_TOP_MENU_ENTER: u32 = 0x000f_de90;

// ============================================================================================
// POSING THE TITLE SCREEN, which is what "hide the menu with the logo" actually needs.
//
// The three constants below replace a whole line of failed reasoning, so the correction is worth
// stating once, at the top, rather than three times below.
//
// `[`FE_TITLE_CONTEXT`]` + 0x80 IS A `FeSceneTitle`. Two crates had it as two different things --
// `ds2-dialog-skip` called it "the title scene" and `ds2-continue` called it "the top menu group"
// -- and both were reading the same eight bytes. It is a `FeSceneTitle`, and its constructor at
// `0x140f3391` settles it without a single inference: that function writes vtable `0x1410bcab0`
// (RTTI `FeSceneTitle`, 17 virtuals) to `[this]`, zeroes `[this+0xb8]` -- which is
// [`FE_TOP_MENU_GROUP_OFFSET`], the top-menu group hanging off the scene, exactly as that constant
// already claimed -- and zeroes a WORD at `[this+0xf0]`, whose second byte is the `+0xf1` that the
// open sets and the close clears.
//
// So ONE object carries the logo, the PRESS ANY BUTTON prompt and the six menu rows, which is why
// substate 0x17 and substate 0x47 both poll it, and why the player experiences them as one screen.
// `0x1400f3820` is that scene's OPEN (not "play sequence 0x67" -- it also builds every row), and
// `0x1400f3590` is its CLOSE.
// ============================================================================================

/// `FeGroupBase::v2(this)` -- pose this object's scene hidden, instantly. RVA `0x00505d40`.
///
/// The whole function, and it is null-checked before it touches anything:
///
/// ```c
/// scene = this->_0x08;
/// if (!scene) return;
/// 0x140afdb70(scene);                                   // a query; its result is discarded
/// play(scene, 0x65, /*pose=*/1, 0.0f);                  // tail call
/// ```
///
/// # Why this and not the close
///
/// [`FE_GROUP_TITLE_TOP_MENU_CLOSE`] plays `0x68` with the pose flag CLEAR, so it *animates* out
/// over the sequence's own span. Measured on screen: with that close called every frame from the
/// top-menu update, the menu stayed visible for the whole of the substate's ~25ms residency --
/// because a fade that needs ~14 frames cannot finish in one or two. **The close was working and
/// the animation was the problem.**
///
/// This is the same play with the flag SET, which is the difference between "start fading" and "be
/// faded". See [`FE_SEQUENCE_PLAY_FLAG_POSE`] for where that flag lands.
///
/// # Why the base and not `FeSceneTitle`'s own override
///
/// `FeSceneTitle::v2` (`0x1400f4190`) is exactly `call 0x140505d40(this)` followed by
/// `inc [this->_0x08 + 0x18]`. That increment is a counter the open decrements and the close
/// increments, and re-asserting a pose every frame through the override would run it up without
/// bound -- and a player who returns to the title screen from in-game would find it never comes
/// back. Calling the base skips the counter, so this leaves **no persistent state behind at all**:
/// nothing to restore, and the game's own next open re-shows the screen by itself.
///
/// # Why it is safe from a per-frame detour, unlike the close
///
/// It does not clear the `+0xf1` open flag and does not run the teardown at `0x1400f41b0`. Those
/// two are what [`FE_SUBSTATE_TOP_MENU_ENTER`] records as having taken boot-to-top-menu from 4.3s
/// to 29.9s and then killed the process: the substate's update polls a group the close had shut.
/// Nothing here shuts anything -- it changes a playback position and nothing else.
///
/// Called, never patched, so its Arxan status does not arise.
pub const FE_SCENE_TITLE_POSE_HIDDEN: u32 = 0x0050_5d40;

/// The sequence [`FE_SCENE_TITLE_POSE_HIDDEN`] plays. `0x65`.
///
/// It completes the family the frontend already had names for -- `0x66` open, `0x67` settled,
/// `0x68` close -- as the state *before* an open, which is what makes it the hidden pose.
///
/// **This is the `0x65` from `0x1405116f0`**, the one whose `(0x65, true, 0.0)` triple was read as
/// operator arguments last session and called through vtable slot 24 of an eleven-slot vtable,
/// which read string data and crashed the game. The arguments were right all along; the receiver
/// was not. Here it arrives on the receiver the game's own code uses.
pub const FE_SCENE_TITLE_SEQUENCE_HIDDEN: i32 = 0x65;

/// The play's third argument, when the caller wants a pose rather than an animation. `1`.
///
/// Read out of `FeComponentSprite`'s slot 24 (`0x140b6c4f0`) rather than guessed, because the flag
/// is inverted on its way to the field it controls:
///
/// ```text
/// xmm6 = (float)entry.start + seek      // the seek is relative; see FE_TOP_MENU_SEQUENCE_FADED_SEEK
/// [this+0x40] = xmm6                    // the playback position
/// test dil,dil                          // dil is THIS flag
/// sete dl                               // dl = (flag == 0)
/// call [vtable+0xb0](this, dl)          // slot 22, "keep playing"
/// ```
///
/// So `0` means play on from here and `1` means hold here. Every animated play in the frontend
/// passes `0`; [`FE_SCENE_TITLE_POSE_HIDDEN`] is the one that passes `1`.
///
/// **This is what makes the seek irrelevant.** Posing the FIRST frame of the hidden sequence needs
/// no offset, so none of the sequence-span arithmetic that
/// [`FE_TOP_MENU_SEQUENCE_FADED_SEEK`] had to establish applies here -- and that arithmetic could
/// not have been done statically anyway, since the spans live in `GameDataEbl.bdt`.
pub const FE_SEQUENCE_PLAY_FLAG_POSE: i32 = 1;

/// `FeSceneTitle::open(scene)` -- what raises the title screen. RVA `0x000f3820`.
///
/// ```text
/// if (!this->_0xf1) {                       // not already open
///     scene = this->_0x08;
///     if (scene) {
///         scene->_0x18 -= 1;
///         play(scene, 0x67, 0, 0.0f);       // the settled pose
///         this->_0xf1 = 1;
///     }
/// }
/// ... ~1000 further bytes: component lookups, row construction, more plays ...
/// ```
///
/// # This is the same address as `FE_SCENE_TITLE_PLAY_IDLE`, which is misnamed
///
/// `ds2-dialog-skip` calls it as "play sequence `0x67` on the title scene". It is the screen's
/// whole open, rows and all, and `[title_skip] title_settle` defaults ON -- so **the mod itself
/// opens the title screen during substate `0x17`**, long before `FeSubStateTitleTopMenu` runs.
/// Tracked as `ds2-mods-rs-ebj`; the duplicate constant is left in place until that lands rather
/// than editing another crate's call site from here.
///
/// # Why this is the site to hook to hide the screen
///
/// MEASURED, from the log's own ordering rather than from reasoning about frames. The open logs at
/// line 49 (`settled screen=title-main`) and a pose driven from `FeSubStateTitleTopMenu`'s update
/// logs at line 60, with the entire network and dialog chain in between. The process windows cover
/// the screen for most of that gap, which is why it reads as a brief flash rather than seconds of
/// menu -- what is seen is the window between the last dialog clearing and substate `0x47`
/// arriving. Hooking the open closes that gap by construction: whatever raises the screen --
/// `title_settle` at `0x17`, or `FeSubStateTitleTopMenu::v1` at `0x47` -- is posed hidden on its
/// way out, so there is no interval in which it is up and un-posed.
///
/// # The one configuration this interacts with
///
/// Posing [`FE_SCENE_TITLE_SEQUENCE_HIDDEN`] leaves the scene's current sequence at `0x65`, and
/// the real gate at `0x1400f37f0` reports settled only for `0x67`. `ds2-dialog-skip` replaces that
/// gate outright when `title_sequence_skip` is on, which is the default, so the wait is already
/// forced and nothing observes the posed sequence. `title_settle` ON with `title_sequence_skip`
/// OFF is the combination that would wait forever at PRESS ANY BUTTON -- with `title_settle` off
/// the open does not happen until `0x47`, by which time the gate is long past.
///
/// Not an Arxan redirect: `scripts/ds2-arxan-chain.py` terminates at hop 0 with the clean prologue
/// `48 89 74 24 20` at the entry.
pub const FE_SCENE_TITLE_OPEN: u32 = 0x000f_3820;

/// `SaveLoadSystem`'s save-directory builder -- what produces the folder the `.sl2` lives in.
/// RVA `0x00248db0`.
///
/// ```text
/// FUN_140248db0(std::wstring *out, const wchar_t *subdir)
///     out  = "%APPDATA%\\DarkSoulsII\\"        // via SAVE_APPDATA_ROOT_BUILD
///     out += subdir                            // the Steam ID, as text
///     out += "\\"                              // DAT_1410d04f8, a lone backslash
/// ```
///
/// # Why this site and not the one it calls
///
/// [`SAVE_APPDATA_ROOT_BUILD`] is the wider chokepoint -- it is the only thing in the image that
/// turns `SHGetFolderPathW(CSIDL_APPDATA)` into a DARK SOULS II path -- but it is wider than the
/// job. Its other caller, `FUN_140248d80`, appends `GraphicsConfig_SOFS.xml`, so a detour there
/// moves the graphics config as well as the saves. This function has exactly two callers and both
/// are `SaveLoadSystem` methods (`FUN_1402e6230_saveLoadSetup__` at `0x1402e635c`,
/// `FUN_1402e67f0` at `0x1402e6930`), so hooking it reaches the saves and nothing else.
///
/// # The second argument is the Steam ID, established from the call site
///
/// At `0x1402e6331` the caller makes a virtual call through slot `+0x38` -- the same slot
/// `FUN_140af14e0` uses to fill the cached Steam ID at `DAT_1416681a8` -- converts the result to a
/// string, and hands its character data to this function in `rdx`:
///
/// ```text
/// 0x1402e634a:  cmp    QWORD PTR [rbp-0x19],0x8      // the wstring's capacity field
/// 0x1402e634f:  lea    rdx,[rbp-0x31]                // ... so rdx is the inline buffer,
/// 0x1402e6353:  cmovae rdx,QWORD PTR [rbp-0x31]      // ... or the heap pointer when it spilled
/// 0x1402e6358:  lea    rcx,[rbp-0x1]                 // the out string
/// 0x1402e635c:  call   0x140248db0
/// ```
///
/// That is why the observed layout is `…\DarkSoulsII\<steamid hex>\DS2SOFS0000.sl2` with the
/// graphics config a level above it, and it is why a detour here owns the Steam ID folder too: a
/// redirect can point at a donor save's own folder name instead of renaming it to the running
/// account's.
///
/// **A replacement must end in a backslash.** The caller appends the file name to whatever this
/// leaves behind, and the trailing separator is this function's job, not the caller's.
///
/// Not an Arxan redirect: `scripts/ds2-arxan-chain.py` terminates at hop 0 with the clean prologue
/// `48 89 5c 24 08` at the entry.
pub const SAVE_DIR_BUILD: u32 = 0x0024_8db0;

/// `%APPDATA%\DarkSoulsII\` -- the root every DS2 user path is built on. RVA `0x00248e80`.
///
/// `SHGetFolderPathW(0, 0x1a /* CSIDL_APPDATA */, 0, 0)`, then append the literal
/// `L"\\DarkSoulsII\\"` at `0x1410d04a8`. Recorded because it anchors [`SAVE_DIR_BUILD`] and
/// because it is the site to hook if the graphics config should move as well -- it is deliberately
/// NOT the site this crate hooks. See [`SAVE_DIR_BUILD`] for why.
///
/// Not an Arxan redirect: clean prologue `48 89 5c 24 10` at the entry.
pub const SAVE_APPDATA_ROOT_BUILD: u32 = 0x0024_8e80;

/// `std::wstring::assign(dst, src, len)` -- the game's own assign, in its own CRT. RVA `0x000260b0`.
///
/// Reused rather than reimplemented, and that is the point: the out-parameter of
/// [`SAVE_DIR_BUILD`] is a live MSVC `std::basic_string<wchar_t>` owned by the caller, which may
/// already hold a heap allocation from the game's allocator. Writing its fields by hand would
/// leak that allocation or free it with the wrong allocator; calling the game's assign hands both
/// problems back to the code that owns them. It is the same function
/// `SAVE_APPDATA_ROOT_BUILD` itself calls to seat the `SHGetFolderPathW` result.
///
/// `len` is in `wchar_t`, not bytes, and excludes the terminator.
pub const WSTRING_ASSIGN: u32 = 0x0002_60b0;

/// Byte offset of the length field in the game's `std::wstring`. Length is in `wchar_t`.
///
/// Read out of `FUN_140043050`/`FUN_1400260b0`, which index `_x10_strLen_` at this offset and
/// `_x18_strCapacity_` at [`WSTRING_CAPACITY_OFFSET`], and treat the first sixteen bytes as a
/// union of an inline buffer and a pointer. That is stock MSVC small-string optimisation.
pub const WSTRING_LEN_OFFSET: usize = 0x10;

/// Byte offset of the capacity field in the game's `std::wstring`.
///
/// The discriminant for the small-string union: at or below [`WSTRING_SSO_MAX`] the characters
/// live inline at offset 0, above it offset 0 is a pointer to them. Both string helpers branch on
/// exactly this, and so does the call site documented on [`SAVE_DIR_BUILD`].
pub const WSTRING_CAPACITY_OFFSET: usize = 0x18;

/// Largest capacity that still lives in the inline buffer -- seven `wchar_t` plus a terminator.
///
/// The helpers spell this as `if (7 < capacity) { use the pointer }`, and the disassembled call
/// site as `cmp QWORD PTR [rbp-0x19],0x8` / `cmovae`. Recorded so a reader of a live string does
/// not have to rediscover which side of the comparison is the heap.
pub const WSTRING_SSO_MAX: usize = 7;

// ---------------------------------------------------------------------------------------------
// THE IN-GAME (PAUSE) MENU'S TAB ITEM LISTS
//
// `FeGroupInGameTopSelect` (ctor RVA `0x000a41b0`) owns six `FeGroupInGameGroupSelect` members --
// the six tabs. Each tab's contents are a `DLKR::DLFixedVector` of 8-byte entries built by a
// dedicated function, one per tab, called from that ctor:
//
//   RVA        entries (action, gate)              what the actions open
//   0x000a4990 (0,0)                               FeGroupInGameMenuEquipTop
//   0x000a4db0 (1,0)                               FeGroupInGameMenuInventory2
//   0x000a5620 (2,0) (3,0)                         FeGroupInGameMenuStatusStatus / ...StatusInfo
//   0x000a4fc0 (4,1) (5,2) (6,3)                   FeGroupIngameMessageWrite / ReadHistory / WriteHistory
//   0x000a5900 (7,0) (8,0) (9,4)                   SystemSettingGame / SystemSettingScreen / ReturnTitleCheck
//   0x000a5330 (0xb,0) (0xc,0)                     SystemSettingKeyboard / SystemSettingGraphic
//
// Read out of the six builders and out of the dispatch switch [`FE_INGAME_MENU_DISPATCH`], with
// each destination class confirmed by walking its vtable back to its RTTI type descriptor. None of
// the six builders, the ctor, the dispatch or the per-tab init is an Arxan redirect --
// `scripts/ds2-arxan-chain.py` terminates at hop 0 with a clean prologue on every one of them.
// ---------------------------------------------------------------------------------------------

/// The builder for the tab that carries the quit item. RVA `0x000a5900`.
///
/// `FeGroupInGameTopSelect`'s ctor calls this once, with a stack descriptor in RCX, and it is the
/// ONLY caller -- one candidate RIP-relative reference in the whole image (`0x1400a4382`, inside
/// that ctor), so a detour here reaches this tab and nothing else. It returns its argument in RAX.
///
/// It zeroes the count at [`FE_INGAME_MENU_ITEM_VECTOR_COUNT_OFFSET`] and then pushes three
/// entries -- [`FE_INGAME_MENU_SYSTEM_TAB_ITEMS`]. The entry the player calls "quit" is the third,
/// action [`FE_INGAME_MENU_ACTION_RETURN_TITLE`], which the dispatch resolves to
/// `FeInGameMenuWarehouse + 0x6f10`, the `FeGroupInGameReturnTitleCheck` the warehouse's ctor
/// (`0x1400991e0`) constructs at member `+0xde2`.
///
/// Not an Arxan redirect: clean prologue [`FE_INGAME_TOP_SELECT_SYSTEM_TAB_ITEMS_PROLOGUE`] at the
/// entry.
pub const FE_INGAME_TOP_SELECT_SYSTEM_TAB_ITEMS: u32 = 0x000a_5900;

/// The first six bytes of [`FE_INGAME_TOP_SELECT_SYSTEM_TAB_ITEMS`]: `rex push rbx` /
/// `sub rsp,0x50`.
///
/// Recorded so a detour can REFUSE rather than patch when the bytes are not these. That is the
/// only defence against this table being read against a different build: an RVA is just a number
/// and will happily point into the middle of some other function.
pub const FE_INGAME_TOP_SELECT_SYSTEM_TAB_ITEMS_PROLOGUE: [u8; 6] =
    [0x40, 0x53, 0x48, 0x83, 0xec, 0x50];

/// The dispatch: `switch (action)` over every in-game menu item. RVA `0x000a6090`.
///
/// Reached from the tab's confirm handler (`0x1400a6b10`), which reads the entry under the cursor,
/// applies its gate, and passes either the action or `-1`. `-1` selects a different sound id and
/// falls through the switch's `default`, which is how a gated row refuses.
///
/// Cases 0-9, 0xb, 0xc and **0xd** are present. Two shapes: a direct
/// `FeInGameMenuWarehouse` member (0, 2, 4, 5, 6, 9), or a `FexDynamicGroupExecJob` carrying a
/// *kind* that the factory at `0x1400a67c0` turns into a freshly allocated group (3 -> kind 0,
/// 7 -> 2, 8 -> 3, 0xb -> 4, 0xc -> 5, 0xd -> 6). Action `0xa` has no case at all.
///
/// Recorded for provenance and because it is the site to extend for a genuinely new action. Not
/// currently hooked by anything.
///
/// Not an Arxan redirect: clean prologue `48 89 5c 24 18` at the entry.
pub const FE_INGAME_MENU_DISPATCH: u32 = 0x000a_6090;

/// The per-tab init that turns the item list into rows. RVA `0x000a4d20`.
///
/// It binds the grid to the layout ([`FEX_GRID_CONTROL_LAYOUT_BIND`]), then calls
/// `FUN_140021b30(tab, tab->count)`, then the availability pass `0x1400a77c0`, which walks
/// `0..count`, reads entry `i`, and greys the row whose gate refuses.
///
/// **`FUN_140021b30` sets the count the CURSOR is bounded by, not the number of drawable cells.**
/// A run on 2026-08-28 appended a fourth entry to the quit tab and got exactly that: a fourth item
/// the cursor reaches and that responds, with nothing drawn for it. The drawable cells were already
/// fixed by the bind on the line above. This comment used to claim the visible row count came from
/// here; it does not, and `docs/DS2-INGAME-MENU.md` keeps the wrong version beside the right one.
///
/// Not hooked. Not an Arxan redirect: clean prologue `40 53 48 81 ec b0 00 00 00` at the entry.
pub const FE_INGAME_MENU_TAB_INIT: u32 = 0x000a_4d20;

/// `FrontendEx::FexGridControl`'s layout bind -- where a grid's drawable cells come from. RVA
/// `0x000216d0`.
///
/// It takes no extent from anywhere. It DISCOVERS one, by asking the layout for the element at
/// each `(col, row)` and stopping a row at the first one that comes back null:
///
/// ```text
/// for (row = 0; row < 15; row++)
///   for (col = 0; col < 32; col++) {
///       element = (*namer->vtable[0x10])(col, row);
///       if (element == 0) break;                        // this row ends here
///       cell = FUN_14010a060(...);                      // a drawable cell object
///       grid[FEX_GRID_COL_EXTENT_OFFSET] = max(that, col + 1);
///       grid[FEX_GRID_ROW_EXTENT_OFFSET] = max(that, row + 1);
///   }
/// ```
///
/// So the extents are a count of AUTHORED LAYOUT ELEMENTS, not a constant to raise, and the probe
/// stops at the first hole -- authoring cell 4 without cell 3 would find neither. `0x140022160`
/// closes the loop from the other side: resolving a cell whose column equals the column extent
/// takes the `vtable+0x48` one-past-the-end naming branch instead of the ordinary-cell branch.
///
/// Recorded because it is the answer to "why is the appended row invisible", and because it is
/// where anyone extending a menu with new layout data has to look. Not hooked.
pub const FEX_GRID_CONTROL_LAYOUT_BIND: u32 = 0x0002_16d0;

/// Byte offset of a `FexGridControl`'s logical item count -- what the cursor may reach.
///
/// Written by `FUN_140021b30`, read by the grid's `v34` (`0x14001c020`, `return this->+0x38` on the
/// `FexGroupList` base subobject, which lands here on the whole object). This is the field an
/// appended item moves.
pub const FEX_GRID_ITEM_COUNT_OFFSET: usize = 0xc8;

/// Byte offset of a `FexGridControl`'s COLUMN extent -- how many drawable cells the layout gave it.
///
/// Set only by [`FEX_GRID_CONTROL_LAYOUT_BIND`], as a running `max` over the elements it found.
/// `FUN_140021b30` never touches it, which is the whole gap between an item being selectable and
/// an item being visible.
pub const FEX_GRID_COL_EXTENT_OFFSET: usize = 0xd4;

/// Byte offset of the scroll state a `FexGridControl` drives. RVA-relative to the grid.
///
/// `FUN_140021b30` reads it as `grid[0x1c]` and writes the total at `+0x2c`, compares that total
/// against `+0x28`, and takes one of two branches: total at or below `+0x28` plays sequence `0x7a`
/// on the object at `[scroll]`, above it plays `0x70` and computes a thumb size from `+0x44`.
/// That is a scrollbar being hidden or shown, which is what makes "how many cells are VISIBLE"
/// a different number again from the extent and the item count.
pub const FEX_GRID_SCROLL_OFFSET: usize = 0xe0;

/// Within the scroll object: the number of cells on screen at once.
pub const FEX_GRID_SCROLL_VISIBLE_OFFSET: usize = 0x28;

/// Within the scroll object: the total the count above is compared against.
pub const FEX_GRID_SCROLL_TOTAL_OFFSET: usize = 0x2c;

/// Byte offset of a `FexGridControl`'s ROW extent. Set the same way as
/// [`FEX_GRID_COL_EXTENT_OFFSET`].
///
/// `1` means a single-line list, and `0x1400222c0` special-cases it: index `n` maps to
/// `(col = n, row = 0)`. The in-game menu tabs are that shape, so their items run along the column
/// axis and [`FEX_GRID_COL_EXTENT_OFFSET`] is the one that bounds them.
pub const FEX_GRID_ROW_EXTENT_OFFSET: usize = 0xd8;

/// Byte offset of a tab's item `DLFixedVector` inside the constructed
/// `FeGroupInGameGroupSelect`.
///
/// `FUN_1400a40e0` copies the builder's stack descriptor in with
/// `FUN_1400a3ef0(param_1 + 0x1f, descriptor)` -- qword `0x1f` -- and
/// [`FE_INGAME_MENU_TAB_ITEM_LOOKUP`] addresses the same storage as `lea rcx,[rbx+0xf8]`. Two
/// spellings, one offset.
pub const FE_INGAME_MENU_TAB_ITEM_VECTOR_OFFSET: usize = 0xf8;

/// Byte offset of the element count inside a tab's item `DLFixedVector`.
///
/// Every one of the six builders opens with `mov QWORD PTR [rcx+0x30], 0`, and the copy into the
/// live group (`0x1400a3ef0`) reads and writes the same field. In the constructed group the pair
/// lands at `+0xf8` (elements) and `+0x128` (count), which is `0xf8 + 0x30` -- the same struct.
pub const FE_INGAME_MENU_ITEM_VECTOR_COUNT_OFFSET: usize = 0x30;

/// Most entries a tab's item vector can hold.
///
/// Spelled by the builders as `if (5 < newCount) panic("out of memory.")` against
/// `DLFixedVector.inl:0x24c`, and independently by the copy at `0x1400a3ef0`, which panics unless
/// the source count is `< 6`. Both agree: five.
///
/// **It is no longer the ceiling on ROWS, and it never was a ceiling on anything but the vector.**
/// The storage is inline -- elements at `descriptor + (-descriptor & 3) + n * 8` with the count at
/// `+0x30`, so element 6 would land on the count -- but an item is only ever READ through
/// [`FE_INGAME_MENU_TAB_ITEM_LOOKUP`], and a detour there serves an entry from storage of our own.
/// See the section above that constant.
pub const FE_INGAME_MENU_ITEM_VECTOR_CAPACITY: usize = 5;

/// Size of one item entry: a `u32` action id followed by a `u32` gate index.
///
/// The builders write a whole 8-byte slot per push (`*puVar1 = 0x400000009` for the quit entry --
/// action `9`, gate `4`), and the two readers split it: the confirm path takes the action from
/// `[entry]` and the gate from `[entry+4]` (`lea rcx,[rax+4]` at `0x1400a4cce`).
pub const FE_INGAME_MENU_ITEM_STRIDE: usize = 8;

/// The three entries [`FE_INGAME_TOP_SELECT_SYSTEM_TAB_ITEMS`] is expected to leave behind, as
/// `(action, gate)` pairs.
///
/// Held so a detour can CHECK what the original produced before touching it. Without that, a
/// build whose tabs are ordered differently would be silently modified in the wrong place, and
/// the resulting screenshot would be evidence about nothing.
pub const FE_INGAME_MENU_SYSTEM_TAB_ITEMS: [(u32, u32); 3] = [
    (
        FE_INGAME_MENU_ACTION_SETTING_GAME,
        FE_INGAME_MENU_GATE_ALWAYS,
    ),
    (
        FE_INGAME_MENU_ACTION_SETTING_SCREEN,
        FE_INGAME_MENU_GATE_ALWAYS,
    ),
    (
        FE_INGAME_MENU_ACTION_RETURN_TITLE,
        FE_INGAME_MENU_GATE_RETURN_TITLE,
    ),
];

/// Action `7` -- `FeGroupInGameSystemSettingGame`, via dynamic-group kind 2.
pub const FE_INGAME_MENU_ACTION_SETTING_GAME: u32 = 7;

/// Action `8` -- `FeGroupInGameSystemSettingScreen`, via dynamic-group kind 3.
pub const FE_INGAME_MENU_ACTION_SETTING_SCREEN: u32 = 8;

/// Action `9` -- `FeGroupInGameReturnTitleCheck`. **This is the quit item.**
///
/// The dispatch resolves it to `warehouse + 0x6f10`, and the warehouse's ctor builds
/// `FeGroupInGameReturnTitleCheck` (vtable `0x1410b1768`, ctor `0x14006e390`) at exactly that
/// member. It is the confirm dialog that offers to save on the way to the title screen.
pub const FE_INGAME_MENU_ACTION_RETURN_TITLE: u32 = 9;

/// Action `0xd` -- present in the dispatch, listed by **no** tab.
///
/// Its factory branch shares a `case` label with kind 4: `case 4: case 6:` both allocate `0xc68`
/// bytes and call `FUN_1400803b0`, whose vtable writes name `FeGroupInGameSystemSettingKeyboard`.
/// The job carries the kind ONLY to select that branch (`mov edx,[rbx+0x28]` at `0x14002d884`,
/// its one and only read), so executing action `0xd` is byte-for-byte what executing the shipped
/// Key Bindings row already does every time it is pressed.
///
/// That is the entire reason it is the probe's payload: it adds a row without adding a code path.
pub const FE_INGAME_MENU_ACTION_KEY_BINDINGS_UNUSED: u32 = 0x0d;

/// Gate `0` -- no gate. The predicate at `0x1400a4e50` returns `0` (selectable) immediately on it.
pub const FE_INGAME_MENU_GATE_ALWAYS: u32 = 0;

/// Gate `4` -- the one the quit item carries.
///
/// Resolves the session object at `GameManagerImp + 0x22f0` through `FUN_140513270` and asks
/// `FUN_14025f690` about it; a nonzero answer means the row is refused. Neither callee is named in
/// the project yet, so what it actually forbids is NOT recorded here -- only that this is the gate
/// the shipped quit row uses.
pub const FE_INGAME_MENU_GATE_RETURN_TITLE: u32 = 4;

/// The gate predicate itself. RVA `0x000a_4e50`, `bool refused(const u32 *gate)`.
///
/// It takes a pointer to the gate index rather than the index. The confirm handler reaches it with
/// `lea rcx,[rax+4]` -- the second `u32` of the item-vector entry -- and the body opens
/// `if (*param_1 == 0) return false`, which is what makes gate `0` mean "no gate". `true` is
/// refused; the confirm path turns that into the action `-1` that falls out of the dispatch's
/// `switch` through its `default`.
///
/// Recorded because anything that fires a shipped action without going through the tab's own
/// confirm handler has to apply the gate itself, or it is not doing what the row does. The function
/// only reads -- `GameManagerImp + 0x22f0`, the net-server manager, and two predicates on the
/// session -- so calling it costs nothing and forges nothing.
pub const FE_INGAME_MENU_GATE_EVALUATE: u32 = 0x000a_4e50;

/// The five bytes [`FE_INGAME_MENU_GATE_EVALUATE`] must begin with.
///
/// `rex push rbx; sub rsp,0x20`, read out of the image rather than assumed from the shape of its
/// neighbours -- the first guess at these bytes was the other common MSVC opener and was wrong.
/// `scripts/ds2-arxan-chain.py 0x1400a4e50` reports a clean prologue at the entry, so these are
/// what the live process holds.
pub const FE_INGAME_MENU_GATE_EVALUATE_PROLOGUE: [u8; 5] = [0x40, 0x53, 0x48, 0x83, 0xec];

// ---------------------------------------------------------------------------------------------
// A SEVENTH TAB: WHERE THE SIX IS WRITTEN DOWN, AND WHAT EACH SPELLING COSTS
//
// This block used to say a seventh tab could not exist, and it was wrong in the way a wall is
// wrong when it turns out to be a door with five locks. Every "bound" below is a literal inside a
// function, and a literal inside a function is a detour site. The row work proved the pattern on
// two of them already.
//
// A tab's item vector is per tab -- `FUN_1400a40e0` copies each builder's stack descriptor into
// the group it is constructing with `FUN_1400a3ef0(group + 0x1f, descriptor)`, which is
// `group + 0xf8` -- so a seventh tab starts with five empty slots of its own before any of the row
// machinery is involved.
//
// The six spellings, and the answer to each:
//
//   1. The six groups are inline members of `FeGroupInGameTopSelect`, at
//      [`FE_INGAME_TOP_SELECT_TAB_OFFSETS`] with stride [`FE_INGAME_TOP_SELECT_TAB_STRIDE`], and
//      that object is itself an inline member of `FeSceneInGame` (`FUN_1400995a0` calls
//      `FUN_1400a41b0(scene + 0x28, ..)` in qwords -- `scene + 0x140`). A seventh would begin at
//      [`FE_INGAME_TOP_SELECT_AFTER_TABS`], where the same constructor already builds an element
//      accessor. -> Do not put it there. A group is [`FE_INGAME_GROUP_SELECT_SIZE`] bytes and
//      [`FE_INGAME_GROUP_SELECT_CTOR`] will construct one anywhere, so it goes in storage we own.
//      This is the only genuinely new mechanism of the six.
//   2. Navigation is a six-entry stack table of `this + literal` behind a `< 6` guard --
//      [`FE_INGAME_TOP_SELECT_TAB_TABLE`], which returns null for index 6. -> Detour it. Measured:
//      five callers plus one vtable reference at `0x1418a53f4` and nothing else, so it is the
//      funnel every consumer goes through rather than one of several ways in.
//   3. The tab strip's cell namer ([`FE_INGAME_TOP_SELECT_NAMER`]) pushes exactly six ids through
//      [`FE_SCENE_NAMER_PUSH`], which panics at seven. -> The same stand-in
//      [`FE_SCENE_NAMER_CELL_LOOKUP`] already serves row cells from, one level up.
//   4. [`FE_INGAME_TOP_SELECT_STRIP_INIT`] sets the strip's item count with a literal
//      `FUN_140021b30(this, 6)` and walks an unrolled six-pointer array of the same addresses as
//      (2). -> The same count raise already detoured at [`FE_INGAME_MENU_TAB_INIT`]. The strip is
//      itself a `FexGridControl`, so its real ceiling is [`FEX_GRID_MAX_COLS`].
//   5. [`FE_INGAME_TOP_SELECT_TAB_CAPTION_PATH`] holds a five-entry table and hands back an empty
//      accessor above index 4. -> Nothing to do. There are six tabs and five entries, so index 5
//      already takes the empty arm today; index 6 behaves exactly as the shipped System tab does.
//   6. The strip's `.flo` container [`FLO_TAB_STRIP_DEFINITION`] carries six cell records and a
//      child count that is also the display-list capacity. -> The same child-count raise and
//      record append `ds2-menu-row`'s layout module already performs on a tab's row container.
//
// So one new mechanism and five repeats. What the six bounds really are is a list of every place
// the number six is written down, which is what a build needs and is why they are kept here.
//
// The two per-tab vectors are still inline storage that cannot be grown -- both are one element
// short of overwriting their own count:
//
//   vector   elements at `descriptor + (-descriptor & 3) + n * 8`, count at `+0x30`
//            -> element 6 lands exactly on the count
//   namer    entries at `list + (-list & 7) + n * 0x30`, count at `list + 0x128`
//            -> entry 6 spans the count
//
// -- which is why a tab's rows are served through [`FE_INGAME_MENU_TAB_ITEM_LOOKUP`] and
// [`FE_SCENE_NAMER_CELL_LOOKUP`] past that point, leaving [`FEX_GRID_MAX_ROWS`] as the only bound.
// ---------------------------------------------------------------------------------------------

/// Most rows a `FrontendEx::FexGridControl` will ever bind cells for. **Fifteen.**
///
/// [`FEX_GRID_CONTROL_LAYOUT_BIND`] ends its outer loop with `iVar13 + 1; if (0xe < iVar13) return`
/// -- rows `0..=14` -- and the fixed vector it collects the built cells in refuses above `0x1e0`,
/// which is exactly `15 * 32`. Two spellings of the same bound in one function.
///
/// This is the ceiling a tab has once the item vector and the namer list are no longer in the way.
pub const FEX_GRID_MAX_ROWS: usize = 15;

/// Most columns, from the inner loop of the same function: `while (iVar12 < 0x20)`.
///
/// Recorded because it is the other half of the `0x1e0` the cell vector caps at, which is what makes
/// [`FEX_GRID_MAX_ROWS`] two measurements rather than one.
pub const FEX_GRID_MAX_COLS: usize = 32;

/// How many `FeGroupInGameGroupSelect` members `FeGroupInGameTopSelect` owns. Six, inline.
pub const FE_INGAME_TOP_SELECT_TABS: usize = 6;

/// Byte offset of each tab subobject inside the top select, in CONSTRUCTION order.
///
/// Read off `FeGroupInGameTopSelect`'s constructor, which builds them at `param_1 + 0x33`, `+0x60`,
/// `+0x8d`, `+0xba`, `+0xe7` and `+0x114` in qwords, and corroborated by
/// [`FE_INGAME_TOP_SELECT_TAB_TABLE`], which spells the same six as byte immediates.
pub const FE_INGAME_TOP_SELECT_TAB_OFFSETS: [usize; FE_INGAME_TOP_SELECT_TABS] =
    [0x198, 0x300, 0x468, 0x5d0, 0x738, 0x8a0];

/// Bytes per tab subobject, from the spacing above.
pub const FE_INGAME_TOP_SELECT_TAB_STRIDE: usize = 0x168;

/// Where a seventh tab would have to begin -- and what is already there.
///
/// `0x8a0 + 0x168`. The constructor's next act after the sixth tab is
/// `FUN_140027c80(.., param_1 + 0x141, ..)`, and `0x141 * 8` is this. A seventh tab would be
/// constructed on top of the top select's own caption accessor.
pub const FE_INGAME_TOP_SELECT_AFTER_TABS: usize = 0xa08;

/// `FUN_1400a66d0(topSelect)` -- display index to tab subobject. RVA `0x000a66d0`.
///
/// ```text
/// index = FEX_GRID_CURRENT_INDEX(topSelect);
/// local[0] = this + 0x198; local[1] = this + 0x300; local[2] = this + 0x468;
/// local[3] = this + 0x5d0; local[4] = this + 0x8a0; local[5] = this + 0x738;
/// return index < 6 ? local[index] : 0;
/// ```
///
/// A stack array of `this + literal`, with the bound as a literal too -- there is no table in data
/// to lengthen, which is spelling (2). What there is instead is a function with five callers and one
/// vtable reference (`0x1418a53f4`) and no other route to a tab, so a detour here is where a seventh
/// group is handed out.
///
/// Note it takes no index: it reads the cursor through [`FE_INGAME_TOP_SELECT_TAB_INDEX`], so a
/// detour is answering "the tab the player is on".
///
/// Not an Arxan redirect: prologue [`FE_INGAME_TOP_SELECT_TAB_TABLE_PROLOGUE`], with
/// `scripts/ds2-arxan-chain.py` terminating at hop 0.
pub const FE_INGAME_TOP_SELECT_TAB_TABLE: u32 = 0x000a_66d0;

/// The first six bytes of [`FE_INGAME_TOP_SELECT_TAB_TABLE`]: `rex push rbx` / `sub rsp,0x50`.
pub const FE_INGAME_TOP_SELECT_TAB_TABLE_PROLOGUE: [u8; 6] = [0x40, 0x53, 0x48, 0x83, 0xec, 0x50];

/// The order [`FE_INGAME_TOP_SELECT_TAB_TABLE`] puts the tabs in, as indices into
/// [`FE_INGAME_TOP_SELECT_TAB_OFFSETS`].
///
/// **Display order is not construction order, and the last two are swapped.** The System tab --
/// Game Options / Screen Options / Quit Game, built fifth by
/// [`FE_INGAME_TOP_SELECT_SYSTEM_TAB_ITEMS`] at `+0x738` -- is the LAST tab on screen, and the Key
/// Bindings / Graphics tab built sixth at `+0x8a0` is the one before it. Worth writing down because
/// this repo's own probe enumerates tabs in construction order, so its "tab 4" and the player's
/// "last tab" are the same tab under two numbers.
pub const FE_INGAME_TOP_SELECT_TAB_ORDER: [usize; FE_INGAME_TOP_SELECT_TABS] = [0, 1, 2, 3, 5, 4];

/// The TAB STRIP's own cell namer constructor. RVA `0x000a5c60`.
///
/// Same shape as a tab's row namer ([`FE_INGAME_MENU_QUIT_TAB_NAMER`]): a one-component base path
/// (`0x1eaba9`), then a six-slot id array pushed one at a time through [`FE_SCENE_NAMER_PUSH`] in a
/// `do { } while (i < 6)` loop. The difference is the one that matters -- **none of its six slots is
/// spare.**
pub const FE_INGAME_TOP_SELECT_NAMER: u32 = 0x000a_5c60;

/// The six cell ids [`FE_INGAME_TOP_SELECT_NAMER`] pushes, in the order it pushes them.
///
/// Six ids into a list that holds [`FE_SCENE_NAMER_LIST_CAPACITY`], which is spelling (3): pushing a
/// seventh panics in the game's own allocator, so a seventh tab's cell is served from a stand-in
/// through [`FE_SCENE_NAMER_CELL_LOOKUP`] instead of being pushed here.
pub const FE_INGAME_TOP_SELECT_NAMER_CELL_IDS: [u32; FE_INGAME_TOP_SELECT_TABS] = [
    0x001e_aba2,
    0x001e_aba3,
    0x001e_aba4,
    0x001e_aba6,
    0x001e_aba7,
    0x001e_aba5,
];

/// The first five bytes of [`FE_INGAME_TOP_SELECT_NAMER`]: `rex push rsi` / `push r14` / `sub`.
///
/// Not an Arxan redirect: `scripts/ds2-arxan-chain.py 0x1400a5c60` terminates at hop 0.
pub const FE_INGAME_TOP_SELECT_NAMER_PROLOGUE: [u8; 5] = [0x40, 0x56, 0x41, 0x56, 0x48];

/// Components in one of the tab strip's cell paths: the strip, then the cell. Two.
///
/// A tab's ROW paths are five ([`FE_QUIT_TAB_BASE_PATH`] plus the row), and the difference is why
/// the id cannot be read at a fixed offset: it is the last component, so it sits at
/// `(length - 1) * 4` and the length is the field at [`FE_SCENE_NAMER_ENTRY_LEN_OFFSET`].
/// `FUN_1400a5c60` writes `0x1eaba9` into a path whose length it sets to `1`, seals it, and appends
/// one cell id per entry -- so each finished entry is two long.
pub const FE_INGAME_TOP_SELECT_NAMER_ENTRY_LEN: u32 = 2;

/// The single base component of every tab-strip cell path, checked before an entry is cloned.
pub const FE_INGAME_TOP_SELECT_NAMER_BASE: u32 = 0x001e_aba9;

/// The tab strip's caption path builder, `fn(topSelect, out, index)`. RVA `0x000a6310`.
///
/// Holds a five-entry stack table (`0x1eab9b`, `0x1eab9c`, `0x1eab9d`, `0x1eab9f`, `0x1eab9e`)
/// behind `if (index < 5)`, and calls `FUN_140027980` -- make-empty -- for anything else.
///
/// **Five entries against six tabs**, so the shipped System tab already takes the empty arm and a
/// seventh tab needs nothing here. This is spelling (5) and it costs nothing.
pub const FE_INGAME_TOP_SELECT_TAB_CAPTION_PATH: u32 = 0x000a_6310;

/// `FeGroupInGameTopSelect`'s constructor, `fn(topSelect, arg2, arg3) -> topSelect`.
/// RVA `0x000a41b0`.
///
/// Where a seventh tab is built, because it is where the first six are. It constructs each one with
/// [`FE_INGAME_GROUP_SELECT_CTOR`] at the offsets in [`FE_INGAME_TOP_SELECT_TAB_OFFSETS`], handing
/// each a namer and a descriptor produced by that tab's own two builders:
///
/// ```text
/// descriptor = FUN_1400a5900(stack)                 // the System tab's items
/// namer      = FUN_1400a5b50(&out, proxy)           // the System tab's cells
/// FUN_1400a40e0(topSelect + 0xe7 * 8, proxy, &namer, descriptor)
/// ```
///
/// `proxy` is `topSelect + `[`FE_INGAME_TOP_SELECT_PROXY_OFFSET`], the same value
/// [`FE_INGAME_TOP_SELECT_TAB_CAPTION_PATH`] reads. A detour that runs the original and then repeats
/// those three lines into storage of its own gets a seventh group built by the game's own code.
///
/// Not an Arxan redirect: prologue [`FE_INGAME_TOP_SELECT_CTOR_PROLOGUE`], with
/// `scripts/ds2-arxan-chain.py` terminating at hop 0.
pub const FE_INGAME_TOP_SELECT_CTOR: u32 = 0x000a_41b0;

/// The first five bytes of [`FE_INGAME_TOP_SELECT_CTOR`]: `mov [rsp+0x10],rbx`.
pub const FE_INGAME_TOP_SELECT_CTOR_PROLOGUE: [u8; 5] = [0x48, 0x89, 0x5c, 0x24, 0x10];

/// `FeGroupInGameGroupSelect`'s constructor, `fn(group, proxy, *namer, descriptor) -> group`.
/// RVA `0x000a40e0`.
///
/// ```text
/// FUN_140020df0(group, &namer)                 // FeGroupBase, takes the namer reference
/// group[0x00] = group[0x0b] = FeGroupInGameGroupSelect::vftable
/// FUN_1400a3ef0(group + 0x1f, descriptor)      // group + 0xf8  <- the item vector
/// FUN_1400189f0(group + 0x26, descriptor + 0x38)
/// group[0x2c] = proxy                          // group + 0x160
/// Unref(namer)                                 // it consumes the caller's reference
/// ```
///
/// It writes nothing outside `group[0 ..= 0x2c]`, which is what makes
/// [`FE_INGAME_GROUP_SELECT_SIZE`] enough and a group constructible outside the scene.
pub const FE_INGAME_GROUP_SELECT_CTOR: u32 = 0x000a_40e0;

/// Bytes in one `FeGroupInGameGroupSelect`. `0x168`.
///
/// Two independent spellings: the spacing of [`FE_INGAME_TOP_SELECT_TAB_OFFSETS`], and the highest
/// field [`FE_INGAME_GROUP_SELECT_CTOR`] writes -- `group[0x2c]`, the last qword inside `0x168`.
pub const FE_INGAME_GROUP_SELECT_SIZE: usize = FE_INGAME_TOP_SELECT_TAB_STRIDE;

/// Byte offset of the layout proxy inside `FeGroupInGameTopSelect`. `0x150`.
///
/// `FUN_1400a41b0` opens by taking `param_1 + 0x2a` in qwords and passes it to every tab
/// constructor; [`FE_INGAME_TOP_SELECT_TAB_CAPTION_PATH`] spells the same address as
/// `param_1 + 0x150`. A seventh group needs this value and nothing else from the top select.
pub const FE_INGAME_TOP_SELECT_PROXY_OFFSET: usize = 0x150;

/// `FeGroupInGameTopSelect::v21` -- the strip's own init. RVA `0x000a6da0`.
///
/// Sets the tab strip's item count with a literal `FUN_140021b30(this, 6)`
/// ([`FEX_GRID_SET_ITEM_COUNT`]) and then runs [`FE_INGAME_MENU_TAB_INIT`] once per tab over an
/// unrolled six-pointer stack array. Spelling (4).
///
/// A detour that runs the original and then re-calls the count setter with seven is the same shape
/// as the per-tab raise `ds2-menu-row` already installs, one level up. The strip is a
/// `FrontendEx::FexGridControl`, so the bound past this literal is [`FEX_GRID_MAX_COLS`].
///
/// Not an Arxan redirect: prologue [`FE_INGAME_TOP_SELECT_STRIP_INIT_PROLOGUE`], with
/// `scripts/ds2-arxan-chain.py` terminating at hop 0.
pub const FE_INGAME_TOP_SELECT_STRIP_INIT: u32 = 0x000a_6da0;

/// The first five bytes of [`FE_INGAME_TOP_SELECT_STRIP_INIT`]: `mov [rsp+0x10],rbx`.
pub const FE_INGAME_TOP_SELECT_STRIP_INIT_PROLOGUE: [u8; 5] = [0x48, 0x89, 0x5c, 0x24, 0x10];

/// Which tab [`FE_INGAME_TOP_SELECT_TAB_TABLE`] is being asked for: [`FEX_GRID_CURRENT_INDEX`],
/// called with the top select itself.
///
/// The table takes no index argument -- it reads the cursor. So a detour there is answering "the tab
/// the player is on", and the number it has to recognise is [`FE_INGAME_TOP_SELECT_TABS`].
pub const FE_INGAME_TOP_SELECT_TAB_INDEX: u32 = FEX_GRID_CURRENT_INDEX;

// ---------------------------------------------------------------------------------------------
// THE TWO ACCESSORS, WHICH IS WHERE STORAGE OF OUR OWN GOES IN
//
// Neither the item vector nor the namer list can be grown or repointed. Both are read through ONE
// function each, and a detour there can serve an entry from anywhere -- which turns two hard fixed
// bounds (5 and 6) into [`FEX_GRID_MAX_ROWS`].
// ---------------------------------------------------------------------------------------------

/// `FUN_1400a6750(tab) -> *entry` -- the item entry under the cursor. RVA `0x000a6750`.
///
/// ```text
/// index = FEX_GRID_CURRENT_INDEX(tab);
/// if (index < *(u64*)(tab + 0x128))
///     return tab + 0xf8 + (-(int)(tab + 0xf8) & 3) + index * 8;
/// return &static{ action = 0xffffffff, gate = 0 };
/// ```
///
/// **Its two callers are the only readers of an item's ACTION**: the confirm handler
/// (`0x1400a6b10`) and `FUN_1400a4cc0`, which is the same read again for the enable/sound decision.
/// The availability pass [`FE_INGAME_MENU_AVAILABILITY_PASS`] does NOT come through here -- it
/// inlines the same bounds check and only ever reads the GATE.
///
/// So a detour here can answer for an index the vector does not hold, and the failure mode if it
/// declines is the game's own `0xffffffff` -- an action no case matches, i.e. an inert row.
///
/// Not an Arxan redirect: prologue [`FE_INGAME_MENU_TAB_ITEM_LOOKUP_PROLOGUE`], with
/// `scripts/ds2-arxan-chain.py` terminating at hop 0.
pub const FE_INGAME_MENU_TAB_ITEM_LOOKUP: u32 = 0x000a_6750;

/// The first six bytes of [`FE_INGAME_MENU_TAB_ITEM_LOOKUP`]: `rex push rbx` / `sub rsp,0x20`.
pub const FE_INGAME_MENU_TAB_ITEM_LOOKUP_PROLOGUE: [u8; 6] = [0x40, 0x53, 0x48, 0x83, 0xec, 0x20];

/// The availability pass, `FeGroupInGameGroupSelect::FUN_1400a77c0(tab)`. RVA `0x000a77c0`.
///
/// Walks `0..itemCount` -- the VIRTUAL count at [`FEX_GRID_ITEM_COUNT_OFFSET`], read through a
/// vtable slot -- and for each index reads the entry INLINE, with its own copy of
/// [`FE_INGAME_MENU_TAB_ITEM_LOOKUP`]'s bounds check against the vector's count and the same static
/// `(0xffffffff, 0)` fallback. It then does nothing at all unless that entry's GATE is non-zero.
///
/// **That is what makes raising the virtual item count safe.** Past the vector's own count this pass
/// reads the static, sees gate `0`, and skips -- it never touches the storage beyond the last real
/// entry, so there is no uninitialised gate for it to hand the gate predicate. Its only caller is
/// [`FE_INGAME_MENU_TAB_INIT`]. Recorded, not hooked.
pub const FE_INGAME_MENU_AVAILABILITY_PASS: u32 = 0x000a_77c0;

/// The first nine bytes of [`FE_INGAME_MENU_TAB_INIT`]: `rex push rbx` / `sub rsp,0xb0`.
pub const FE_INGAME_MENU_TAB_INIT_PROLOGUE: [u8; 9] =
    [0x40, 0x53, 0x48, 0x81, 0xec, 0xb0, 0x00, 0x00, 0x00];

/// `FUN_140022140(grid) -> int` -- the index under the cursor. RVA `0x00022140`.
///
/// `if (grid[0x1e] != 0 || grid[0xd0] < 0) return grid[0xcc]; else return grid[0xd0];`. Called
/// rather than reimplemented wherever a detour needs the same index its caller is about to use.
pub const FEX_GRID_CURRENT_INDEX: u32 = 0x0002_2140;

/// `FUN_140021b30(grid, count)` -- write [`FEX_GRID_ITEM_COUNT_OFFSET`] and drive the scrollbar.
/// RVA `0x00021b30`.
///
/// The only writer of that field, and on the in-game menu tabs the scroll object at
/// [`FEX_GRID_SCROLL_OFFSET`] is null, so on this path it writes the count and returns at the null
/// check. Called -- not hooked -- so that a tab's cursor bound can be raised past what its item
/// vector holds, through the game's own setter rather than by storing into a field.
///
/// Twenty-three callers across the frontend; the two on this path are [`FE_INGAME_MENU_TAB_INIT`]
/// (per tab, with the vector's count) and `0x1400a6da0` (the tab strip, with a literal `6`).
/// Nothing else writes the field on these objects, which is why a value written after the init
/// survives.
pub const FEX_GRID_SET_ITEM_COUNT: u32 = 0x0002_1b30;

/// `FrontendEx::IngameTopLayoutAdapter`'s ordinary-cell lookup, `fn(namer, out, cell) -> out`.
/// RVA `0x000a4b20`, vtable slot `+0x10` at `0x1410b69b8`.
///
/// **It reads exactly three fields of the namer and nothing else**, which is read off the
/// disassembly rather than the decompiler:
///
/// ```text
/// cmp DWORD PTR [r8],0x0          ; cell.col != 0 -> empty
/// mov edx,DWORD PTR [r8+0x4]      ; cell.row
/// cmp rdx,QWORD PTR [rcx+0x140]   ; >= count -> empty
/// lea r9,[rcx+0x18]               ; the entry list
/// mov rcx,QWORD PTR [rcx+0x10]    ; the scene proxy
/// ...  r8 = list + (-list & 7) + row * 0x30
/// call 0x140026790                ; (proxy, out, entry)
/// ```
///
/// Three fields is what makes a stand-in possible: a buffer carrying a scene proxy at
/// [`FE_SCENE_NAMER_PROXY_OFFSET`], one entry at [`FE_SCENE_NAMER_LIST_OFFSET`] and a count of `1`
/// at `0x140` is indistinguishable from a namer here. Passing that, plus a cell of `(0, 0)`, makes
/// the game's own code build the accessor for an entry of ours -- with nothing reimplemented, and
/// without the entry ever sitting at an index whose stride would reach the count field.
///
/// The sibling slot `+0x18` (`FUN_1400a4c80`) is `make-empty; return` for every cell on every tab,
/// so there is no second element to supply.
///
/// Not an Arxan redirect: prologue [`FE_SCENE_NAMER_CELL_LOOKUP_PROLOGUE`].
pub const FE_SCENE_NAMER_CELL_LOOKUP: u32 = 0x000a_4b20;

/// The first nine bytes of [`FE_SCENE_NAMER_CELL_LOOKUP`]: `rex push rbx` / `sub rsp,0x260`.
pub const FE_SCENE_NAMER_CELL_LOOKUP_PROLOGUE: [u8; 9] =
    [0x40, 0x53, 0x48, 0x81, 0xec, 0x60, 0x02, 0x00, 0x00];

/// The TAB STRIP's cell lookup -- the same function for the other axis. RVA `0x000a4a70`.
///
/// `IngameTopLayoutAdapter` is two classes, not one, and detouring only the first is why a seventh
/// tab could be selected and never drawn. Their vtables sit next to each other and slot 2 of each
/// is the cell lookup:
///
/// | adapter | vtable | slot 2 | serves |
/// |---|---|---|---|
/// | `VLayoutAdapter` | `0x1410b69a8` | [`FE_SCENE_NAMER_CELL_LOOKUP`] | a tab's rows |
/// | `HLayoutAdapter` | `0x1410b6a08` | this | the strip's tab cells |
///
/// The bodies are mirror images and the difference is which field of the cell is the index:
///
/// ```asm
/// 0x1400a4b20:  cmp DWORD PTR [r8],0x0        ; the tab's:   col must be zero
///               mov edx,DWORD PTR [r8+0x4]    ;              row is the index
/// 0x1400a4a70:  cmp DWORD PTR [r8+0x4],0x0    ; the strip's: ROW must be zero
///               mov edx,DWORD PTR [r8]        ;              COL is the index
/// ```
///
/// Everything after that is identical -- `[rcx+0x140]` is the count, `rcx+0x18` the entry list,
/// `[rcx+0x10]` the scene proxy, stride `0x30` -- so a stand-in built for one is a stand-in for the
/// other, and the cell it is asked with is `(0, 0)` either way.
///
/// Not an Arxan redirect: `scripts/ds2-arxan-chain.py 0x1400a4a70` terminates at hop 0.
pub const FE_SCENE_NAMER_STRIP_CELL_LOOKUP: u32 = 0x000a_4a70;

/// The first seven bytes of [`FE_SCENE_NAMER_STRIP_CELL_LOOKUP`]: `rex push rbx` / `sub rsp,0x260`.
pub const FE_SCENE_NAMER_STRIP_CELL_LOOKUP_PROLOGUE: [u8; 7] =
    [0x40, 0x53, 0x48, 0x81, 0xec, 0x60, 0x02];

/// The tab strip's second element per cell, slot 3 of `HLayoutAdapter`. RVA `0x000a4bd0`.
///
/// A grid asks its adapter for two elements per cell, and on a tab the second one
/// (`FUN_1400a4c80`, slot 3 of `VLayoutAdapter`) is `make-empty; return` -- which is why the row
/// work never needed it, and why this table used to say there was no second element to supply.
///
/// On the strip there is. This function is the same body as
/// [`FE_SCENE_NAMER_STRIP_CELL_LOOKUP`] instruction for instruction -- same row-must-be-zero test,
/// same `[rcx+0x140]` count, same `rcx+0x18` list, same `FUN_140026790` -- so it resolves the same
/// entry to a second accessor. Serving the first and not the second gave a seventh tab whose
/// selection highlight drew and whose glyph did not.
///
/// Not an Arxan redirect: `scripts/ds2-arxan-chain.py 0x1400a4bd0` terminates at hop 0, and its
/// prologue is [`FE_SCENE_NAMER_STRIP_CELL_LOOKUP_PROLOGUE`] -- the same seven bytes, because the
/// two functions are the same function twice.
pub const FE_SCENE_NAMER_STRIP_CELL_SECOND: u32 = 0x000a_4bd0;

/// Byte offset, inside a cell namer, of the scene proxy its lookup resolves paths against.
/// `mov rcx,QWORD PTR [rcx+0x10]`.
pub const FE_SCENE_NAMER_PROXY_OFFSET: usize = 0x10;

/// `FUN_140027980(out) -> out` -- construct the EMPTY element accessor. RVA `0x00027980`.
///
/// Four calls: a base init, the `FrontendEx::SceneObjProxy` vtable, a default at `+0x58`, and an
/// empty path copied into `+0x60`. It is what [`FE_SCENE_NAMER_CELL_LOOKUP`] itself returns for a
/// cell that is not there, and its slot 0 resolves to null -- which is how the grid's layout bind
/// learns a row has ended.
///
/// Recorded because it is the one correct answer a detour on that lookup can give when it cannot
/// reach the original: handing back an untouched output buffer would leave the caller to call a
/// vtable slot on uninitialised stack.
pub const FE_SCENE_ACCESSOR_MAKE_EMPTY: u32 = 0x0002_7980;

/// Byte offset of a cell namer's entry count, from the namer rather than from its list.
/// [`FE_SCENE_NAMER_LIST_OFFSET`]` + `[`FE_SCENE_NAMER_COUNT_OFFSET`], which the lookup spells as
/// the immediate `0x140`.
pub const FE_SCENE_NAMER_COUNT_FROM_NAMER: usize =
    FE_SCENE_NAMER_LIST_OFFSET + FE_SCENE_NAMER_COUNT_OFFSET;

/// Bytes a stand-in namer has to cover: past [`FE_SCENE_NAMER_COUNT_FROM_NAMER`].
pub const FE_SCENE_NAMER_SHADOW_SIZE: usize = FE_SCENE_NAMER_COUNT_FROM_NAMER + 8;

/// `FUN_1400189f0(dst, src) -> dst` -- the copy one namer LIST ENTRY is made with. RVA `0x000189f0`.
///
/// **This is what says an entry is a `DLKR::DLFixedVector<u32, 8>` and not an opaque struct.** The
/// function refuses a source count above `8`, copies that many `u32`s from `src` to `dst` at
/// stride 4, and writes the count at `+0x28` -- which is exactly
/// [`FE_SCENE_NAMER_ENTRY_LEN_OFFSET`], the field this table used to call "the path length". It is
/// the length, and it is a vector's count.
///
/// So the "uninitialised slack" between the last id and `+0x28` is the unused tail of a
/// fixed-capacity array, and a copy through this function reproduces an entry exactly as the game's
/// own push does -- [`FE_SCENE_NAMER_PUSH`] calls this to do the copying.
pub const FE_SCENE_NAMER_ENTRY_COPY: u32 = 0x0001_89f0;

/// Ids one namer list entry can hold, from `if (8 < count) panic` in
/// [`FE_SCENE_NAMER_ENTRY_COPY`]. The quit tab's paths use five of the eight.
pub const FE_SCENE_NAMER_ENTRY_CAPACITY: usize = 8;

// ---------------------------------------------------------------------------------------------
// QUITTING TO DESKTOP
// ---------------------------------------------------------------------------------------------

/// `FeSubStateTitleShutdown::v1` (enter) -- the game's own quit-to-desktop, in full. RVA
/// `0x000fde20`.
///
/// Three instructions, and there is no fourth:
///
/// ```text
/// mov rax, QWORD PTR [rip+0x15773d1]      ; [FE_SYSTEM_SINGLETON]
/// mov BYTE PTR [rax+0x13a], 1             ; FE_SYSTEM_SHUTDOWN_REQUEST_OFFSET
/// ret
/// ```
///
/// Its `update` (`0x1400ff2e0`) is an empty `ret`, so the substate does not run the shutdown -- it
/// only asks for one. Recorded because it is the whole implementation of "quit to desktop" and it
/// is a byte, not a call.
pub const FE_SUBSTATE_TITLE_SHUTDOWN_ENTER: u32 = 0x000f_de20;

/// The `FeSystem`-ish singleton pointer the title flow reads everything off. RVA `0x016751f8`.
///
/// Already relied on elsewhere in this repo without being named: `FeSubStateTitleLogo`'s skip
/// tests a state word reached through it, and `FeSubStateWarningNoCopy`'s shipped early-out calls
/// one of its virtuals. It holds a POINTER; dereference it before adding an offset.
pub const FE_SYSTEM_SINGLETON: u32 = 0x0167_51f8;

/// Byte offset of the shutdown request inside [`FE_SYSTEM_SINGLETON`]'s target.
///
/// **Setting it to 1 quits the game, and that is the entire mechanism.** Four sites in the image
/// write it (`0x1400f4a9a`, `0x1400fbc00`, `0x1400fde23`, `0x1401c2303`) and exactly two read it
/// (`0x1401bf97e`, `0x1401c0196`) -- both inside `GameManagerImp`'s per-frame master update, the
/// function that also drives `mapManUpdate`, `damageManUpdate`, `bulletManUpdate`, `demoManager`
/// and `saveRequest`.
///
/// Being polled by the main loop is what makes this usable from anywhere: a write takes effect on
/// the next frame, through the game's own shutdown, with no confirmation dialog and no new code
/// path. It is the same byte the title screen's own exit row writes.
///
/// It is NOT a save. The game's quit-to-title flow offers to save because that flow asks; this
/// does not, which is what "without a confirmation" costs.
pub const FE_SYSTEM_SHUTDOWN_REQUEST_OFFSET: usize = 0x13a;

/// The first action id this repo hands out. Slot `n` gets `BASE + n`. Deliberately outside the
/// game's own space.
///
/// The shipped dispatch has cases `0..=9`, `0xb`, `0xc`, `0xd`. Anything else falls to `default`,
/// which plays the ordinary confirm sound and does nothing. That is the correct failure mode for
/// an id whose behaviour lives in a detour: if the detour is ever absent, the row is INERT rather
/// than quietly doing whatever the game does for some id we borrowed.
///
/// The range is `0x1000..` and the ceiling on rows is five per tab, so the ids stay nowhere near
/// anything the game uses no matter how many tabs are eventually measured.
pub const FE_INGAME_MENU_ACTION_BASE: u32 = 0x1000;

/// `FrontendEx::FexGridControl` linear-index -> `(col, row)`. RVA `0x000222c0`.
///
/// `if (grid->rowExtent == 1) { col = index; row = 0; } else { row = index / cols; col = index % cols; }`.
/// Measured at runtime, the in-game menu tabs are one COLUMN by N ROWS -- every tab this repo has
/// looked at reports `col-extent = 1` and `row-extent = itemCount` -- so they take the second
/// branch and an item's cell is `(0, index)`.
pub const FEX_GRID_INDEX_TO_CELL: u32 = 0x0002_22c0;

/// `FrontendEx::FexGridControl` `(col, row)` -> element accessor. RVA `0x00022160`.
///
/// Asks the namer at `[grid + 0xf0]` through one of five vtable slots, picked by whether the cell
/// is on an edge: `+0x30` when `row == -1`, `+0x38` when `row == rowExtent`, `+0x40` when
/// `col == -1`, `+0x48` when `col == colExtent`, and `+0x10` for an ordinary interior cell.
///
/// **The `col == colExtent` branch is the one an appended item takes**, which is why its element
/// resolves to nothing: it is the "one past the end" namer, not the ordinary-cell namer.
pub const FEX_GRID_CELL_TO_ELEMENT: u32 = 0x0002_2160;

// ---------------------------------------------------------------------------------------------
// THE CELL NAMER, AND WHY A FOURTH ROW MIGHT BE FOUR BYTES
// ---------------------------------------------------------------------------------------------

/// The quit tab's cell-namer constructor. RVA `0x000a5b50`.
///
/// Read from the disassembly, because the decompiler drops half of it. It builds a FOUR-component
/// base path and then a SIX-slot array of cell ids of which three are zero:
///
/// ```asm
/// mov  r9d, 0x1eace8
/// lea  r8d, [r9-0x19]                  ; 0x1eaccf
/// mov  edx, 0x1eaba9
/// mov  DWORD PTR [rsp+0x20], 0x1eace6  ; -> path [0x1eaba9, 0x1eaccf, 0x1eace8, 0x1eace6]
/// mov  DWORD PTR [rsp+0xc0], 0x1eacc9
/// mov  DWORD PTR [rsp+0xc4], 0x1eacca
/// mov  QWORD PTR [rsp+0xc8], 0x1eace9  ; and the pad dword behind it
/// mov  QWORD PTR [rsp+0xd0], rbx       ; zero, zero
/// ...  cmp rcx, 6 ; jb                 ; the loop already runs SIX times
/// ```
///
/// Each non-zero id becomes one entry pushed into the namer's list at `+0x18`, and that list is
/// what `FEX_GRID_CELL_TO_ELEMENT` indexes. **So a fourth cell is a four-byte constant here, not a
/// layout edit -- provided an element exists for the id to resolve to.** Whether one does is the
/// question `ds2-menu-row`'s enumerator answers.
///
/// Both cells the tab gained after the fact are out of sequence -- `c9, ca, ... e9` here and
/// `7b, 7c, ... d5` on tab 3 -- which is what appending a row to a shipped tab looks like.
pub const FE_INGAME_MENU_QUIT_TAB_NAMER: u32 = 0x000a_5b50;

/// The quit tab's cell base path, as the four ids the namer builds it from.
pub const FE_QUIT_TAB_BASE_PATH: [u32; 4] = [0x001e_aba9, 0x001e_accf, 0x001e_ace8, 0x001e_ace6];

/// The three cell ids the quit tab ships with, appended to [`FE_QUIT_TAB_BASE_PATH`].
pub const FE_QUIT_TAB_CELL_IDS: [u32; 3] = [0x001e_acc9, 0x001e_acca, 0x001e_ace9];

/// `FrontendEx` scene path builder, four ids. RVA `0x000756a0`.
///
/// `fn(out, id0, id1, id2, id3) -> out`, with the fourth id on the stack at `[rsp+0x20]`.
pub const FE_SCENE_PATH_BUILD4: u32 = 0x0007_56a0;

/// Converts a built path into the form the append below takes. RVA `0x0001f8a0`. `fn(path, out)`.
pub const FE_SCENE_PATH_SEAL: u32 = 0x0001_f8a0;

/// Appends one id to a sealed path, producing an element accessor. RVA `0x0001ed80`.
///
/// `fn(path, out, id) -> out`. The accessor's own vtable slot 0 resolves it: non-zero is the live
/// element, zero means the scene has nothing at that path. That resolve is exactly what the grid's
/// layout bind uses to decide whether a cell exists, so asking it is asking the same question the
/// bind asks.
pub const FE_SCENE_PATH_APPEND: u32 = 0x0001_ed80;

/// The quit tab's cell ids that the LAYOUT authors and the namer never lists.
///
/// `FeSceneInGameMenu`'s element cache (`0x140099f90`) resolves five cell-shaped children under
/// `[0x1eaba9, 0x1eaccf, 0x1eace8, 0x1eace7]`, each followed by its own label element:
///
/// ```text
/// 0x1eacc9 + label 0x1eac46      Game Options
/// 0x1eacca + label 0x1eac47      Screen Settings
/// 0x1eaccd + label 0x1eac4a      <- authored, never listed
/// 0x1eacce + label 0x1eac4b      <- authored, never listed
/// 0x1eace9 + label 0x1eac4c      Quit Game
/// ```
///
/// **THAT PARAGRAPH USED TO SAY THE TAB WAS AUTHORED FOR FIVE ROWS. It was not measured and it is
/// probably false.** The cache ASKS for five ids; `FUN_140afda00` reaches `FUN_140b507d0`, which is
/// a plain lookup returning 0 when nothing is there, and the cache stores the answer without
/// checking it. So five requests is evidence of five requests.
///
/// What was measured afterwards, with controls: a cloned namer entry becomes a real cell, and
/// clones naming `0x1eaccd` or `0x1eacce` do not resolve while an unmodified clone does. The two
/// spares are therefore absent from the container the namer can reach, and most likely absent
/// full stop -- leftovers from a five-row design that did not ship.
///
/// The namer builds its cell paths under `0x1eace6` while the cache resolves under `0x1eace7` --
/// two sibling containers below `0x1eace8`, with the same cell ids in each. The pair is presumably
/// an interaction layer and a drawing layer.
pub const FE_QUIT_TAB_SPARE_CELL_IDS: [u32; 2] = [0x001e_accd, 0x001e_acce];

/// Pushes one built accessor onto a cell namer's list at `namer + 0x18`. RVA `0x000a7b30`.
///
/// `fn(&namer[0x18], accessor)`. This is the call [`FE_INGAME_MENU_QUIT_TAB_NAMER`] makes once per
/// non-zero id in its six-slot array, so appending a fourth entry is the same operation the game
/// performs three times on the way in.
pub const FE_SCENE_NAMER_PUSH: u32 = 0x000a_7b30;

/// Byte offset of the cell list inside a cell namer.
pub const FE_SCENE_NAMER_LIST_OFFSET: usize = 0x18;

/// Most entries a cell namer's list at [`FE_SCENE_NAMER_LIST_OFFSET`] can hold.
///
/// Measured the expensive way: a run that pushed six extra entries onto a list the game had already
/// put three in wrote through a null on the seventh and killed the game at `0x141bee1c4`
/// (`access_kind=1`, `rcx=rdx=rax=0`). Three plus three is fine; three plus four is not.
///
/// Six is also the bound of the id loop in [`FE_INGAME_MENU_QUIT_TAB_NAMER`], so the array and the
/// list it fills are the same size -- which is the sort of agreement worth writing down, because
/// it says the spare slots in that array are genuinely usable rather than accidental padding.
///
/// **And it is spelled in code as well as measured**, which the crash did not establish:
/// [`FE_SCENE_NAMER_PUSH`] opens `count + 1; if (6 < that) panic("out of memory.")` against
/// `DLFixedVector.inl:0x24c`. The run that died on the seventh push was reading a bound the
/// disassembly already had.
///
/// Like [`FE_INGAME_MENU_ITEM_VECTOR_CAPACITY`], it stopped being the row ceiling once
/// [`FE_SCENE_NAMER_CELL_LOOKUP`] -- the one place a cell's element is read -- could be answered
/// from storage of our own.
pub const FE_SCENE_NAMER_LIST_CAPACITY: usize = 6;

/// Byte offset of the count inside a cell namer's list, relative to the list itself.
///
/// `FUN_1400a7b30` reads `*(u64*)(list + 0x128)`, refuses above
/// [`FE_SCENE_NAMER_LIST_CAPACITY`], and writes the incremented value back before copying the new
/// element in. Relative to the namer that lands at `+0x140`, which is exactly the field
/// `VLayoutAdapter`'s cell lookup (`0x1400a4b20`) compares the requested row against.
pub const FE_SCENE_NAMER_COUNT_OFFSET: usize = 0x128;

/// Bytes per entry in a cell namer's list. `FUN_1400a7b30` addresses element `n` at
/// `list + (-list & 7) + n * 0x30`, and `0x1400a4b20` reads it back at the same stride.
pub const FE_SCENE_NAMER_ENTRY_STRIDE: usize = 0x30;

/// A cell namer entry, as dumped from two live entries of the quit tab's list.
///
/// ```text
/// e0: a9ab1e00 cfac1e00 e8ac1e00 e6ac1e00 c9ac1e00 <slack> 05000000
/// e1: a9ab1e00 cfac1e00 e8ac1e00 e6ac1e00 caac1e00 <slack> 05000000
///      +0x00    +0x04    +0x08    +0x0c    +0x10            +0x28
/// ```
///
/// Five `u32` ids, then uninitialised slack, then the path LENGTH. The slack genuinely differs
/// between two entries the game built one after the other -- it is stack residue -- which is why a
/// clone must copy an entry rather than be assembled field by field, and why a byte-diff has to
/// ignore everything outside the fields named here.
/// Byte offset of the tab-subtree component, which is the second of the five ids and the one a tab
/// of our own rewrites: [`FE_QUIT_TAB_BASE_PATH`]`[1]`, [`FLO_TAB_STRIP_PANEL_ID`] as the game
/// builds it and [`FLO_ADDED_TAB_SUBTREE_ID`] once the seventh tab is in.
///
/// It is also the offset inside a scene PATH object, which carries the same five ids at the same
/// places -- the entry is a path plus its length.
pub const FE_SCENE_NAMER_ENTRY_SUBTREE_OFFSET: usize = 0x04;
pub const FE_SCENE_NAMER_ENTRY_CONTAINER_OFFSET: usize = 0x0c;
pub const FE_SCENE_NAMER_ENTRY_ID_OFFSET: usize = 0x10;
pub const FE_SCENE_NAMER_ENTRY_LEN_OFFSET: usize = 0x28;
/// The path length every quit-tab entry carries: root, region, `ace8`, container, cell.
pub const FE_SCENE_NAMER_ENTRY_LEN: u32 = 5;
/// The sibling container the scene's element cache resolves the five-cell set under, where the
/// namer's own entries use [`FE_QUIT_TAB_BASE_PATH`]`[3]`.
pub const FE_QUIT_TAB_CACHE_CONTAINER: u32 = 0x001e_ace7;

// ---------------------------------------------------------------------------------------------
// The frontend layout document (`.flo`), and the container the quit tab's rows hang off.
//
// This is the section that turned the "invisible fourth row" from a file-repacking project into a
// four-pointer edit. The pause menu's rows are records in `menu/02.febnd.dcx`'s
// `l02_01_In-Game.flo`, and the game loads that file IN PLACE -- the header IS the document
// object, and the `u64` file offsets inside it are absolute pointers once the fixup has run. So
// the table that says how many rows a container has can be replaced with a copy that says one
// more, without touching the archive, the DCX, the BND4 or a single byte on disk.
//
// Reproduce every number below with:
//
//     python3 scripts/ds2-ebl.py extract /menu/02.febnd.dcx --out /tmp/menu02
//     python3 scripts/ds2-flo.py tree /tmp/menu02/l02_01_In-Game.flo --def 0x263
//
// THE FILE IS `/menu/02.febnd.dcx`, NOT `/menu/42.febnd.dcx`. 42 is the OPTIONS screen
// (`l42_01_OptionSetting.flo`) and shares none of these ids; an earlier note in this repo pointed
// at it and cost a wrong search.
// ---------------------------------------------------------------------------------------------

/// `FeLayoutDocument::findDefinition(doc, index)`. RVA `0x00b54740`.
///
/// `fn(&doc, u32 index) -> *definition`. A linear scan of `[[doc]+0x18]` over `[[doc]+0x4c]`
/// entries at stride [`FLO_DEFINITION_STRIDE`], keyed by the `u16` at the definition's `+0x00`;
/// returns null on a miss. Prologue `48 8b 01 44 8b ca 48 85 c0` -- its own, not one of the 286
/// Arxan redirects.
///
/// **Every consumer of a definition goes through here**, which is what makes one detour enough:
/// the builder reads the child count and the child array out of whatever this hands back, and the
/// built container keeps the same pointer at its `+0x48` for the capacity check below.
pub const FLO_FIND_DEFINITION: u32 = 0x00b5_4740;

/// Bytes per definition. `FUN_140b54740`: `add rcx, 0x48`.
pub const FLO_DEFINITION_STRIDE: usize = 0x48;

/// `u16` child count inside a definition. `FUN_140b50f20` walks that many child records --
/// **and `FUN_140b6bd80` uses the same field as the display list's CAPACITY**, refusing to attach
/// a child once `parent+0x66` reaches it. One field, both meanings, so raising it raises both.
pub const FLO_DEFINITION_CHILD_COUNT_OFFSET: usize = 0x02;

/// Pointer to a definition's child record array. A file offset on disk, an absolute pointer once
/// the document is loaded.
pub const FLO_DEFINITION_CHILDREN_OFFSET: usize = 0x08;

/// Bytes per child record. `FUN_140b50f20`: `lVar8 = lVar8 + 0x28`.
pub const FLO_RECORD_STRIDE: usize = 0x28;

/// `u16` definition index a record instantiates -- the argument [`FLO_FIND_DEFINITION`] takes.
pub const FLO_RECORD_DEFINITION_OFFSET: usize = 0x00;

/// Pointer to a record's transform block. Read by `FUN_140b50bc0` as `*(float**)(rec+0x08)`.
pub const FLO_RECORD_TRANSFORM_OFFSET: usize = 0x08;

/// `u16` depth. Passed on only for the leaf kinds; a nested record's copy is inert, and the draw
/// order of siblings follows the order they are attached in.
pub const FLO_RECORD_DEPTH_OFFSET: usize = 0x10;

/// `u16` kind flags, the value `FUN_140b50bc0` switches on: `1` shape, `2` mask, `4` a nested
/// definition, **`8` text**. A FLAG WORD rather than an enum -- the builder masks it with `0xd` and
/// records carrying `0x1004` exist, so the bits above the low nibble mean something unread. Every
/// quit-tab row is plain `4`, and this crate only ever copies the field.
///
/// **THIS SAID "`2` TEXT, `8` TEXTURE" UNTIL 2026-08-28 AND IT WAS WRONG BOTH WAYS.** The check
/// that settles it needs no disassembly: `scripts/ds2-flo.py tree l02_01_In-Game.flo --def 0x22c`
/// walks the caption mark down to the leaf `caption.rs` writes row labels into -- an element whose
/// kind is not in question, because this repo already puts text in it -- and prints `kind=0x8`.
///
/// The derivation, for the version that does need disassembly: each bit selects a table and a
/// builder, the builder calls a constructor, the constructor writes a vtable, and MSVC RTTI names
/// it. `0x1` -> `0x140b6ef80` -> `FeComponentTextureShape` (or `FeComponentTextureMask` when the
/// builder is inside a mask walk, branched at `0x140b50d29`); `0x2` -> `0x140b6d080` ->
/// `FeComponentMaskShape`; `0x8` -> `0x140b6d390` -> `FeComponentTextField`.
///
/// The mistake survived because it never contradicted anything: in `l02_01_In-Game.flo` the mask
/// table's count at `doc+0x4a` is zero, so every `kind & 2` record misses that lookup and falls
/// through to the shape table and draws -- which is exactly what a reader expecting "shape" sees.
pub const FLO_RECORD_KIND_OFFSET: usize = 0x12;

/// `u16` last frame and `u16` first frame. `0xffff` as the last frame means "never ends", which is
/// what every permanent element carries.
pub const FLO_RECORD_LAST_FRAME_OFFSET: usize = 0x14;
pub const FLO_RECORD_FIRST_FRAME_OFFSET: usize = 0x16;

/// `u32` ELEMENT ID -- the field a scene path resolves against.
///
/// `FeComponentObject::findByIdPath` (`0x140b6a130`) is `mov rax,[rcx+0x48]; cmp [rax+0x1c],r9d`:
/// the component's `+0x48` is its record, and `+0x1c` of that record is the id being matched. So
/// the ids in [`FE_QUIT_TAB_CELL_IDS`] are literally these bytes, and a fourth row is a fourth
/// record carrying a fourth id.
pub const FLO_RECORD_ID_OFFSET: usize = 0x1c;

/// Bytes per transform block, from the spacing of the blocks the quit tab's records point at.
pub const FLO_TRANSFORM_SIZE: usize = 0x30;

/// `f32` x and `f32` y inside a transform block.
///
/// Not guessed from position in the struct: `FUN_140b50f20`'s "is this child trivial enough to
/// inline" test reads `pfVar1[0]` and `pfVar1[1]` and requires them to be `0.0`, then `pfVar1[2]`
/// and `pfVar1[3]` and requires them to be `1.0`. Translate-zero and scale-one is an identity
/// test, which fixes all four fields at once.
pub const FLO_TRANSFORM_X_OFFSET: usize = 0x00;
pub const FLO_TRANSFORM_Y_OFFSET: usize = 0x04;

/// The definition index of the container the quit tab's rows are children of.
///
/// It is the definition instantiated by the record whose id is [`FE_QUIT_TAB_BASE_PATH`]`[3]`
/// (`0x1eace6`), which is the last component of every cell path the namer builds. Its seven
/// children are [`FLO_QUIT_TAB_CHILD_IDS`].
pub const FLO_QUIT_TAB_CONTAINER_DEFINITION: u32 = 0x0263;

/// The seven children of [`FLO_QUIT_TAB_CONTAINER_DEFINITION`], in file order, by element id.
///
/// ```text
/// [0] 0x1eac81  def 0x0221  xy (   0, -103   )  the tab's own header
/// [1] 0x1eace9  def 0x0258  xy (-0.10, 103.90)  row 2, Quit Game    <- has a greyed-out variant
/// [2] 0x1eacca  def 0x025d  xy (-3.15,  55.90)  row 1
/// [3] 0x1eacc9  def 0x0262  xy ( 3.95,  10.60)  row 0
/// [4] 0x1eac4c  def 0x022c  xy (60.20, 114.35)  row 2's mark
/// [5] 0x1eac47  def 0x022c  xy (60.20,  65.95)  row 1's mark
/// [6] 0x1eac46  def 0x022c  xy (60.20,  17.55)  row 0's mark
/// ```
///
/// **Seven, and all seven slots are used** -- none of them is flattened away, because
/// `FUN_140b50bc0` only inlines a child whose id is zero and whose transform is the identity, and
/// every one of these has a non-zero id. That is why a fourth row cannot be squeezed into the
/// shipped display list and the child count has to rise.
///
/// This array is the content check `ds2-menu-row` runs before it substitutes anything. A
/// definition index is a number, and index `0x263` on a document this table was not read from is
/// some other container entirely.
pub const FLO_QUIT_TAB_CHILD_IDS: [u32; 7] = [
    0x001e_ac81,
    0x001e_ace9,
    0x001e_acca,
    0x001e_acc9,
    0x001e_ac4c,
    0x001e_ac47,
    0x001e_ac46,
];

/// Index into [`FLO_QUIT_TAB_CHILD_IDS`] of the row whose RECORD a new row is cloned from.
///
/// The record only, not what it points at: its definition index is overwritten with
/// [`FLO_QUIT_ICON_DEFINITION`] straight after the copy. What is inherited is the fields this
/// repo has not decoded -- the `u16` at `+0x02` (`1` here, `0x3b` on the flash records), the kind
/// flags, the frame range -- and all three container rows carry the same values for those, so the
/// choice of 3 is arbitrary and only has to be a plain row rather than a flash.
pub const FLO_QUIT_TAB_ROW_TEMPLATE: usize = 3;

// ---------------------------------------------------------------------------------------------
// The added row's own definition: the Quit Game glyph, tinted, keeping the selection highlight.
//
// A row definition holds TWO children and both of them matter. The first is the icon; the second
// is a shape at `(6.9, -3.45)`, colour `00ffffff`, frames `1..69` -- transparent at rest, which is
// why it reads as decoration in the file and is in fact THE SELECTION HIGHLIGHT. Pointing the
// container's row record straight at the icon (`0x0254`) put the right glyph on screen and took
// the highlight away with it, which a run showed immediately.
//
// So the row gets a copy of row 2's definition with one child swapped, rather than a bare icon.
// ---------------------------------------------------------------------------------------------

/// Row 2's definition -- Quit Game -- the one the added row's own definition is copied from.
pub const FLO_QUIT_ROW_DEFINITION: u32 = 0x0258;

/// Children [`FLO_QUIT_ROW_DEFINITION`] declares, and which is which.
///
/// ```text
/// [0] def 0x0255 id 0x1eacd0 (8.10,  4.55)  the icon, paired with its greyed-out twin
/// [1] def 0x0257 id 0        (6.90, -3.45)  the selection highlight, alpha 0 at rest
/// ```
pub const FLO_QUIT_ROW_CHILDREN: usize = 2;
pub const FLO_QUIT_ROW_ICON: usize = 0;
pub const FLO_QUIT_ROW_HIGHLIGHT: usize = 1;

/// What child [`FLO_QUIT_ROW_ICON`] names in the shipped file, and what replaces it.
///
/// `0x0255` instantiates [`FLO_QUIT_ICON_DEFINITION`] **twice** -- once at `ffffffff` and once at
/// `ff808080` carrying id [`FLO_QUIT_ROW_DISABLED_ID`]. That second one is the greyed-out overlay
/// for a REFUSED quit, and it would never come off:
/// `FeGroupInGameGroupSelect::FUN_1400a77c0` walks the cells and its first act per cell is
/// `cmp DWORD PTR [rcx+0x4], 0` / `je` -- the entry's GATE. Only a gated row reaches the call at
/// `0x1400a78af` that resolves `0x1eacd0` under that cell and sets its visibility from the gate's
/// verdict. This crate's item is [`FE_INGAME_MENU_GATE_ALWAYS`], so the pass skips it, nothing
/// ever hides the twin, and the record's own `ff808080` is what draws.
///
/// `0x1eacd0` is the shared id for that overlay -- `0x0232`, `0x0238`, `0x023f` and `0x0255` all
/// use it, which is the four gated rows in this file: the three message rows (gates 1, 2, 3) and
/// quit (gate 4). It lines up exactly with the builder table in `docs/DS2-INGAME-MENU.md`.
pub const FLO_QUIT_ROW_ICON_GROUP: u32 = 0x0255;
pub const FLO_QUIT_ROW_DISABLED_ID: u32 = 0x001e_acd0;

/// What child [`FLO_QUIT_ROW_HIGHLIGHT`] names, checked and then copied through untouched.
pub const FLO_QUIT_ROW_HIGHLIGHT_DEFINITION: u32 = 0x0257;

/// The definition that IS the Quit Game icon, and nothing else.
///
/// The white copy of row 2's icon, with a child id of `0`, so substituting it for
/// [`FLO_QUIT_ROW_ICON_GROUP`] drops the grey twin and adds no duplicate id to the scene.
pub const FLO_QUIT_ICON_DEFINITION: u32 = 0x0254;

/// The index the added row's definition is filed under. Ours, like
/// [`FLO_ADDED_PANEL_DEFINITION`], and asked for only by our own record.
pub const FLO_ADDED_ROW_DEFINITION: u32 = 0xf258;

/// Index into [`FLO_QUIT_TAB_CHILD_IDS`] of the mark a new row's mark is cloned from.
pub const FLO_QUIT_TAB_MARK_TEMPLATE: usize = 6;

/// Element ids for the rows this repo adds, one per slot, chosen because the file contains none of
/// them.
///
/// **Absent from the whole file, not merely from the quit tab's container.** A row id is what the
/// namer resolves and what the substituted record carries, so an id used anywhere else in the
/// document is an id whose path could resolve to something that already exists. Scanned as raw
/// dwords over all 285088 bytes: of `0x1eacc0..0x1eacdf`, the free ones are `c0`-`c8`, `cc`, `cd`,
/// `ce`, `d3`, `d7` and `df` -- fifteen, against a ceiling of two per tab.
///
/// `0x1eaccd` is first because it is the one already on record: the earlier runtime experiment
/// that named it in the namer got `row-extent 3`, i.e. nothing resolved, which is the same answer
/// the file gives from the other side.
///
/// **There are twelve because the ceiling is now [`FEX_GRID_MAX_ROWS`] rather than the item
/// vector's five.** Re-running the scan over the current file gives 113 free ids in
/// `0x1eac00..0x1eacff`, of which `0x1eacc0..0x1eacc8`, `0x1eaccc`, `0x1eaccd`, `0x1eacce`,
/// `0x1eacd3`, `0x1eacd7` and `0x1eacdf` are the fifteen in the row block -- the same fifteen this
/// comment claimed before, by a script anyone can re-run:
///
/// ```text
/// python3 scripts/ds2-ebl.py extract /menu/02.febnd.dcx --out /tmp/menu02
/// python3 scripts/ds2-flo.py find /tmp/menu02/l02_01_In-Game.flo --id 0x1eaccd
/// ```
pub const FLO_ADDED_ROW_IDS: [u32; 12] = [
    0x001e_accd,
    0x001e_acce,
    0x001e_accc,
    0x001e_acc0,
    0x001e_acc1,
    0x001e_acc2,
    0x001e_acc3,
    0x001e_acc4,
    0x001e_acc5,
    0x001e_acc6,
    0x001e_acc7,
    0x001e_acc8,
];

/// Element ids for those rows' caption marks, one per slot.
///
/// **These are scoped to the container and only need to be free THERE.** `FeComponentObject`'s
/// `findByIdPath` matches one path component at a time against the record's `+0x1c`, so a label id
/// used under some other container is not a collision -- the path differs before it gets there.
/// Every id in `0x1eac40..0x1eac50` appears somewhere in this document; none of these appears among
/// [`FLO_QUIT_TAB_CHILD_IDS`].
///
/// `0x1eac4a` is first because it is the id the cut fourth row used -- its caption, `0x200f28`, is
/// still in the FMG and still reads "Mouse Settings".
///
/// **The first two and the other ten are chosen by different rules, deliberately.** Slots 0 and 1
/// are the two cut rows' own ids and are the pair that has actually been on screen; there is no
/// reason to move a working id for the sake of a tidy table. The remaining ten are ids the file does
/// not use ANYWHERE -- a strictly stronger property than the container-scope freedom a label needs,
/// and the cheap way to be sure of ten at once.
pub const FLO_ADDED_LABEL_IDS: [u32; 12] = [
    0x001e_ac4a,
    0x001e_ac4b,
    0x001e_aca0,
    0x001e_aca1,
    0x001e_aca2,
    0x001e_aca3,
    0x001e_aca4,
    0x001e_aca5,
    0x001e_aca6,
    0x001e_aca7,
    0x001e_aca8,
    0x001e_aca9,
];

/// How far apart consecutive added rows and their marks sit.
///
/// Both series are the shipped ones continued. The rows sit at `10.60`, `55.90`, `103.90` and the
/// marks at `17.55`, `65.95`, `114.35`; the last step of each is `48.00` and `48.40`, and those are
/// what a fourth and fifth row continue. **They are not the same number**, which is why the two
/// pitches are separate constants rather than one shared `48`.
pub const FLO_ROW_PITCH: f32 = 48.0;
pub const FLO_MARK_PITCH: f32 = 48.4;

/// Where the added row and its mark go, in the container's own coordinates.
///
/// The three shipped rows sit at y `10.60`, `55.90`, `103.90` and their marks at `17.55`, `65.95`,
/// `114.35`. Both series step by ~48 with +y downwards, so the fourth of each continues it:
/// `103.90 + 48.00` and `114.35 + 48.40`. The x is row 2's and mark 2's, unchanged -- the shipped
/// rows' x values wobble by a few units and there is no pattern in that to continue.
///
/// This is the ROW's position and not the icon's. The record names [`FLO_ADDED_ROW_DEFINITION`],
/// which places its own icon at `(8.10, 4.55)` inside it exactly as row 2 does, so the glyph lands
/// where row 2's would one step down.
pub const FLO_ADDED_ROW_XY: (f32, f32) = (-0.1, 151.9);
pub const FLO_ADDED_MARK_XY: (f32, f32) = (60.2, 162.75);

/// Where a tab's first row goes -- the shipped row 0's own position, read off the container.
///
/// Child 3 of [`FLO_QUIT_TAB_CONTAINER_DEFINITION`] is `(3.95, 10.60)` and child 6 -- its mark --
/// is `(60.20, 17.55)`. The x is the shipped row 0's rather than row 2's, because on a tab whose
/// rows all belong to this crate there is no shipped row above to line up with.
///
/// This is the origin a tab of our own uses, where [`FLO_ADDED_ROW_XY`] is the origin for rows
/// appended below three shipped ones. The two differ by three pitches, which is the three rows that
/// are not there on the seventh tab.
pub const FLO_FIRST_ROW_XY: (f32, f32) = (3.95, 10.6);
pub const FLO_FIRST_MARK_XY: (f32, f32) = (60.2, 17.55);

/// Byte offset of the packed colour inside a transform block, and the tint the added row's icon
/// is drawn with.
///
/// **The offset is read off the loader, not counted off the front of the struct.**
/// `FUN_140b50bc0`'s "is this child trivial enough to inline away" test ends
///
/// ```asm
/// test  DWORD PTR [rax+0x20], 0x10f
/// jne   not_trivial
/// cmp   BYTE PTR [rax+0x1b], 0xff        ; rax is the record's transform block
/// jne   not_trivial
/// ```
///
/// so `+0x1b` is a byte the builder requires to be `0xff` before it will flatten a child away.
/// That is the alpha: the 35 records in `l02_01_In-Game.flo` carrying `00ffffff` are the
/// transparent flash overlays, and every one of them fails that test rather than being inlined.
/// A field the builder refuses to flatten over is a field the draw applies.
///
/// **And the game itself demonstrates the tint on this very icon.** `0x0255` instantiates
/// [`FLO_QUIT_ICON_DEFINITION`] twice, at `ffffffff` and at `ff808080`, and the second one is the
/// greyed-out quit icon. One definition, two colours, two appearances, from this field alone --
/// which is also what proves the colour reaches the shape underneath, since the record carrying
/// `ff808080` is a nested record and the shape below it is `ffffffff`.
///
/// Byte order in memory is **R, G, B, A** -- see [`FLO_ADDED_ROW_TINT`], which had it backwards
/// and cost a run to find out. Only the alpha's position is readable from the file, via the
/// `+0x1b` test above; the file's opaque non-white records are all greys and blacks, so nothing in
/// it distinguishes R-first from B-first.
///
/// Note this does NOT disturb the census in [`FLO_TRANSFORM_FLAGS_OFFSET`]: "RGB is white" there
/// means the low three bytes of the little-endian `u32` are `ff ff ff`, which is the same set of
/// records under either reading.
///
/// **On its own the colour does nothing.** See [`FLO_TRANSFORM_FLAGS_OFFSET`].
pub const FLO_TRANSFORM_COLOUR_OFFSET: usize = 0x18;

/// Byte offset of the flag word beside it, and the two bits that make the colour mean anything.
///
/// **THE COLOUR IS INERT WITHOUT THESE, AND A RUN PROVED IT.** The first version of this wrote
/// `ffff6450` into a transform block copied from a row whose flags were `0`, and the icon came
/// back on screen in its shipped colour with no other symptom -- no refusal in the log, no crash,
/// nothing to read. The field is not a colour, it is a colour PLUS a licence to use it, and
/// copying a block from a row that never wanted one copies the licence's absence.
///
/// Both bits were then settled against all 1045 records in the file, and the split is total:
///
/// ```text
///  flags    n   white  non-white  alpha<ff
///  0x000  928     928          0         0
///  0x001    9       9          0         0
///  0x010   77       0         77        77    alpha only -- RGB is ffffff in all 77
///  0x011   19       0         19        19    likewise
///  0x110    5       0          5         0    RGB changed: ff000000, ff808080
///  0x111    3       0          3         0    RGB changed: 800080ff, cdff80ff, daff73e9
///  0x130    4       0          4         0    RGB changed: ff000000
/// ```
///
/// `0x10` set with a non-white colour: 108 records. `0x10` clear with a non-white colour: **zero**.
/// And every record carrying `0x100` has non-white RGB, while every record without it is
/// `xxffffff` -- varying alpha over white. So `0x10` is "the colour word is live" and `0x100` is
/// "and its RGB is not white".
///
/// An opaque re-skin needs both, which is exactly what `0x0255`'s grey twin carries: flags
/// `0x110`, colour `ff808080`. The one record in the file already doing what this crate wants to
/// do. Bit `0x20` (in `0x130`) and bit `0x1` are left alone -- `0x1` occurs on white records too,
/// so neither is about colour and neither is worth setting on a guess.
pub const FLO_TRANSFORM_FLAGS_OFFSET: usize = 0x20;
pub const FLO_TRANSFORM_COLOUR_LIVE: u32 = 0x0010;
pub const FLO_TRANSFORM_COLOUR_RGB: u32 = 0x0100;

/// The tint, as the four bytes it occupies in memory: **R, G, B, A**.
///
/// **A byte array and not a `u32`, because a `u32` is what got this wrong.** The first version was
/// `0xff_ff_64_50` under a doc comment claiming `0xAARRGGBB` stored B, G, R, A. Written
/// little-endian that lays down `50 64 ff ff`, and the run came back with a BLUE icon -- which is
/// `(0x50, 0x64, 0xff)` read straight through, R first.
///
/// Alpha last was already fixed, by the builder's `cmp BYTE PTR [rax+0x1b], 0xff`. The other three
/// were asserted from nothing, and **the file could not have settled them either way**: its only
/// opaque non-white records are `ff808080` and `ff000000`, greys and blacks, where the order does
/// not show. The three hued records are all translucent and all read plausibly under either
/// convention. So this was never a fact in the file waiting to be read -- it was a coin flip
/// written down as a measurement, and the run is what called it.
///
/// Red, opaque, in the order the bytes are actually laid down. The added row wears the shipped
/// Quit Game glyph, so without this it is the same icon as the row directly above it and the only
/// thing telling them apart is the caption. Red because the row is the one that does not ask.
///
/// **It is a strength and a hue rather than three bytes, because the colour MULTIPLIES.** The
/// game's own greyed-out state is the demonstration: `ff808080` on this very glyph reads as
/// disabled, and a flat mid-grey silhouette would not -- it darkens the artwork, so white is the
/// identity and anything below it composites down. Which means a fraction of a hue is meaningful,
/// and the fraction is the thing worth naming.
pub const FLO_ADDED_ROW_TINT: [u8; 4] = [
    toward_white(FLO_ADDED_ROW_HUE[0], FLO_ADDED_ROW_TINT_STRENGTH),
    toward_white(FLO_ADDED_ROW_HUE[1], FLO_ADDED_ROW_TINT_STRENGTH),
    toward_white(FLO_ADDED_ROW_HUE[2], FLO_ADDED_ROW_TINT_STRENGTH),
    0xff,
];

/// The hue [`FLO_ADDED_ROW_TINT`] is mixed from, at full strength. R, G, B.
pub const FLO_ADDED_ROW_HUE: [u8; 3] = [0xff, 0x64, 0x50];

/// How far [`FLO_ADDED_ROW_TINT`] is pushed from white toward [`FLO_ADDED_ROW_HUE`], out of `255`.
///
/// **This is the one number in this block that is taste and not measurement**, and it is on its
/// own line so it can be turned without touching the hue or the byte order that took a run each to
/// settle. Every value that has actually been on screen, and what it was called:
///
/// ```text
///  255  100% linear  #ff6450   "this is red"           -- a re-skin, a different KIND of row
///   26   10% linear  #fff0ee   "it looks like 0% red"  -- green moved 15 of 255
///   77   30% linear  #ffd1cb   "that looks like 10%"
///  120   47% linear  #ffb7ad   asked for as 33%
/// ```
///
/// **Linear is not perceptual, and the three earlier points say by how much.** A 10% mix showed
/// nothing and a 30% mix read as 10%, which fits a floor of roughly 20% before anything registers
/// and proportional response above it:
///
/// ```text
/// perceived  ~=  (linear - 0.20) / 0.80
/// ```
///
/// That model reproduces all three points (`10% -> 0`, `30% -> 13`, `100% -> 100`) and is what
/// picked `120` for a perceived third rather than another guess at the ramp. It is fitted to three
/// samples of one person's eye on one glyph over one background, so it is a working rule and not a
/// law -- but it beats halving the interval each time.
pub const FLO_ADDED_ROW_TINT_STRENGTH: u8 = 120;

/// Mix one channel `strength/255` of the way from white toward `channel`.
///
/// White is the identity for a multiply, so a partial tint is a partial step away from it.
const fn toward_white(channel: u8, strength: u8) -> u8 {
    (255 - ((255 - channel as u16) * strength as u16) / 255) as u8
}

/// Index of the alpha byte inside [`FLO_ADDED_ROW_TINT`]. It is the byte
/// [`FLO_TRANSFORM_COLOUR_OFFSET`]` + 3` lands on, which is the one the builder reads.
pub const FLO_TINT_ALPHA: usize = 3;

// ---------------------------------------------------------------------------------------------
// Captions: how a pause-menu row gets its text, and where the strings live.
//
// Reproduce the strings with:
//
//     python3 scripts/ds2-ebl.py extract /menu/text/english/ingamemenu.fmg --out /tmp/ds2text
//     python3 scripts/ds2-fmg.py /tmp/ds2text/ingamemenu.fmg --id 0x200f26 --id 0x200f2a
// ---------------------------------------------------------------------------------------------

/// `FeGroupInGameTopSelect::bindCaptions(this)`. RVA `0x000a7130`.
///
/// Ten `(scene path, FMG text id)` pairs, built on the stack at stride `0x38` with the text id at
/// `+0x30`, then a hardcoded ten-iteration loop that for each one does
///
/// ```text
/// FUN_140026790(this + 0x150, accessor, entry)   ; resolve the element
/// FUN_14003d870(command, 7, entry->textId)       ; kind 7 = "text by FMG id"
/// FUN_140029840(accessor + 0x30, command)        ; apply
/// ```
///
/// **The count is a literal in the code and the table is on its stack**, so an eleventh caption
/// cannot be added by substituting data the way the container's child list can. It has to be bound
/// by making the same calls again — which is what `ds2-menu-row`'s caption module does, using the
/// path the original itself just built rather than rebuilding one.
///
/// Three of the ten are the quit tab's, all under [`FE_QUIT_TAB_BASE_PATH`]:
///
/// ```text
/// label 0x1eac46  text 0x200f26  "Game Options"
/// label 0x1eac47  text 0x200f27  "Screen Options"
/// label 0x1eac4c  text 0x200f2a  "Quit Game"
/// ```
///
/// And the two ids the tab never binds are `0x200f28` "Mouse Settings" and `0x200f29` "Keyboard
/// Settings" — the PC port's cut input rows, which is what the two spare cells in
/// [`FE_QUIT_TAB_SPARE_CELL_IDS`] were for. That is the whole story of the missing fourth row,
/// told by the game's own text.
pub const FE_INGAME_TOP_SELECT_CAPTIONS: u32 = 0x000a_7130;

/// Byte offset of the scene holder [`FE_BIND_SCENE_OBJ_PROXY`] is called against, inside
/// `FeGroupInGameTopSelect`. `lea rcx,[rsi+0x150]` in the loop above.
pub const FE_INGAME_TOP_SELECT_SCENE_HOLDER_OFFSET: usize = 0x150;

/// Byte offset, inside the accessor [`FE_BIND_SCENE_OBJ_PROXY`] fills, of the slot the text calls
/// take. `lea rcx,[rbp+0x90]` against an accessor built at `rbp+0x60`.
pub const FE_ELEMENT_ACCESSOR_TEXT_SLOT_OFFSET: usize = 0x30;

/// Bytes the accessor occupies. The original gives it `rbp+0x60 .. rbp+0xf0`.
///
/// A second site agrees, and independently: `FUN_1400b7680`'s eight accessors are `0x90` apart
/// (`+0x90`, `+0x120`, `+0x1b0`, `+0x240`, `+0x2d0`, `+0x360`, `+0x3f0`, `+0x480`), built into
/// stack temporaries it never destroys -- which is also why a caller may use one and walk away.
/// One stack range and one stride, same number.
pub const FE_ELEMENT_ACCESSOR_SIZE: usize = 0x90;

/// `FeElement::setText(accessor + 0x30, string)`. RVA `0x000297d0`.
///
/// **It only READS the string**, which is what makes supplying one cheap. The layout it reads is
/// MSVC's small-string optimisation as `dantelion2` spells it: the capacity at `+0x18` decides
/// where the characters are, inline at `+0x00` when it is `<= 7` and behind the pointer at `+0x00`
/// otherwise. So a caption longer than seven characters is a four-field struct pointing at a
/// `static` in this DLL — no allocator, no constructor, nothing to free.
///
/// It then measures the UTF-16 length itself and calls the element's own `vtable[0x148]`.
pub const FE_ELEMENT_SET_TEXT: u32 = 0x0002_97d0;

/// Byte offset of the capacity field the string layout above is chosen by.
pub const DL_STRING_CAPACITY_OFFSET: usize = 0x18;
/// Capacity at or below which the characters are inline rather than behind the pointer.
pub const DL_STRING_INLINE_CAPACITY: u64 = 7;

/// The quit tab's bottom row, "Quit Game" — the one that returns to the title screen and offers to
/// save on the way. Its caption is retargeted so that the row this repo adds can be the one called
/// "Quit Game", which is what it actually does.
pub const FE_QUIT_TAB_ROW_TITLE_LABEL_ID: u32 = 0x001e_ac4c;

/// The label element id given to the added row.
///
/// `0x1eac4a` rather than something outside the pool: `0x1eac45 + n` is the shared row-label id
/// space the in-game menu screens draw from, and `0x1eac4a` is the one the cut fourth row used
/// (its caption, `0x200f28`, is still in the FMG and still reads "Mouse Settings"). Nothing in the
/// quit tab's container carries it, so nothing is shadowed.
pub const FLO_ADDED_ROW_LABEL_ID: u32 = 0x001e_ac4a;

/// How much taller the quit tab's panel has to be drawn for a fourth row to sit on it.
///
/// **Measured, not chosen.** All three menu tabs share panel definition `0x0221` at scale `(1,1)`;
/// its shape is `57.80 x 341.25` and its own translate puts it at panel-local `y = -59.90 ..
/// 281.35`. The three rows sit at panel-local `y = 113.6, 158.9, 206.9` and a row plate is
/// `48.80` tall, so the bottom row ends at `255.7` and the scroll's own bottom edge is `25.65`
/// below that.
///
/// **That 25.65 is the scroll's bottom MARGIN, and the first version of this constant spent it.**
/// It reached `303.7`, where the fourth row ends, which puts the scroll's bottom curl exactly on
/// the row's bottom edge -- arithmetically "covered" and visibly still short. The target is the
/// fourth row's end PLUS the margin the shipped rows are drawn with:
///
/// ```text
/// (303.7 + 25.65) / 281.35 = 1.1706
/// ```
///
/// Scaling is about the record's own origin, so the top edge moves up by 10.1 as well and the
/// header margin stretches by the same 17%. A scroll can absorb that; a nine-slice would be better
/// and this file does not obviously offer one.
///
/// It scales the panel's CURSOR with it -- `0x1eac81`'s definition carries both the scroll and the
/// highlight. Only the quit tab is affected, because what is scaled is this crate's own copy of
/// that tab's child record rather than the shared definition all three tabs instantiate.
/// **ANSWERED, AND THE ANSWER IS THAT THIS FIELD DOES NOTHING.** `1.0794` and then `1.1706` were both reported
/// as no visible change, and "17% of a 341-unit scroll is invisible" is not credible -- that is 58
/// units, more than a row. So the thing in doubt is no longer the factor, it is whether this field
/// reaches the scroll at all. `2.0` is deliberately unmissable, and because it is applied to ONE
/// axis it separates three outcomes in one look:
///
/// * the scroll is twice as TALL -- this offset is scale-y, and the factor goes back to `1.1706`;
/// * the scroll is twice as WIDE -- this offset is scale-X, and the sibling at `+0x08` is the one;
/// * nothing moves -- the panel record is not what draws the scroll, and the next step is reading
///   the 17 quads in shape `0x0220`'s geometry rather than guessing a fourth time.
///
/// It was the third: nothing moved on either axis. The tree walk then found the reason -- the panel
/// draws through a `FeComponentTextureShape`, which is sized by its own quad
/// ([`FE_TEXTURE_SHAPE_DEST_RECT_OFFSET`]) and never re-derives it from an ancestor's transform.
///
/// So this is `1.0`: the substitution still copies the panel's record like every other, and the
/// scale it writes is the one the game shipped. A number that provably changes nothing has no
/// business sitting in a table of measurements pretending otherwise.
pub const FLO_PANEL_STRETCH_Y: f32 = 1.0;

/// Index into [`FLO_QUIT_TAB_CHILD_IDS`] of the panel the stretch applies to.
pub const FLO_QUIT_TAB_PANEL: usize = 0;

/// `f32` scale-x inside a transform block. `pfVar1[2]` in the builder's identity test, and the
/// pair is visible in the file: every shipped block reads `1.0, 1.0` here except the item icon's
/// (`l02_03_equipment.flo` transform `0x010470`), which carries `0.810806, 0.810806`.
pub const FLO_TRANSFORM_SCALE_X_OFFSET: usize = 0x08;

/// `f32` scale-y inside a transform block. `pfVar1[3]` in the builder's identity test.
pub const FLO_TRANSFORM_SCALE_Y_OFFSET: usize = 0x0c;

// ---------------------------------------------------------------------------------------------
// The live component tree, for finding an element the file alone will not identify.
//
// Three stretch factors were tried on the quit tab's panel record and the third one -- scale 2.0,
// deliberately unmissable -- changed nothing on screen while the log proved the write landed. So
// the element being scaled is not the one that draws the banner, and no amount of further
// arithmetic on the `.flo` fixes that. What is needed is the live tree: what is actually built
// under this tab, and which of it is big enough to be the background.
// ---------------------------------------------------------------------------------------------

/// `FeLayoutScene::findByIdPath(scene, ids, count)`. RVA `0x00afdad0`.
///
/// `mov rcx,[rcx+0x28]` then a tail-jump into the search; returns the component or null. This is
/// the lookup every scene path in the frontend bottoms out in, so resolving a path by hand and
/// resolving it the way the grid does are the same operation.
pub const FE_SCENE_FIND_BY_ID_PATH: u32 = 0x00af_dad0;

/// Byte offset, inside a `FrontendEx::SceneObjProxy`, of the scene proxy its resolve reads.
///
/// `SceneObjProxy::resolve` (`0x140027ce0`) opens `mov rcx,[rcx+0x58]; mov rax,[rcx]; call
/// [rax+8]`, so the scene comes from slot 1 of whatever lives here.
pub const FE_SCENE_OBJ_PROXY_SCENE_OFFSET: usize = 0x58;
/// Vtable slot on that object which returns the scene.
pub const FE_SCENE_PROXY_GET_SCENE_SLOT: usize = 0x08;

/// Component tree links, read off `FUN_140b77dc0` and `FeComponentObject::findByIdPath`.
///
/// ```text
/// child   = [parent + 0x38]      first child
/// child   = [child  + 0x28]      next sibling
/// record  = [child  + 0x48]      the `.flo` record, whose +0x1c is the element id
/// ```
///
/// `+0x48` is a RECORD and not a definition, which matters because both structs carry a `u16` at
/// `+0x02` and `FUN_140b6bd80` bounds the display list by exactly that field. Three readings, two
/// static and one live, agree: `findByIdPath` at `0x140b6a141` is `mov rax,[rcx+0x48]; cmp
/// [rax+0x1c],r9d` and [`FLO_RECORD_ID_OFFSET`] is `0x1c`; the display-list bound reads `+0x02`;
/// and `ds2-item-warn` read the field off two live components in one frame -- its badge and the
/// infusion glyph it is cloned from differed by exactly `9 * FLO_RECORD_STRIDE`, while both name
/// definition `0x005a`, so a pointer to a definition would have been equal.
pub const FE_COMPONENT_NEXT_SIBLING_OFFSET: usize = 0x28;
pub const FE_COMPONENT_FIRST_CHILD_OFFSET: usize = 0x38;
pub const FE_COMPONENT_RECORD_OFFSET: usize = 0x48;

/// Where a component's own transform starts, which is NOT the same for every class.
///
/// `FeComponentObject`'s constructor (`0x140b69d10`) writes a 4x3 identity at `+0x60`;
/// `FeComponentScene`'s (`0x140b6b730`) writes the same identity at `+0x50`. They are siblings
/// under `FeComponentBase`, not parent and child, which is why the offsets differ -- so a dump
/// covers both ranges and the vtable says which one to read.
pub const FE_COMPONENT_TRANSFORM_DUMP_START: usize = 0x50;
pub const FE_COMPONENT_TRANSFORM_DUMP_END: usize = 0xa0;

/// The ONLY two classes whose `+0x38` is a child list, as vtable RVAs.
///
/// **This cost a crash.** `FUN_140b77dc0` reads `[parent+0x38]` and every component in the tree
/// looked like a parent, so a walk that followed that offset unconditionally descended into a
/// `FeComponentTextureShape`, read some unrelated field as a pointer, and recursed until it died
/// -- `exception_address=DINPUT8.dll+0x769ad` with five identical frames above it.
///
/// The classes that recurse are the ones whose `findByIdPath` (vtable `+0x190`) reaches
/// `FUN_140b77dc0`: `FeComponentObject` matches its own id first and then descends,
/// `FeComponentScene` has no id and descends immediately. Every other `FeComponent*` overrides that
/// slot with something else and is a leaf as far as the tree is concerned.
///
/// Names from `scripts/ds2-rtti-vtables.py 'FeComponent'`.
pub const FE_COMPONENT_OBJECT_VTABLE: u32 = 0x011d_dfa8;
pub const FE_COMPONENT_SCENE_VTABLE: u32 = 0x011d_e158;

/// The display list, its live count, and its stride, inside a `FeComponentSprite`.
pub const FE_COMPONENT_DISPLAY_LIST_OFFSET: usize = 0x70;
pub const FE_COMPONENT_DISPLAY_COUNT_OFFSET: usize = 0x66;
pub const FE_COMPONENT_DISPLAY_ENTRY_STRIDE: usize = 0x10;
/// Offset of the child pointer and of the id key inside one display-list entry.
pub const FE_COMPONENT_DISPLAY_ENTRY_CHILD_OFFSET: usize = 0x00;
pub const FE_COMPONENT_DISPLAY_ENTRY_KEY_OFFSET: usize = 0x0c;

/// Classes whose `findByIdPath` is `xor eax,eax; ret` (`0x140b6d2a0`) -- genuine leaves:
/// `FeComponentLinked`, `FeComponentMaskShape`, `FeComponentTextureMask`,
/// `FeComponentTextureShape`. Following `+0x38` on one of these is what crashed the first walk.
pub const FE_COMPONENT_LEAF_FIND_BY_ID_PATH: u32 = 0x00b6_d2a0;

// ---------------------------------------------------------------------------------------------
// FeComponentTextureShape: the thing that actually draws the quit tab's banner.
//
// Three stretch factors on ancestor transforms did nothing, and this is why: a texture shape is
// sized by its OWN quad, copied into it at build time and never re-derived from a parent.
// ---------------------------------------------------------------------------------------------

/// `FeComponentTextureShape`'s vtable. RVA `0x011dea18`, VA `0x1411dea18`. From MSVC RTTI.
pub const FE_COMPONENT_TEXTURE_SHAPE_VTABLE: u32 = 0x011d_ea18;

/// `FeComponentTextureShape::initFromShape(this, allocator, shapeEntry)`. RVA `0x00b70200`.
///
/// Everything below is read off it. It allocates four parallel arrays, one element per quad, and
/// fills them from the shape table entry's sub-records:
///
/// ```text
/// count = [[this+0x40] + 0x02]                 quads in this shape
/// [this+0x48]  count * 0x30                    per-quad vertex block, seeded from a constant
/// [this+0x50]  count * 0x10                    four floats per quad   <- the RECT
/// [this+0x58]  count * 0x10                    four floats per quad   <- the second RECT
/// [this+0x60]  count * 0x04                    per-quad colour, from sub-record +0x18..+0x1b
/// ```
///
/// For each quad, `sub = [[this+0x40] + 0x08] + i * 0x40` and `geom = [sub + 0x30]`:
///
/// * `geom != 0` -- both rects are filled with the SAME four floats from `geom[0..3]`;
/// * `geom == 0` -- both are filled with `{0, 0, w, h}` taken from `[[sub+0x20] + 0x0e]` and
///   `+0x10`, i.e. the texture's own pixel size.
///
/// The quit tab's banner is shape `0x0220`, `count = 1`, and its single quad reads
/// `(914.20, 1.10, 972.00, 342.35)` -- `57.80 x 341.25`, which is the `341.25` the panel
/// measurements were built on. The sub-record's own translate is `(-914.30, -61.00)`, cancelling
/// the atlas origin, so these are atlas coordinates mapped into layout space.
pub const FE_TEXTURE_SHAPE_INIT: u32 = 0x00b7_0200;

/// The shape table entry a texture shape was built from, and the count field inside it.
pub const FE_TEXTURE_SHAPE_ENTRY_OFFSET: usize = 0x40;
pub const FE_SHAPE_ENTRY_COUNT_OFFSET: usize = 0x02;

/// The two per-quad rect arrays, `0x10` bytes each: DESTINATION and SOURCE.
///
/// The initialiser seeds them identically and so cannot tell them apart. The DRAW can:
/// `FeComponentTextureShape`'s render (`0x140b6f200`, vtable slot 46) ends every quad with
///
/// ```text
/// FUN_140b521c0(ctx, [this+0x50] + i*0x10, colour, [this+0x58] + i*0x10)
/// ```
///
/// and in the branch where a texture is actually bound it **replaces the fourth argument** with a
/// local `{0, 0, texWidth, texHeight}` read from `[tex+0x40]` and `[tex+0x44]`. A rect that can be
/// substituted by the texture's own pixel size is the SOURCE.
///
/// So growing both is what put art on the added row that was mostly transparent -- the destination
/// made room and the source pulled in whatever sits below the banner in the atlas. Growing the
/// destination alone stretches the shipped art to fill instead.
pub const FE_TEXTURE_SHAPE_DEST_RECT_OFFSET: usize = 0x50;

/// The per-quad matrix array on a `FeComponentTextureShape`, `0x30` bytes per quad.
///
/// `FUN_140b70200` allocates it as `quads * 0x30` at `+0x48` -- alongside the destination and
/// source rects at `+0x50` and `+0x58`, which it allocates as `quads * 0x10` -- and seeds each one
/// from the constants at `_FLOAT_141596af0..`. The art's composed position is this translation
/// plus the destination rect's corner, which is why the quad's own offset
/// (`(-934.70, -52.50)` for the infusion arrow) cancels a rect that starts at `(934.70, 52.50)`
/// and leaves the art sitting on its record's origin.
pub const FE_TEXTURE_SHAPE_QUAD_MATRIX_OFFSET: usize = 0x48;

/// Which two of those twelve floats are the translation: `3` and `7`.
///
/// `FUN_140b53c10` is what says so. It reads the twelve and writes a 4x4 whose rows are
/// `(p0,p4,p8,c)`, `(p1,p5,p9,c)`, `(p2,p6,p10,c)`, `(p3,p7,p11,c)` -- a transpose. So the stored
/// block is a row-major `3x4` whose last column is `p3, p7, p11`, and transposing it puts that
/// column in the 4x4's last row, which is where row-vector math (`v' = v * M`) keeps a translate.
///
/// **At the cell bind this block is still identity, and a run says so.** `ds2-item-warn` logged all
/// twelve on four consecutive badges and got `[1,0,0,0, 0,1,0,0, 0,0,1,0]` every time -- exactly
/// what `FUN_140b70200` seeds from `_FLOAT_141596af0..`, translation included, which is to say
/// none. So whatever folds a quad's own offset in -- and the cloned infusion glyph's `.flo` quad
/// carries `(-934.70, -52.50)` against a rect starting at `(934.70, 52.50)` -- does it after the
/// bind and not here.
///
/// These indices are where a translation is read from, then. They are not a claim that one is in
/// the block when `ds2-item-warn` looks: reading it at bind time measures the seed.
pub const FE_TEXTURE_SHAPE_QUAD_MATRIX_TRANSLATE: [usize; 2] = [3, 7];
pub const FE_TEXTURE_SHAPE_SOURCE_RECT_OFFSET: usize = 0x58;
pub const FE_TEXTURE_SHAPE_RECT_STRIDE: usize = 0x10;

/// The display-list key the panel's texture shape is filed under.
///
/// Not an element id -- `FUN_140b6bd80` sets a child's key from `FUN_140b6a440(child)`, and a shape
/// with no id of its own lands on `0xffffffff`. Observed on every texture shape in the live tree.
pub const FE_TEXTURE_SHAPE_DISPLAY_KEY: u32 = 0xffff_ffff;

/// How much taller the banner's quad has to be for a fourth row.
///
/// The quad is `341.25` tall and covers three row slots with `25.65` of margin below the last.
/// One more row is one more `48.00` of pitch, so `341.25 + 48.00 = 389.25` keeps the margin
/// exactly. As a rect that is `y1 = 1.10 + 389.25 = 390.35`.
pub const FE_BANNER_QUAD_Y1: f32 = 390.35;
/// What the shipped quad's `y1` reads, checked before anything is written.
pub const FE_BANNER_QUAD_SHIPPED_Y1: f32 = 342.35;

/// Where the banner's quad has to end for `rows` added rows: one pitch per row, margin preserved.
///
/// The same series [`caret_y`] walks, and for the same reason -- both follow the last row, which
/// moves down one [`FLO_ROW_PITCH`] per row added below the shipped three.
pub const fn banner_y1(rows: usize) -> f32 {
    FE_BANNER_QUAD_SHIPPED_Y1 + FLO_ROW_PITCH * rows as f32
}

/// [`banner_y1`] for a tab whose rows start at [`FLO_FIRST_ROW_XY`] rather than below three shipped
/// ones: the same series, lifted by [`FLO_FIRST_ROW_RISE`], exactly as [`caret_y_from_top`] is.
///
/// The lift is what stops the seventh tab's panel from being three rows longer than its list.
/// Without it a tab of our own carrying two rows is sized for five, and the three rows below the
/// last one are empty panel.
pub const fn banner_y1_from_top(rows: usize) -> f32 {
    banner_y1(rows) - FLO_FIRST_ROW_RISE
}

/// The panel definition the quit tab's `0x1eac81` instantiates, and the caret inside it.
///
/// `0x0221` holds exactly two children: the banner shape `0x0220` at `(0, 0)`, and `0x004e` at
/// `(40.85, 244.65)` -- panel-local, which is container y `141.65`, just below the third row's mark
/// at `114.35`. That second one is the scroll caret.
///
/// **It is SHARED.** All three menu tabs' panels instantiate `0x0221`, so moving the caret inside it
/// moves it on every tab. The quit tab therefore gets its own copy of the definition under an index
/// nothing else asks for, reached the same way the container is: our record names it, and the
/// lookup detour answers it.
pub const FLO_PANEL_DEFINITION: u32 = 0x0221;
/// Index of the caret among `0x0221`'s two children.
pub const FLO_PANEL_CARET: usize = 1;
/// Children `0x0221` declares.
pub const FLO_PANEL_CHILDREN: usize = 2;

/// The definition index the quit tab's own panel copy is filed under.
///
/// Outside the file's own range -- `l02_01_In-Game.flo` declares 342 definitions and the highest
/// index seen is `0x0272` -- so a lookup for it can only come from the record this crate wrote.
pub const FLO_ADDED_PANEL_DEFINITION: u32 = 0xf221;

/// Where the caret goes: down from the shipped `244.65` by one row pitch per added row.
///
/// **It used to be the constant `292.65` -- `244.65 + 48.00` -- and one row's worth of movement was
/// wrong the moment a second row could be registered.** The caret sits just below the last row, so
/// what it follows is the number of rows, exactly as the banner's quad does in
/// [`FE_BANNER_QUAD_SHIPPED_Y1`]'s consumer. A fixed offset put it under row 4 on a tab showing five.
///
/// The value is the caret's own authored y in PANEL-LOCAL coordinates, so the panel's own position
/// does not enter into it.
pub const fn caret_y(rows: usize) -> f32 {
    FLO_CARET_SHIPPED_Y + FLO_ROW_PITCH * rows as f32
}

/// How much higher a tab's first row sits than the first APPENDED row. `141.30`.
///
/// `151.90 - 10.60`. **Not three pitches**, which would be `144.00`: the three shipped rows step by
/// `45.30` then `48.00`, so the distance they actually span is `93.30` and only the step past them is
/// a clean pitch. Deriving this as `3 * FLO_ROW_PITCH` is off by `2.70`, which is the sort of number
/// that reads as correct in a comment and wrong on a screen.
pub const FLO_FIRST_ROW_RISE: f32 = FLO_ADDED_ROW_XY.1 - FLO_FIRST_ROW_XY.1;

/// [`caret_y`] for a tab whose rows start at [`FLO_FIRST_ROW_XY`] rather than below three shipped
/// ones: the same series, lifted by [`FLO_FIRST_ROW_RISE`].
pub const fn caret_y_from_top(rows: usize) -> f32 {
    caret_y(rows) - FLO_FIRST_ROW_RISE
}
/// What the shipped caret's y reads, checked before anything is written.
pub const FLO_CARET_SHIPPED_Y: f32 = 244.65;

// ---------------------------------------------------------------------------------------------
// THE TAB STRIP'S OWN RECORDS, WHICH ARE A ROW CONTAINER ONE LEVEL UP
//
// The strip is a container with six cell records in it, and its child count is its display-list
// capacity, exactly as a tab's row container is. So a seventh tab's icon is the same substitution
// `ds2-menu-row`'s layout module already performs: raise the count, append a record copied from the
// last one with a new x and a new element id.
// ---------------------------------------------------------------------------------------------

/// The tab strip's container definition. `0x0271`, with [`FLO_TAB_STRIP_CHILDREN`] children.
///
/// Reached as child 0 of definition `0x0272`, under element id `0x1eaba9` -- which is the same id
/// [`FE_INGAME_TOP_SELECT_NAMER`] uses as its base path, so the code and the layout agree on what
/// the strip is.
pub const FLO_TAB_STRIP_DEFINITION: u32 = 0x0271;

/// Children the shipped strip carries, checked before the count is raised. Eighteen.
///
/// The last six are the tab cells; the first twelve are the strip's own furniture -- the frame, the
/// `LB`/`RB` prompts and the two captions.
pub const FLO_TAB_STRIP_CHILDREN: usize = 18;

/// Index of the first tab-cell record inside [`FLO_TAB_STRIP_DEFINITION`]'s child array. Twelve.
pub const FLO_TAB_STRIP_FIRST_CELL: usize = 12;

/// The definition every tab cell shares. `0x0270`.
///
/// Two children, both the same `0x026f` highlight shape at differing frame ranges, and no icon: the
/// glyph on a tab is bound by the grid control rather than authored here. A seventh cell reusing
/// this definition therefore inherits the selection highlight and nothing else, which is what a tab
/// whose icon is bound at runtime needs.
pub const FLO_TAB_STRIP_CELL_DEFINITION: u32 = 0x0270;

/// The six tab cells' element ids, in child-array order.
///
/// Not the same order as [`FE_INGAME_TOP_SELECT_NAMER_CELL_IDS`] pushes them in -- the namer pushes
/// `..aba6, ..aba7, ..aba5` where the array holds `..aba6, ..aba7, ..aba5` at 15, 16, 17. They agree
/// here; the ORDER that differs is [`FE_INGAME_TOP_SELECT_TAB_ORDER`], which is about the groups.
pub const FLO_TAB_STRIP_CELL_IDS: [u32; FE_INGAME_TOP_SELECT_TABS] = [
    0x001e_aba2,
    0x001e_aba3,
    0x001e_aba4,
    0x001e_aba6,
    0x001e_aba7,
    0x001e_aba5,
];

/// The element id a seventh tab's cell is written under. `0x1eaba8`.
///
/// The one gap in the strip's own block: `scripts/ds2-flo.py find` reports no record for it in
/// `l02_01_In-Game.flo`, while every id either side of it resolves. Taking the neighbour rather than
/// a free id from the row block keeps the strip's records reading as one run.
pub const FLO_ADDED_TAB_ID: u32 = 0x001e_aba8;

/// Horizontal spacing between tab cells. `54.0`.
///
/// The shipped six sit at `-4.6, 49.4, 103.4, 157.4, 211.55, 265.05`. Four of the five gaps are
/// `54.0` and the other two are `54.15` and `53.5`, which is authoring drift rather than a second
/// pitch -- so a seventh at `265.05 + 54.0` lands where the eye expects it.
pub const FLO_TAB_PITCH: f32 = 54.0;

/// Depth step between tab cells. Four.
///
/// `69, 73, 77, 81, 85, 89`. A seventh takes `93`, which is below the strip's own furniture and
/// above nothing -- it is the last record either way.
pub const FLO_TAB_DEPTH_PITCH: u16 = 4;

// ---------------------------------------------------------------------------------------------
// THE SEVENTH TAB'S OWN PANEL SUBTREE
//
// A row record is a grid CELL: the tab's namer names it, and a cell no namer names is not drawn.
// A mark record is a plain child, and a plain child draws whenever its container is posed. That
// asymmetry is why the seventh tab's first run put the System tab's three captions letter-over-
// letter on top of its own rows -- both tabs were posing one container, our namer could suppress
// the shipped rows and nothing could suppress the shipped marks.
//
// The level that separates two tabs is not the container. Every tab already owns a subtree hung
// off the strip, and only the posed one draws:
//
//     0x0271  the strip
//       [ 4]  id 0x1eaccf  def 0x0265   the System tab's subtree
//               [0] id 0x1eace8  def 0x0264
//                     [0] id 0x1eace6  def 0x0263   the row container
//       [12..17]  the six tab cells
//
// So the seventh tab gets a second such child: three chained copies with our rows in the last one
// and the shipped rows and marks left behind in the original. The copies differ from the originals
// in one field each -- the definition index the child record names -- because the whole point is to
// reach a different `0x0263`.
//
// Reproduce with:
//
//     python3 scripts/ds2-flo.py tree /tmp/menu02/l02_01_In-Game.flo --def 0x271
//     python3 scripts/ds2-flo.py tree /tmp/menu02/l02_01_In-Game.flo --def 0x265
//     python3 scripts/ds2-flo.py find /tmp/menu02/l02_01_In-Game.flo --id 0x1eaceb
//
// ---------------------------------------------------------------------------------------------

/// Index of the System tab's subtree record inside [`FLO_TAB_STRIP_DEFINITION`]'s child array.
///
/// The template the seventh tab's own subtree record is cloned from, and the reason the clone is
/// inserted beside it rather than appended: a nested-definition record's depth is never read back
/// (`FUN_140b50bc0` passes depth on for leaf kinds only), so the draw order of two subtrees is the
/// order they sit in the array. Appending ours after the six cells would draw a tab's panel over
/// the tab strip.
pub const FLO_TAB_STRIP_PANEL: usize = 4;

/// The element id at [`FLO_TAB_STRIP_PANEL`], which is also
/// [`FE_QUIT_TAB_BASE_PATH`]`[1]` -- the component every one of the System tab's cell paths carries
/// and the one the seventh tab rewrites.
pub const FLO_TAB_STRIP_PANEL_ID: u32 = 0x001e_accf;

/// The definition [`FLO_TAB_STRIP_PANEL`] names. `0x0265`, two children.
pub const FLO_TAB_SUBTREE_DEFINITION: u32 = 0x0265;
/// Children `0x0265` carries: the frame below, and a `0x0251` that plays over frames 23..29.
pub const FLO_TAB_SUBTREE_CHILDREN: usize = 2;
/// Index of the child of `0x0265` that names [`FLO_TAB_FRAME_DEFINITION`].
pub const FLO_TAB_SUBTREE_FRAME: usize = 0;
/// That child's element id, checked before the copy is made.
pub const FLO_TAB_SUBTREE_FRAME_ID: u32 = 0x001e_ace8;

/// The definition between the subtree and the row container. `0x0264`, one child.
pub const FLO_TAB_FRAME_DEFINITION: u32 = 0x0264;
/// Children `0x0264` carries. One: the row container.
pub const FLO_TAB_FRAME_CHILDREN: usize = 1;
/// Index of the child of `0x0264` that names [`FLO_QUIT_TAB_CONTAINER_DEFINITION`].
pub const FLO_TAB_FRAME_CONTAINER: usize = 0;
/// That child's element id, which is [`FE_QUIT_TAB_BASE_PATH`]`[3]`.
pub const FLO_TAB_FRAME_CONTAINER_ID: u32 = 0x001e_ace6;

/// The element id the seventh tab's subtree is written under. `0x1eaceb`.
///
/// Free in the shipped document: `scripts/ds2-flo.py find --id 0x1eaceb` reports no record, while
/// `0x1eace6` through `0x1eace9` all resolve. Taking a neighbour of the block it sits beside keeps
/// the tab's ids reading as one run, the way [`FLO_ADDED_TAB_ID`] does for the cell.
///
/// Only this one id is new. The copies below reuse [`FLO_TAB_SUBTREE_FRAME_ID`] and
/// [`FLO_TAB_FRAME_CONTAINER_ID`] verbatim, because a path is resolved one level at a time and the
/// two subtrees diverge at the level above -- which is also what lets the seventh tab's cell paths
/// differ from the System tab's in exactly one component.
pub const FLO_ADDED_TAB_SUBTREE_ID: u32 = 0x001e_aceb;

/// Indices the three copies are served under, none of which the shipped document uses.
///
/// Same arrangement as [`FLO_ADDED_PANEL_DEFINITION`]: a lookup for one of these can only have come
/// from a record this crate wrote, so the answer is whatever copy was built last.
///
/// `0xe000` rather than the `0xf000` the panel and the rows use, and the reason is arithmetic
/// rather than taste. [`FLO_ADDED_ROW_DEFINITION`] is `0xf258` and claims one index per slot, so
/// the rows own `0xf258` through `0xf263` -- which swallows the `0xf263` and `0xf264` this block
/// would otherwise have taken, and the collision is silent: a lookup for the last slot's row would
/// have been answered with the seventh tab's container. A test in `ds2-menu-row` asserts the two
/// blocks stay apart; it is the test that found this.
pub const FLO_ADDED_TAB_SUBTREE_DEFINITION: u32 = 0xe265;
pub const FLO_ADDED_TAB_FRAME_DEFINITION: u32 = 0xe264;
pub const FLO_ADDED_TAB_CONTAINER_DEFINITION: u32 = 0xe263;

/// Byte offset of the layout path inside a tab descriptor. `0x38`.
///
/// `FUN_1400a5900` builds a `DLKR::DLFixedVector<u32, 8>` on its own stack, writes `0x1eaba9` and
/// `0x1eaccf` into it, and copies it here with `FUN_1400a4630(descriptor + 0x38, &local)`. The
/// group constructor then copies that into `group + 0x130`, and
/// [`FE_INGAME_MENU_TAB_INIT`] resolves it and plays sequence `0x65` on what comes back. So this
/// field is which subtree a tab poses, and a seventh tab that poses its own writes one dword here.
pub const FE_INGAME_MENU_TAB_PATH_OFFSET: usize = 0x38;

/// The two ids `FUN_1400a5900` writes into that path: the strip, then the System tab's subtree.
///
/// Verified before the second is rewritten, for the same reason every other copy in this crate is
/// verified: a descriptor that does not hold these is not the one this was read from.
pub const FE_INGAME_MENU_TAB_PATH: [u32; 2] = [0x001e_aba9, FLO_TAB_STRIP_PANEL_ID];

/// The path's count field, at [`FE_SCENE_NAMER_ENTRY_LEN_OFFSET`] past its base -- so `0x60` in the
/// descriptor, which is the qword `FUN_1400a5900` zeroes on its second instruction.
pub const FE_INGAME_MENU_TAB_PATH_COUNT_OFFSET: usize =
    FE_INGAME_MENU_TAB_PATH_OFFSET + FE_SCENE_NAMER_ENTRY_LEN_OFFSET;

// =================================================================================================
// THE TAB ICONS, WHICH ARE ONE BAKED QUAD AND NOT SIX ELEMENTS
//
// A seventh tab was reachable, highlighted and drew its rows, and wore no icon. Two readings were
// tried and both were wrong: that the glyph is bound into the cell by the grid -- it is not, the
// cell definition `FLO_TAB_STRIP_CELL_DEFINITION` holds two copies of one highlight shape and
// nothing inside it carries an element id, so no path can reach into a cell and no bind can put a
// glyph there -- and that answering the strip's second per-cell lookup would supply it, which
// changed nothing, because that lookup resolves the same entry to a second accessor.
//
// The strip's eighteen children are: four tab panels, a mask, two captions, the `LB`/`RB` labels,
// three texture leaves, and the six cells. The six hexagons and the six glyphs are inside one of
// those leaves -- `FLO_TAB_PLATE_SHAPE`, a single textured quad that samples
// `(1.10, 781.95)-(337.60, 850.90)` out of the 1024x1024 atlas `In-game_01` and lands it at
// `(1.10, 6.25)-(337.60, 75.20)`, which is exactly the band the six tabs occupy. There is no
// seventh hexagon anywhere in that atlas: the art immediately right of the plate belongs to
// another shape, and the plate's own right edge is its last hexagon's.
//
// So the seventh tab's icon is not a lookup to answer. It is a quad to draw.
//
// The quad this draws is the plate's own last pitch, repeated one pitch right. The six hexagons
// sit at `FLO_TAB_PITCH`, so the plate's art is periodic at that pitch, and a copy of its final
// `54.0` of atlas placed immediately past its right edge continues the row -- the seam falls
// between two hexagons, where the art repeats, and the glyph inside the new hexagon is the sixth
// tab's glyph shifted whole. That is the one construction needing no number this file cannot
// check: not the hexagon's width, not its offset inside a cell, only the pitch the six cells
// already spell.
//
// The seventh tab therefore wears the sixth tab's glyph. The atlas has six and this mod ships no
// texture of its own; a duplicate glyph in a hexagon that is the right shape, the right size and
// in the right place is what is available. `FLO_ADDED_TAB_ICON_SOURCE_LEFT` is the one number to
// change if a different slice is ever wanted.
//
// Every offset below was read off the function that reads it, not matched to a pattern:
//
//   `FUN_140b54780(doc, index)`  the shape lookup: scan of `[doc+0x08]` over `[doc+0x48]` entries,
//                                stride 0x18, key = the `u16` at `+0x00`, returning the entry.
//   `FUN_140b70200(this, .., e)` `FeComponentTextureShape::init`: `movzx ecx,[e+0x02]` is the quad
//                                count, `[e+0x08] + (i << 6)` is quad `i` -- stride 0x40.
//   `FUN_140b70200` again        `rax = [quad+0x30]`, then four floats `[rax]`..`[rax+0x0c]`
//                                copied into two per-quad buffers: the source rect.
//   `FUN_140b50bc0`              `movzx edx,[rec]` -- a `kind & 1` record's `+0x00` is the shape
//                                index, the same field a `kind & 4` record uses for a definition.
//
// Reproduce the numbers with:
//
//     python3 scripts/ds2-flo.py tree /tmp/menu02/l02_01_In-Game.flo --def 0x271
//
// =================================================================================================

/// `FeLayoutDocument::findShape(doc, index)`. RVA `0x00b54780`.
///
/// `fn(&doc, u32 index) -> *entry`. A linear scan of `[[doc]+0x08]` over `[[doc]+0x48]` entries at
/// stride [`FLO_SHAPE_STRIDE`], keyed by the `u16` at `+0x00`; `mov rax,rcx; ret` on a hit and
/// null on a miss, so a detour that declines returns what the trampoline gave it.
///
/// Three sibling lookups share its shape and its prologue -- `0x00b54700` is the mask table,
/// `0x00b54740` is [`FLO_FIND_DEFINITION`], `0x00b547c0` is the text table. So the prologue check
/// proves the bytes are a lookup; it does not prove which of the four. The rva is what says that,
/// and it came from the call site at `0x140b50d18`, reached with `kind & 1`.
pub const FLO_FIND_SHAPE: u32 = 0x00b5_4780;

/// `mov rax,[rcx]; mov r9d,edx; test rax,rax`. Shared with the three sibling lookups; see above.
pub const FLO_FIND_SHAPE_PROLOGUE: [u8; 9] = [0x48, 0x8b, 0x01, 0x44, 0x8b, 0xca, 0x48, 0x85, 0xc0];

/// Bytes per shape-table entry. `FUN_140b54780`: `add rcx,0x18`.
pub const FLO_SHAPE_STRIDE: usize = 0x18;
/// `u16` shape index inside an entry -- the key the scan compares.
pub const FLO_SHAPE_KEY_OFFSET: usize = 0x00;
/// `u16` how many quads the shape holds. `FUN_140b70200`: `movzx ecx,WORD PTR [rax+0x2]`, used to
/// size four allocations before anything is read.
pub const FLO_SHAPE_QUAD_COUNT_OFFSET: usize = 0x02;
/// Pointer to the quad array. `FUN_140b70200`: `add r8,QWORD PTR [rax+0x8]` after `shl r8,0x6`.
pub const FLO_SHAPE_QUADS_OFFSET: usize = 0x08;

/// Bytes per quad. `FUN_140b70200`: `shl r8,0x6` -- the index times sixty-four.
pub const FLO_QUAD_STRIDE: usize = 0x40;
/// `f32` x and `f32` y the quad's source rect is offset by to reach the screen.
pub const FLO_QUAD_X_OFFSET: usize = 0x00;
pub const FLO_QUAD_Y_OFFSET: usize = 0x04;
/// `f32` scale x inside a quad, at the same place a transform block keeps it. `-1` mirrors.
pub const FLO_QUAD_SCALE_X_OFFSET: usize = 0x08;
/// Pointer to the quad's source rect. `FUN_140b70200`: `mov rax,QWORD PTR [r8+0x30]`, and the quad
/// is skipped when it is null.
pub const FLO_QUAD_SOURCE_OFFSET: usize = 0x30;

/// The four colour bytes inside a quad, which is where a shape's colour lives.
///
/// Not the transform block's [`FLO_TRANSFORM_COLOUR_OFFSET`], and one run is what proved the
/// difference. That colour was written into the added hexagon's own copy of the plate's transform
/// block, with both flag bits set, and the log said so -- and the tab drew in the shipped grey.
///
/// `FUN_140b50bc0` is why. It dispatches on the record's kind, and a record naming a shape
/// (`kind & 1`) takes a branch that reaches `FUN_140b51270` -> `FUN_140b70200`, neither of which
/// is handed the record or its transform. A record naming a definition takes `FUN_140b50f20`,
/// which is the path the added rows' icons are tinted on and the reason that tint works. Two kinds
/// of record, two colours, and only one of them lives in a transform block.
///
/// `FUN_140b70200` copies the quad's four bytes into the drawable's per-quad colour array, one
/// quad at a time, and it reverses the first three on the way:
///
/// ```asm
/// mov  al, byte ptr [quad+0x18]   ; -> array[2]
/// mov  al, byte ptr [quad+0x19]   ; -> array[1]
/// mov  al, byte ptr [quad+0x1a]   ; -> array[0]
/// mov  al, byte ptr [quad+0x1b]   ; -> array[3]
/// ```
///
/// So whatever the array's order is, the quad's is that order reversed over its first three bytes
/// with the fourth left where it is. The array's own order has not been read -- its consumer was
/// not chased -- so [`FLO_QUAD_COLOUR_ORDER_IS_BGR`] carries the guess, on its own line, with what
/// a run would say about it.
pub const FLO_QUAD_COLOUR_OFFSET: usize = 0x18;

/// Bytes in a quad's colour. Four, from the four `mov`s above.
pub const FLO_QUAD_COLOUR_SIZE: usize = 4;

/// **Nothing in this crate writes a quad colour, and the byte order above is therefore unsettled.**
/// The reversal is read off the four `mov`s; what the array's consumer expects is not, so which of
/// its bytes is red is not known and nothing here guesses. A previous attempt did guess, and could
/// not be checked: the added hexagon's quad is the layer UNDER the copy `ds2-menu-row`'s `strip`
/// draws over it, so the colour written there was invisible either way. An unverifiable guess in a
/// constant is worse than no constant -- it reads as a measurement to the next person.
///
/// The seventh tab is tinted through [`FLO_TRANSFORM_COLOUR_OFFSET`] on that upper copy instead,
/// whose order a run did settle. Anyone who later needs a shape's own colour starts here, and
/// starts by picking a hue whose channels are far apart so one screenshot answers it.
pub const FLO_QUAD_COLOUR_ORDER_UNSETTLED: () = ();

/// Bytes in a source rect: four floats. `FUN_140b70200` reads `[rax]`, `[rax+4]`, `[rax+8]` and
/// `[rax+0xc]` and copies all four into two sixteen-byte per-quad buffers.
pub const FLO_SOURCE_RECT_SIZE: usize = 0x10;
/// `f32` left, top, right and bottom of a source rect, in atlas pixels.
pub const FLO_SOURCE_LEFT_OFFSET: usize = 0x00;
pub const FLO_SOURCE_TOP_OFFSET: usize = 0x04;
pub const FLO_SOURCE_RIGHT_OFFSET: usize = 0x08;
pub const FLO_SOURCE_BOTTOM_OFFSET: usize = 0x0c;

/// The shape index of the six-hexagon plate. `0x0268`, one quad.
///
/// Child [`FLO_TAB_STRIP_PLATE`] of [`FLO_TAB_STRIP_DEFINITION`], a `kind & 1` record at `(0, 0)`
/// and depth `60` -- below the six cells at `69..89`, which is why a selected tab's highlight draws
/// over its icon.
pub const FLO_TAB_PLATE_SHAPE: u32 = 0x0268;

/// Index of the plate's record inside [`FLO_TAB_STRIP_DEFINITION`]'s child array. Seven.
///
/// The seventh tab's icon record goes in beside it, so the two hexagon rows sit adjacent in array
/// order as well as in depth and neither reading of the draw order can put one over a cell.
pub const FLO_TAB_STRIP_PLATE: usize = 7;

/// Quads the plate carries. One, checked before it is copied: a plate with two is not this plate.
pub const FLO_TAB_PLATE_QUADS: usize = 1;

/// The plate quad's source rect, checked before the copy is made. `(1.10, 781.95)-(337.60, 850.90)`
/// of the 1024x1024 atlas `In-game_01`.
pub const FLO_TAB_PLATE_SOURCE: [f32; 4] = [1.10, 781.95, 337.60, 850.90];

/// The plate quad's screen offset, checked with the rect above. `(0, -775.70)`, which lands that
/// rect at `(1.10, 6.25)-(337.60, 75.20)`.
pub const FLO_TAB_PLATE_OFFSET: [f32; 2] = [0.0, -775.70];

/// The shape index the seventh tab's icon is served under. `0xe268`.
///
/// Same arrangement as [`FLO_ADDED_TAB_SUBTREE_DEFINITION`] and for the same reason: nothing in the
/// shipped document names it, so a lookup for it can only have come from a record this crate wrote.
/// `0xe268` is `0xe000` plus the plate's own index, which keeps the two readable together.
pub const FLO_ADDED_TAB_ICON_SHAPE: u32 = 0xe268;

/// Where the seventh tab's slice starts in the atlas. The plate's right edge less one
/// [`FLO_TAB_PITCH`], which is the last whole tab period the plate holds.
pub const FLO_ADDED_TAB_ICON_SOURCE_LEFT: f32 = FLO_TAB_PLATE_SOURCE[2] - FLO_TAB_PITCH;

/// Depth the icon record is attached at. `62` -- above the plate's `60` and the end cap's `61`, and
/// below the first cell's `69`, so the icon sits on the strip and under its own highlight.
pub const FLO_ADDED_TAB_ICON_DEPTH: u16 = 62;

/// The hue the seventh tab's hexagon is tinted with, at full strength. R, G, B.
///
/// **The same hue the added rows wear**, [`FLO_ADDED_ROW_HUE`], and deliberately not a second
/// number: the tab and the rows inside it are one mod, and the byte order that hue is laid down in
/// cost a run to settle (see [`FLO_ADDED_ROW_TINT`]). Reusing it inherits that measurement instead
/// of betting on it again.
///
/// The seventh tab's art is the sixth tab's, shifted one [`FLO_TAB_PITCH`] -- there are six
/// hexagons in the atlas and this mod ships no texture -- so without a tint the tab this project
/// adds is pixel-identical to the tab beside it. The colour is the only thing that says which one
/// is ours.
pub const FLO_ADDED_TAB_ICON_HUE: [u8; 3] = FLO_ADDED_ROW_HUE;

/// How far the hexagon is pushed from white toward [`FLO_ADDED_TAB_ICON_HUE`], out of `255`.
///
/// **Full strength, where a row's icon is [`FLO_ADDED_ROW_TINT_STRENGTH`] = 120**, and the two
/// differ because they are answering different questions. A row's tint distinguishes two rows that
/// sit one above the other with captions to tell them apart, so a third of a hue is enough and more
/// reads as a re-skin. The tab has no caption and no neighbour to compare against at a glance -- it
/// is one hexagon in a row of seven identical hexagons -- and "a re-skin, a different KIND of row"
/// is exactly the reading wanted here.
///
/// This is taste, not measurement, and it is on its own line for the same reason the row's is: it
/// can be turned without touching the hue or the byte order underneath it.
pub const FLO_ADDED_TAB_ICON_TINT_STRENGTH: u8 = 255;

/// Index, in [`FLO_TAB_STRIP_DEFINITION`]'s child array, of the cap drawn over the strip's right
/// end. Its container is `0x026a` and its quad lands at `(271.05, 5.05)-(349.40, 64.45)`, over the
/// sixth tab -- so a seventh tab needs it one [`FLO_TAB_PITCH`] further along.
pub const FLO_TAB_STRIP_END_CAP: usize = 8;
/// The definition index at that child, checked before its transform is copied.
pub const FLO_TAB_STRIP_END_CAP_DEFINITION: u32 = 0x026a;

/// Index of the `RB` prompt's label, which sits right of the last tab at `x = 347.05` and would
/// otherwise be underneath the seventh tab's hexagon.
pub const FLO_TAB_STRIP_RB_LABEL: usize = 10;
/// The definition index at that child, checked before its transform is copied.
pub const FLO_TAB_STRIP_RB_LABEL_DEFINITION: u32 = 0x026d;

/// The shape index of the two chevrons that flank the strip. `0x026b`, two quads off one mirrored
/// source rect: quad `0` is the right-hand chevron -- its `scale x` is `-1` -- and quad `1` the
/// left.
///
/// Used by exactly one record -- child `9` of the strip -- so serving a moved copy of it moves the
/// right chevron and nothing else in the document.
pub const FLO_TAB_ARROWS_SHAPE: u32 = 0x026b;
/// Quads it carries. Two, checked before either is copied.
pub const FLO_TAB_ARROWS_QUADS: usize = 2;
/// Index of the right-hand chevron inside that shape -- the one a seventh tab displaces.
pub const FLO_TAB_ARROWS_RIGHT: usize = 0;
/// That quad's `scale x`, which is `-1` because it is the left chevron's art mirrored. Checked
/// before the move, because it is what says quad `0` is the right chevron and not the left.
pub const FLO_TAB_ARROWS_RIGHT_SCALE_X: f32 = -1.0;
/// That quad's screen offset x, checked before it is moved. Mirrored art subtracts, so the chevron
/// lands at `1357.45 - (995.75..1020.75)`, which is `336.70..361.70` -- flush against the plate's
/// right edge at `337.60`.
pub const FLO_TAB_ARROWS_RIGHT_X: f32 = 1357.45;

/// That quad's source rect, checked with the two fields above.
///
/// The shape index alone is a weak identity here in a way the plate's is not: this detour sees every
/// shape lookup in every `.flo` the game loads, and `0x026b` in some other document is some other
/// picture. Four more floats that all have to agree is what makes the chevron the chevron.
pub const FLO_TAB_ARROWS_RIGHT_SOURCE: [f32; 4] = [995.75, 724.40, 1020.75, 769.10];

// =================================================================================================
// THE SOFTWARE KEYBOARD, AND THE ONE DWORD THAT KEEPS IT SAFE TO BORROW
//
// DARK SOULS II already asks Steam for a text field -- `SoftwareKeyboardManagerImpl` wraps
// `ISteamUtils::ShowGamepadTextInput` and the game uses it for character naming. A mod can call the
// same API for its own field, and should, because the alternative is drawing and driving a text
// box by hand. But the game's own dismissal listener does NOT check whether it asked for the
// keyboard, so a session this mod opens is a session the game will react to.
//
// EVERY ADDRESS BELOW WAS BYTE-CHECKED AGAINST `darksoulsii-deobf.bin`, not read out of a report.
// The four that matter are quoted with their bytes in their own doc comments.
// =================================================================================================

/// `SoftwareKeyboard::detail::SoftwareKeyboardManagerImpl`'s singleton pointer. RVA `0x01896a08`.
///
/// **Null until the first soft-keyboard interaction**, and that null is a fact worth checking
/// rather than defending against: the game's dismissal listener is registered by the impl's
/// CONSTRUCTOR, so while this pointer is null there is no listener for `GamepadTextInputDismissed_t`
/// anywhere in the game and a mod's own keyboard session cannot disturb anything.
///
/// Established from the accessor `0x140ff1d20`, which both reads and writes it:
///
/// ```text
/// 0x140ff1d4e: 48 8b 0d b3 4c 8a 00   mov rcx,[rip+0x8a4cb3]   ; -> 0x141896a08
/// 0x140ff1dbe: 48 89 05 43 4c 8a 00   mov [rip+0x8a4c43],rax   ; the store after construction
/// ```
///
/// Both displacements resolve to the same address, from a scan of the accessor's whole body rather
/// than from one hit.
pub const SOFTWARE_KEYBOARD_IMPL_SINGLETON: u32 = 0x0189_6a08;

/// `int32 m_state` inside the impl. `+0x08`.
///
/// The constructor at `0x140ff1e28` opens it at idle -- `c7 43 08 ff ff ff ff`,
/// `mov DWORD PTR [rbx+0x8],0xffffffff` -- and it is the only field any of the three parties
/// (the game, Steam's callback, this mod) needs to agree on.
pub const SOFTWARE_KEYBOARD_IMPL_STATE_OFFSET: usize = 0x08;

/// `m_state` when no session is running. `-1`.
///
/// **The value [`SOFTWARE_KEYBOARD_IMPL_SHOW`] demands before it will open anything**, and the
/// value this mod must restore when its own session ends.
pub const SOFTWARE_KEYBOARD_STATE_IDLE: i32 = -1;
/// `m_state` while a Steam keyboard is up. `0`. Written by `show` only after
/// `ShowGamepadTextInput` returned true.
pub const SOFTWARE_KEYBOARD_STATE_SHOWING: i32 = 0;
/// `m_state` after a dismissal the player did not submit. `2`.
pub const SOFTWARE_KEYBOARD_STATE_CANCELLED: i32 = 2;
/// `m_state` after a dismissal the player submitted. `3`.
pub const SOFTWARE_KEYBOARD_STATE_SUBMITTED: i32 = 3;

/// `SoftwareKeyboardManagerImpl::show`. RVA `0x00ff22e0`. **Recorded for its gate, not to be
/// called.**
///
/// ```text
/// 0x140ff2317: 83 79 08 ff   cmp DWORD PTR [rcx+0x8],0xffffffff
/// 0x140ff231b: 74 07         je  proceed
/// 0x140ff231d: 32 c0         xor al,al          ; refuse, and never reach Steam
/// ```
///
/// Those bytes are the whole reason [`SOFTWARE_KEYBOARD_STATE_IDLE`] has to be restored. `show`
/// refuses unless it reads `-1`, and the only writers of `-1` live inside `getResult`, which the
/// game cannot reach once `show` has started failing -- so a state this mod leaves behind is a
/// state the game never recovers from on its own. It is not a crash. It is the Steam keyboard
/// silently never appearing again for the rest of the process.
pub const SOFTWARE_KEYBOARD_IMPL_SHOW: u32 = 0x00ff_22e0;

/// The game's `GamepadTextInputDismissed_t` listener. RVA `0x00ff2040`. Fourteen bytes, no
/// branches, and it is the entire collision risk:
///
/// ```text
/// 33 c0        xor  eax,eax
/// 38 02        cmp  BYTE PTR [rdx],al     ; the callback's m_bSubmitted
/// 0f 95 c0     setne al
/// 83 c0 02     add  eax,0x2               ; -> 2 or 3
/// 89 41 08     mov  DWORD PTR [rcx+0x8],eax
/// c3           ret
/// ```
///
/// **It reads no text and it cannot fail** -- no allocation, no call, no branch, and the only
/// pointers it touches are its own `this` and the callback payload. So a session this mod opens can
/// neither crash the game nor corrupt it.
///
/// **What it does not do is ask whether the game wanted this dismissal.** There is no pending-request
/// flag and no owner check. It is registered exactly once in the image (`0x140ff1e72`, id `714`) and
/// lives in `steam_api64.dll`'s process-wide table, so it fires for a keyboard THIS MOD opened just
/// as readily -- which is why not registering a listener of our own buys nothing, and why the
/// interlock is on `m_state` instead.
pub const SOFTWARE_KEYBOARD_DISMISSED_HANDLER: u32 = 0x00ff_2040;

/// The `ISteamUtils` version whose `ShowGamepadTextInput` takes a prefill. `"SteamUtils007"`.
///
/// DS2's own `steam_api64.dll` carries only `SteamUtils005` and `SteamClient012`, and at
/// `0x140ff2415` the game loads four arguments and never writes `[rsp+0x28]` -- the
/// `pchExistingText` slot. **That is the GAME's call site, not a limit of the API.**
///
/// Read off Proton Experimental's generated per-version wrappers in
/// `files/lib/wine/x86_64-unix/lsteamclient.so`, which is the closest thing to the Steamworks
/// headers available offline. `005` and `006` marshal four arguments; from `007` on there is a
/// fifth, `mov 0x1d(%rbx),%r9` -- a full 64-bit pointer:
///
/// ```text
/// 005, 006          4 args   slot 0xa0
/// 007, 008, 009, 010  5 args   slot 0xa0     <- prefill arrives here
/// 011                 5 args   slot 0x98     <- and the slot moves
/// ```
///
/// `007` is therefore the SMALLEST upgrade that gains the prefill, and the one that keeps
/// [`STEAM_UTILS_SHOW_GAMEPAD_TEXT_INPUT_SLOT`] where the game's own build already proves it to be.
/// `~/.local/share/Steam/linux64/steamclient.so` advertises `SteamUtils001` through `011`, so the
/// client vends it.
///
/// **NEVER HAND THE NEWER POINTER TO THE GAME.** `007` keeps the method at the same slot as `005`,
/// so the game's four-argument call would pass straight through it leaving `r9` holding whatever
/// was there -- a garbage `pchExistingText`. Two interfaces, two owners.
pub const STEAM_UTILS_VERSION_WITH_PREFILL: &str = "SteamUtils007";

/// `ISteamClient012::GetISteamUtils(HSteamPipe, const char *pchVersion)`. Vtable slot `+0x48`.
///
/// From Proton's `ISteamClient_SteamClient012_GetISteamUtils` thunk, which is
/// `mov 0x14(%rbx),%rdx` (the version string), `mov 0x10(%rbx),%esi` (the pipe), `call *0x48(%rax)`.
/// The shim does not validate the version string -- `steamclient64.dll` resolves it -- which is what
/// makes [`STEAM_UTILS_VERSION_WITH_PREFILL`] reachable from a binary that only knows `005`.
pub const STEAM_CLIENT_GET_ISTEAM_UTILS_SLOT: usize = 0x48;

/// `ISteamUtils::IsOverlayEnabled()`. Vtable slot `+0x88`.
///
/// Read out of DS2 itself: [`SOFTWARE_KEYBOARD_IMPL_SHOW`] gates on it before it will ask for a
/// keyboard. **It gates this mod's field too, and it is the one thing about this path that no
/// amount of static reading settles** -- whether the overlay is actually attached is a property of
/// the running Steam client, not of the executable.
pub const STEAM_UTILS_IS_OVERLAY_ENABLED_SLOT: usize = 0x88;

/// `ISteamUtils::ShowGamepadTextInput(...)`. Vtable slot `+0xa0`.
///
/// Observed twice, independently: DS2 calls `[vtbl+0xa0]` at `0x140ff2424` on its `005` pointer, and
/// Proton's `007` wrapper calls `*0xa0(%rax)`. Same slot on both sides of the version bump, which is
/// what makes `007` a drop-in for the game's own layout.
pub const STEAM_UTILS_SHOW_GAMEPAD_TEXT_INPUT_SLOT: usize = 0xa0;

/// `ISteamUtils::GetEnteredGamepadTextLength()`. Vtable slot `+0xa8`. From
/// `SoftwareKeyboardManagerImpl::getResult` (`0x00ff2050`), at `0x140ff20b1`.
pub const STEAM_UTILS_GET_ENTERED_TEXT_LENGTH_SLOT: usize = 0xa8;

/// `ISteamUtils::GetEnteredGamepadTextInput(char *pchText, uint32 cchText)`. Vtable slot `+0xb0`.
/// Same function, at `0x140ff2101`. **The text comes back UTF-8**, which is why the impl keeps a
/// `char*` scratch buffer at `+0x10` rather than a wide string.
pub const STEAM_UTILS_GET_ENTERED_TEXT_SLOT: usize = 0xb0;

/// `FeGroupInGameTopSelect::v2` -- the pause-menu group's per-frame update. RVA `0x000a5dd0`.
///
/// **The clock a row needs to report anything.** Captions are otherwise written once, when
/// `FE_INGAME_TOP_SELECT_CAPTIONS` binds them, so a row that learns something while the menu is
/// open could not say so until the menu was closed and opened again.
///
/// How it was established, since "vtable slot 2" is only a number. Both base classes --
/// `FexGroupList<FeGroupGrid>` (`0x1410adf18`) and `FeGroupInGameGroupSelect` (`0x1410b6658`) --
/// hold `0x14001bdd0` in this slot, which is `add rcx,0x58; jmp 0x1400271f0`, and `0x1400271f0`
/// takes a float in XMM1 and forwards it -- the frame-delta signature
/// [`FE_TOP_MENU_UPDATE`] already documents. `FeGroupInGameTopSelect` (vtable `0x1410b67f8`)
/// OVERRIDES it with this, which takes the delta in XMM1, polls the cursor through `0x1400a65f0`,
/// and on a confirm calls [`FE_INGAME_MENU_DISPATCH`] at `0x1400a5e5f`. A function that reads input
/// and dispatches the confirm runs once a frame by construction.
///
/// Signature: `(this /*rcx*/, f32 delta /*xmm1*/)`.
pub const FE_INGAME_TOP_SELECT_UPDATE: u32 = 0x000a_5dd0;

/// The bytes [`FE_INGAME_TOP_SELECT_UPDATE`] must begin with, or the detour refuses to patch.
///
/// `48 89 5c 24 20` is `mov QWORD PTR [rsp+0x20],rbx` -- **exactly five bytes**, so MinHook
/// relocates one whole instruction and never splits one. Read out of the image, and
/// `scripts/ds2-arxan-chain.py 0x1400a5dd0` terminates at hop 0: its own prologue is at its own
/// entry, so the detour never touches Arxan's code.
pub const FE_INGAME_TOP_SELECT_UPDATE_PROLOGUE: [u8; 5] = [0x48, 0x89, 0x5c, 0x24, 0x20];

/// The game's top-level `HWND`, inside [`FE_SYSTEM_SINGLETON`]'s target. `+0x08`.
///
/// Read out of the only two functions that use it -- the game's own clipboard paste
/// (`0x00b4ac70`) and copy (`0x00b4b860`). Both do the same three instructions:
///
/// ```text
/// 0x140b4ac76: 48 8b 05 7b a5 b2 00   mov rax,[rip+0xb2a57b]   ; -> 0x1416751f8
/// 0x140b4ac80: 48 8b 48 08            mov rcx,[rax+0x8]
/// 0x140b4ac84: ff 15 ...              call [OpenClipboard]
/// ```
///
/// `OpenClipboard` takes an `HWND` and nothing else, so the field's TYPE is settled by its only
/// use rather than by guessing at a struct layout -- two independent sites, one meaning.
pub const FE_SYSTEM_HWND_OFFSET: usize = 0x08;

// =================================================================================================
// THE LIVE CHARACTER
//
// `GameManagerImp` -> `GameDataManager` -> `player_data`. Everything here was read out of ONE
// function -- `FeSubStateTitleLoadProfile`'s work starter at `0x1400fc2a2`..`0x1400fc330` -- which
// is the code that populates the block when a save is loaded, so the offsets and their MEANINGS
// come from the same place rather than from a struct dump plus a guess.
//
// WHAT IS NOT HERE: stats, soul level, soul memory, equipment, inventory. None of them is mapped in
// this repo. `PlayerGameData` and `GameDataPlayerInfo` exist as RTTI only inside `FeFunctorJob<...>`
// template names -- non-polymorphic classes with no vtable of their own, so there is no RTTI
// shortcut to them, and reaching stats means a separate excavation from `PlayerCtrl`.
// =================================================================================================

/// `GameManagerImp` -> the object holding the live character block. `+0xC0`.
///
/// `mov rdi,[rcx+0xc0]` at `0x1400fc2b7` (`48 8b b9 c0 00 00 00`), where `rcx` is the
/// `GameDataManager` fetched one instruction earlier by `mov rcx,[rax+0xa8]` at `0x1400fc2a2`
/// (`48 8b 88 a8 00 00 00`) -- that is [`GAME_DATA_MANAGER_OFFSET`], already recorded.
///
/// **Beware the neighbour.** [`GAME_MANAGER_CONTENT_CTX_OFFSET`] is also `0xC0`, but it hangs off
/// `GameManagerImp` and this one hangs off `GameDataManager` -- same number, different object, one
/// hop apart. Confusing them gets a pointer that reads plausibly and means nothing.
pub const GAME_DATA_PLAYER_DATA_OFFSET: usize = 0xC0;

/// The live character's name, as `wchar_t[0x20]`. `player_data + 0x24`.
///
/// From the copy the profile loader performs: `lea rcx,[rdi+0x24]` / `mov r8d,0x20` at
/// `0x1400fc302` (`48 8d 4f 24 41 b8 20 00 00 00`) into `wcsncpy`, taking the name out of the save
/// slot record.
///
/// **The clear site is known too**, which is what makes this usable as a liveness test rather than
/// just a label: `FeSubStateTitleDeleteDataList`'s leave writes `L""` over it at `0x1400fb822`. A
/// first unit of `0` therefore means "no character", not merely "not written yet".
pub const PLAYER_DATA_NAME_OFFSET: usize = 0x24;

/// UTF-16 units in the name field, from the `mov r8d,0x20` the copy is bounded by.
pub const PLAYER_DATA_NAME_UNITS: usize = 0x20;

/// The NEW GAME cycle -- Journey 1, Journey 2, and so on. `player_data + 0x68`, `u32`, **1-based**.
///
/// `mov DWORD PTR [rdi+0x68],1` at `0x1400fc318` (`c7 47 68 01 00 00 00`), immediately after the
/// name copy in the profile loader.
///
/// **THIS WAS RECORDED HERE AS A "PROFILE LOADED" FLAG AND THAT WAS WRONG.** The reading came from
/// seeing one literal `1` written on load and finding nothing that writes it back to `0`, which
/// made it look like a sticky boolean. Both community Cheat Engine tables identify the same field
/// independently, with a dropdown reading `1:Journey 1` through `9:Journey 9` -- so the literal `1`
/// is "New Game", not "true", and nothing writes zero because a journey counter only ever goes up.
///
/// One write site is not a type. A field whose only observed value is `1` is as consistent with a
/// counter as with a flag, and the difference only shows on a character who has been through NG+.
///
/// It is still usable as a liveness hint -- it is `0` before any profile is loaded -- but
/// [`PLAYER_DATA_NAME_OFFSET`] is the better test, and this must never be WRITTEN as a flag.
pub const PLAYER_DATA_JOURNEY_OFFSET: usize = 0x68;

// =================================================================================================
// THE LIVE STAT BLOCK -- `PlayerParam`
//
// `GameManagerImp -> PlayerCtrl (+0xD0) -> PlayerParam (+0x490)`. The chain is VERIFIED in the
// image; the FIELD OFFSETS inside `PlayerParam` are not -- they come from two community Cheat
// Engine tables that agree with each other, and each one says so in its own doc comment.
//
// Corroboration for the shape, from a completely unrelated direction: `scripts/ds2-sl2.py` found
// ELEVEN `i16` at save-slot-record `+0x188` by inspecting real saves ("the nine DS2 stats plus
// two"), and the tables describe eleven contiguous `u16` at `PlayerParam+0x08`. Two sources that
// never saw each other, same eleven-short block.
// =================================================================================================

/// A null-guarded getter for `PlayerParam`. RVA `0x001ab660`. **Call this rather than walking.**
///
/// The whole function, verified byte for byte:
///
/// ```text
/// 48 8b 05 89 92 46 01   mov rax,[rip+0x1469289]   ; -> 0x1416148f0, GAME_MANAGER_IMP
/// 48 85 c0               test rax,rax
/// 74 07                  je  +7
/// 48 8b 80 d0 00 00 00   mov rax,[rax+0xd0]        ; PlayerCtrl
/// 48 85 c0               test rax,rax
/// 74 08                  je  +8
/// 48 8b 80 90 04 00 00   mov rax,[rax+0x490]       ; PlayerParam
/// c3                     ret
/// ```
///
/// Takes nothing, returns the pointer in `rax`, and returns NULL at whichever hop is null rather
/// than faulting -- which is the case on the title screen, where `PlayerCtrl` is null. Using the
/// game's own accessor costs one call and removes two chances to get a null check wrong.
pub const PLAYER_PARAM_GET: u32 = 0x001a_b660;

/// `GameManagerImp -> PlayerCtrl`. `+0xD0`. From [`PLAYER_PARAM_GET`]'s first hop.
pub const PLAYER_CTRL_OFFSET: usize = 0xD0;
/// `PlayerCtrl -> PlayerParam`. `+0x490`. From [`PLAYER_PARAM_GET`]'s second hop.
pub const PLAYER_PARAM_OFFSET: usize = 0x490;

/// The nine levelled stats inside `PlayerParam`, each a `u16`.
///
/// **THE MEMORY ORDER IS NOT THE PLANNER'S ORDER.** soulsplanner emits vigor, endurance, vitality,
/// attunement, strength, dexterity, *adaptability*, intelligence, faith. The game stores
/// adaptability LAST, after intelligence and faith. Zipping one order onto the other silently swaps
/// three stats, and the result is a character that levelled up correctly into the wrong attributes.
///
/// | offset | stat |
/// |---|---|
/// | `+0x08` | vigor |
/// | `+0x0A` | endurance |
/// | `+0x0C` | vitality |
/// | `+0x0E` | attunement |
/// | `+0x10` | strength |
/// | `+0x12` | dexterity |
/// | `+0x14` | intelligence |
/// | `+0x16` | faith |
/// | `+0x18` | adaptability |
///
/// Two further `u16` follow at `+0x1A` and `+0x1C` that both tables leave unnamed -- the eleven
/// shorts `scripts/ds2-sl2.py` also sees on disk.
///
/// **Source: the Cheat Engine tables, not the executable.** Both agree offset for offset, and the
/// eleven-short shape is corroborated from the save file, but no write site was found in the image.
pub const PLAYER_PARAM_STAT_OFFSETS: [usize; 9] =
    [0x08, 0x0A, 0x0C, 0x0E, 0x10, 0x12, 0x14, 0x16, 0x18];

/// Stat names in the order [`PLAYER_PARAM_STAT_OFFSETS`] lists them -- the GAME's order.
pub const PLAYER_PARAM_STAT_NAMES: [&str; 9] = [
    "vigor",
    "endurance",
    "vitality",
    "attunement",
    "strength",
    "dexterity",
    "intelligence",
    "faith",
    "adaptability",
];

/// Soul level. `PlayerParam + 0xD0`, `u32`. Table-sourced.
pub const PLAYER_PARAM_SOUL_LEVEL_OFFSET: usize = 0xD0;

/// Souls currently held. `PlayerParam + 0xEC`, `u32`. Table-sourced, and the LEAST certain constant
/// here: one table labels it "Total Get Soul" and the other labels the same offset "Soul" while
/// separately naming `+0xF4`/`+0xFC` as the totals. The three-record reading (`{u32 value; u8 flag;
/// pad}` at `0xEC`, `0xF4`, `0xFC`) is self-consistent and makes this the spendable balance.
/// Confirm by watching it fall when souls are spent.
pub const PLAYER_PARAM_SOULS_HELD_OFFSET: usize = 0xEC;

/// Soul memory. **TWO fields, and both must be written.** `PlayerParam + 0xF4` and `+ 0xFC`, `u32`.
///
/// The tables call them `TotalGetSoul1` and `TotalGetSoul2`. Writing one and not the other leaves
/// the game free to resynchronise from whichever it trusts, which would silently undo the write --
/// so this is an array rather than a single constant, to make forgetting the second one awkward.
pub const PLAYER_PARAM_SOUL_MEMORY_OFFSETS: [usize; 2] = [0xF4, 0xFC];

/// Covenant. `PlayerParam + 0x1AD`, one byte. See [`COVENANT_NAMES`] for the ids.
///
/// **Confirmed from the binary**, no longer table-sourced only: `SetCovenant` (`0x14038bd80`)
/// writes this byte and the Rat-King branch in `0x140202f10` tests the id against `5`.
pub const PLAYER_PARAM_COVENANT_OFFSET: usize = 0x1AD;

/// The covenant ids, indexed by id. `0` is "no covenant".
///
/// Confirmed two ways: the per-covenant discovered flags at
/// [`PLAYER_PARAM_COVENANT_DISCOVERED_BASE`] line up with the nine the community table names, and
/// the binary's own Rat-King test is against id `5`.
pub const COVENANT_NAMES: [&str; 10] = [
    "No Covenant",
    "Heirs of the Sun",
    "Blue Sentinels",
    "Brotherhood of Blood",
    "Way of Blue",
    "Rat King",
    "Bell Keepers",
    "Dragon Remnants",
    "Company of Champions",
    "Pilgrims of Dark",
];

/// **The per-covenant "discovered" flags, and joining one CANNOT BE UNDONE.** `PlayerParam + 0x1AE`.
///
/// One `u8` per covenant, indexed by id, so covenant `n`'s flag is at `0x1AE + n`.
///
/// `SetCovenant` sets `p[0x1AE + id] = 1` alongside the current-covenant byte, and **nothing in
/// that function ever clears it**. Leaving a covenant changes [`PLAYER_PARAM_COVENANT_OFFSET`] and
/// leaves this set. So joining a covenant to satisfy a build permanently marks that covenant as
/// discovered on that character -- the same shelf as raising soul memory, and worth saying out loud
/// before anyone presses a button.
///
/// Cross-checked offset for offset against the community table's nine named flags: `+0x1AF` Heirs
/// of the Sun, `+0x1B0` Blue Sentinels, ... `+0x1B7` Pilgrims of Dark.
///
/// Rank (`+0x1B9..`) and progress (`+0x1C4..`) are NOT touched by the setter.
pub const PLAYER_PARAM_COVENANT_DISCOVERED_BASE: usize = 0x1AE;

/// **Join a covenant, through the path that maintains everything a covenant change implies.**
/// RVA `0x00202f10`.
///
/// `fn(PlayerCtrl*, i32 covenant_id, bool announce)`.
///
/// Prefer this over `PlayerParam::SetCovenant` (`0x0038bd80`) directly: this one also maintains the
/// Rat-King flag (`[PlayerCtrl + 0xB0] + 0x4E = (id == 5)`) and runs the refresh that the bare
/// setter does not.
///
/// # `announce` MUST be false for an import
///
/// A true `announce` calls the server sync and, in an online session, sends a covenant-change
/// packet to everyone in it. A build import is not a covenant the player walked up to and joined,
/// and it has no business telling the session so.
///
/// # It still cannot be undone
///
/// See [`PLAYER_PARAM_COVENANT_DISCOVERED_BASE`]. `announce = false` suppresses the network, not
/// the permanent flag.
///
/// Not Arxan-redirected.
pub const PLAYER_CTRL_SET_COVENANT: u32 = 0x0020_2f10;

/// The five bytes [`PLAYER_CTRL_SET_COVENANT`] must begin with. `test rcx,rcx; jz ...`.
pub const PLAYER_CTRL_SET_COVENANT_PROLOGUE: [u8; 5] = [0x48, 0x85, 0xc9, 0x74, 0x42];

/// Starting class. `player_data + 0x64`, and it is a **`u32`, not the byte the tables declare**.
///
/// The loader writes it as a dword -- `movzx eax,WORD PTR [rbx+0x1d6]` then
/// `mov DWORD PTR [rdi+0x64],eax` at `0x1400fc311`/`0x1400fc31f` -- widening a `u16` out of the
/// save slot record. Both tables type it `Byte`, which reads correctly on a little-endian machine
/// for every value under 256 and would corrupt the upper three bytes on a write.
///
/// `1` Warrior, `2` Knight, `4` Bandit, `6` Cleric, `7` Sorcerer, `8` Explorer, `9` Swordsman,
/// `10` Deprived. The gaps are the game's.
pub const PLAYER_DATA_CLASS_OFFSET: usize = 0x64;

// =================================================================================================
// THE PARAM TABLES, AT RUNTIME
//
// DARK SOULS II keeps its balance data in `param:/<Name>.param` tables inside the encrypted
// `enc_regulation.bnd.dcx`. Decrypting that file is one way to read them; walking the tables the
// GAME HAS ALREADY LOADED is a better one, and not only because it skips the decryption:
//
//   * it cannot drift from the build actually running, and
//   * it reflects any param mod the player has installed, which a shipped copy of the vanilla
//     numbers would silently contradict.
//
// EVERY CONSTANT BELOW IS TABLE-SOURCED. They come from the community Cheat Engine tables'
// `ParamUtils`, which has been walking these structures for years, and no write site was traced in
// the image. The shapes are self-consistent and the row layout is corroborated across two tables,
// but this is a weaker provenance than the rest of this file and it says so.
// =================================================================================================

/// The hops from `GAME_MANAGER_IMP` to the pointer the master param table is measured against.
///
/// `[[[GameManagerImp] + 0x38] + 0x100] + 0xD8`, then the two constants below are subtracted from
/// what that yields. The subtraction is what makes this table-sourced rather than derived: it is a
/// measured relationship, not a field offset, and nothing in the image was found that explains it.
pub const PARAM_ANCHOR_OFFSETS: [usize; 3] = [0x38, 0x100, 0xD8];

/// The master param table starts this far BELOW the anchor. Subtracted, not added.
pub const PARAM_TABLE_FROM_ANCHOR: usize = 0x2C44;
/// The param index ends this far below the anchor.
pub const PARAM_INDEX_END_FROM_ANCHOR: usize = 0x16A4;
/// The index array begins this far into the master table.
pub const PARAM_INDEX_START_OFFSET: usize = 0x40;
/// Bytes per index entry.
pub const PARAM_INDEX_STRIDE: usize = 0x18;
/// `i32` offset of a param's data, relative to the master table.
pub const PARAM_INDEX_DATA_OFFSET: usize = 0x10;
/// `i32` offset of a param's name string, relative to the master table.
pub const PARAM_INDEX_NAME_OFFSET: usize = 0x14;

/// `u16` row count, inside one param table.
pub const PARAM_ROW_COUNT_OFFSET: usize = 0x0A;
/// Where a param's own row index begins.
pub const PARAM_ROW_INDEX_OFFSET: usize = 0x40;
/// Bytes per row-index entry.
pub const PARAM_ROW_STRIDE: usize = 0x18;
/// `u32` row id, inside a row-index entry.
pub const PARAM_ROW_ID_OFFSET: usize = 0x00;
/// `u32` offset of the row's data, relative to the param table.
pub const PARAM_ROW_DATA_OFFSET: usize = 0x08;

/// The param naming the souls each level costs. 852 rows, stride 12.
///
/// **Row id is the level being LEFT**, and that direction is verified from the game's own level-up
/// code in both directions: the increment (`FUN_1401fb970`) pays the value stored for the level it
/// is leaving, and the decrement (`FUN_1401fb800`) steps down from `L` and refunds `lookup(L-1)`,
/// which is only correct under this reading. One level's error here is the whole bug.
///
/// **Not `LevelUpStatusCalcParam`**, which has the better name and is a nine-row menu table.
pub const PARAM_PLAYER_LEVEL_UP_SOULS: &str = "PlayerLevelUpSoulsParam";

/// Bytes per `PlayerLevelUpSoulsParam` row. `12` -- measured from consecutive row-index offsets
/// rather than assumed, by `scripts/ds2-regulation.py`.
pub const PLAYER_LEVEL_UP_SOULS_STRIDE: usize = 12;

/// `u32` level, at the start of a `PlayerLevelUpSoulsParam` row. Mirrors the row id.
pub const PLAYER_LEVEL_UP_SOULS_LEVEL_OFFSET: usize = 0x00;
/// `u32` souls to go from this row's level to the next. `+0x08`.
///
/// The row is `{u16 level; u16 pad; i32 gradient; i32 souls}`. `gradient` is `0` in every shipped
/// row; where it is not, the game interpolates `(wanted - rowLevel) * gradient + souls`, so a table
/// with a nonzero gradient is not a per-level list and must not be read as one.
pub const PLAYER_LEVEL_UP_SOULS_COST_OFFSET: usize = 0x08;

// =================================================================================================
// GRANTING AN ITEM -- the game's own function
//
// The mod does NOT build inventory slots. A slot the game did not build is a slot whose invariants
// nobody maintained: the linked-list links, the handle at `+0x1C` that the discard path consumes,
// the equip category, and the counters the UI reads. This calls what the game calls.
// =================================================================================================

/// `ItemGive`. RVA `0x001ac3d0`. `fn(inventory, *const ItemSpawn, count) -> bool`.
///
/// A three-instruction thunk, verified byte for byte:
///
/// ```text
/// 48 8b 49 10       mov  rcx,[rcx+0x10]     ; forward the inner bag manager
/// 45 33 c9          xor  r9d,r9d            ; 4th argument forced to 0
/// e9 94 b0 ff ff    jmp  0x1401a7470        ; the implementation
/// ```
///
/// **It returns a bool in `AL` and `false` means nothing was granted.** Neither community
/// implementation checks it, which is part of why their failures are quiet.
///
/// **It depends on nothing but its arguments.** Both community implementations call it from a
/// FRESH REMOTE THREAD (`createthread`, `executeCodeEx`) -- a thread with no registers, no frame and
/// no relationship to any game state -- and that has worked for years. So a caller on the game's own
/// thread, from a pause-menu confirm, is strictly better placed than the code this was learned from.
///
/// Not Arxan-redirected; the first byte is `0x48`.
pub const ITEM_GIVE: u32 = 0x001a_c3d0;

/// The five bytes [`ITEM_GIVE`] must begin with, or the mod refuses to call it.
pub const ITEM_GIVE_PROLOGUE: [u8; 5] = [0x48, 0x8b, 0x49, 0x10, 0x45];

/// The implementation [`ITEM_GIVE`] tail-jumps to. RVA `0x001a7470`.
///
/// Recorded because it exposes the fourth argument the thunk forces to zero -- INFERRED to suppress
/// the pickup notification, from `mov r14d,r9d` and a later `test r14d,r14d / jne` that skips the
/// notify block. Call this instead of the thunk only if a silent grant is wanted, and note it takes
/// the INNER bag manager rather than the inventory.
pub const ITEM_GIVE_IMPL: u32 = 0x001a_7470;

/// Most items one [`ITEM_GIVE`] call accepts. `32`.
///
/// From the implementation's own gate at `0x1401a748e`: `lea eax,[rsi-1]; cmp eax,0x1f; ja` --
/// verified bytes `8d 46 ff 83 f8 1f 0f 87`. **The community scripts cap at 8**, bounded by their
/// own buffer rather than by the engine, and 8 is therefore the only width anyone has exercised.
pub const ITEM_GIVE_MAX_PER_CALL: usize = 32;

/// **The game's own reason for refusing a grant.** `inner + 0x10138`, `u32`.
///
/// `inner` is one hop of `+0x10` from `ItemInventory2` -- the object the grant thunk's
/// `mov rcx,[rcx+0x10]` produces, and the FIRST of the two hops that reach
/// [`ITEM_BAG_LIST_OFFSET`].
///
/// # Read this instead of guessing
///
/// [`ITEM_GIVE`] answers `bool`, and its whole `false` path is one instruction pair in
/// `ItemInventory2::CanGive`: `test dword [inner+0x10138], 0x80000000; sete al`. **Bit 31 means
/// refused and the rest of the word names WHY.** Three items once looked like a mystery worth a
/// disassembly session; the answer was four readable bytes in the running game the whole time.
///
/// Non-fatal bits also appear here: `0x4` category 11, `0x2` the quantity was clamped, `0x1`
/// accepted.
pub const ITEM_GIVE_ERROR_OFFSET: usize = 0x1_0138;

/// A number qualifying [`ITEM_GIVE_ERROR_OFFSET`]. `inner + 0x1013c`, `u32`.
pub const ITEM_GIVE_ERROR_DETAIL_OFFSET: usize = 0x1_013C;

/// The bit in [`ITEM_GIVE_ERROR_OFFSET`] that means the grant was refused.
pub const ITEM_GIVE_ERROR_REFUSED: u32 = 0x8000_0000;

/// Every refusal code, with what it means, in the order the implementation can set them.
///
/// # Four of these mean "the character already has it", which is NOT a failure
///
/// `0x800A0000`, `0x800C0000`, `0x80010000` and `0x81000000` all say the build's intent is already
/// satisfied -- the stack is full, or the item is unique and already held. A mule save that reports
/// `granted 15/18` for that reason looks broken and is not. See
/// [`ITEM_GIVE_ERROR_ALREADY_SATISFIED`].
pub const ITEM_GIVE_ERRORS: [(u32, &str); 13] = [
    (0x8400_0000, "the count was zero or negative"),
    (0x8800_0000, "more than one call may carry"),
    (0xC000_0000, "no such row in ItemParam"),
    (0xA000_0000, "the item's sub-param row is missing"),
    (0x9000_0000, "the item is banned from being granted"),
    (0x8001_0000, "already held, and it is not stackable"),
    (
        0x8040_0000,
        "already held, and the call asked to fail if so",
    ),
    (0x8080_0000, "the bag has no free slot"),
    (0x800A_0000, "the stack is already full"),
    (0x800C_0000, "the existing stack is already full"),
    (0x8100_0000, "not stackable, and a stack already exists"),
    (0x8000_0000, "refused, with no reason bits set"),
    (0x0000_0001, "accepted"),
];

/// The codes that mean the character ALREADY HAS what the build asked for.
///
/// Reported as satisfied rather than failed. The distinction is the difference between a mod that
/// says "I could not give you a Poison Moss" and one that says "you already have ninety-nine".
pub const ITEM_GIVE_ERROR_ALREADY_SATISFIED: [u32; 4] =
    [0x800A_0000, 0x800C_0000, 0x8001_0000, 0x8100_0000];

/// `GameDataManager -> ItemInventory2`. `+0x10`, reached from [`GAME_DATA_MANAGER_OFFSET`].
///
/// The full chain is `[[[GAME_MANAGER_IMP] + 0xA8] + 0x10]`, and every hop must be null-checked --
/// the community scripts' `vortmov` macro emits a `test/jz` after each dereference and bails, which
/// is the behaviour to copy rather than the shortcut to skip.
pub const ITEM_INVENTORY_OFFSET: usize = 0x10;

/// **Equip anything: weapons, armour, rings, ammo, hotbar items AND spells.** RVA `0x001ac510`.
///
/// `fn(ItemInventory2*, u32 internal_slot, *const ItemEntry)`. A NULL entry unequips the slot.
///
/// # DARK SOULS II has ONE equip function, not five
///
/// `ItemInventory2::SetEquip` is the direct sibling of [`ITEM_GIVE`] -- 0x140 bytes away, the same
/// `48 8b 49 10` thunk shape, and it takes THE SAME RECEIVER, so nothing new has to be resolved to
/// call it. Attuning a spell and holstering a flask go through it exactly like equipping a sword;
/// only gestures branch elsewhere. That is a different shape from ELDEN RING, where the equip and
/// the quick-slot writes are separate paths.
///
/// One call updates the inventory AND pushes the change into the character: the implementation
/// reaches `PlayerCtrl` and notifies per category. There is no separate refresh to remember.
///
/// # It is a MOVE, not a toggle
///
/// The implementation strips the item from wherever it currently sits before writing the new slot,
/// so equipping something into the slot it already occupies leaves it equipped. **ELDEN RING
/// toggles it off**, which is why `er-build-import` has to query the slot first; DS2 does not need
/// that check. Equipping one entry into two slots still strips the first -- one entry, one slot.
///
/// **The exception is spells**: unequipping a spell slot compacts the attunement array downward and
/// re-notifies every shifted slot, so re-attuning is not a clean no-op. Attune ascending, once each.
///
/// # It fails SILENTLY, twice
///
/// A category mismatch (an item in a slot its type does not fit) returns having done nothing, with
/// no error. So does attuning past the character's capacity, which *unequips* the slot instead. Both
/// are why [`ITEM_INVENTORY_EQUIPPED_BY_FLAT_SLOT`] exists: every equip must be read back.
///
/// Not Arxan-redirected -- its first byte is `0x48`, and the `e9` at `+4` targets `0x1401a7b80`,
/// inside the game's own `.text` rather than Arxan's `0x141aaf000..0x141d42fff`.
pub const ITEM_SET_EQUIP: u32 = 0x001a_c510;

/// The five bytes [`ITEM_SET_EQUIP`] must begin with. `mov rcx,[rcx+0x10]`, then the tail-jump.
pub const ITEM_SET_EQUIP_PROLOGUE: [u8; 5] = [0x48, 0x8b, 0x49, 0x10, 0xe9];

/// Read back what is equipped in a FLAT slot. RVA `0x001abc00`.
///
/// `fn(ItemInventory2*, i32 flat_slot) -> *const ItemEntry`, null when the slot is empty. Read the
/// item id at [`ITEM_ENTRY_ITEM_ID_OFFSET`].
///
/// **Takes the FLAT index, where [`ITEM_SET_EQUIP`] takes the INTERNAL one.** The two spaces are not
/// the same and [`ITEM_SLOT_FLAT_TO_INTERNAL`] is the map between them.
pub const ITEM_INVENTORY_EQUIPPED_BY_FLAT_SLOT: u32 = 0x001a_bc00;

/// The five bytes [`ITEM_INVENTORY_EQUIPPED_BY_FLAT_SLOT`] must begin with. `sub rsp,0x28`.
pub const ITEM_INVENTORY_EQUIPPED_BY_FLAT_SLOT_PROLOGUE: [u8; 5] = [0x48, 0x83, 0xec, 0x28, 0x48];

/// The entry for a `u16` inventory handle. RVA `0x001abfb0`.
///
/// `fn(ItemInventory2*, u16 handle) -> *const ItemEntry`.
pub const ITEM_INVENTORY_ENTRY_BY_HANDLE: u32 = 0x001a_bfb0;

/// The five bytes [`ITEM_INVENTORY_ENTRY_BY_HANDLE`] must begin with.
pub const ITEM_INVENTORY_ENTRY_BY_HANDLE_PROLOGUE: [u8; 5] = [0x48, 0x8b, 0x49, 0x10, 0xe9];

/// **`ItemInventory2 -> BagList` is TWO hops of `+0x10`, not one.**
///
/// The full chain is `[[ItemInventory2 + 0x10] + 0x10]`, and getting it wrong costs nothing
/// visible: the scan reads a live object that simply is not the bag, finds no entry matching any
/// item, and reports every equip as a missing item. That is exactly what the first in-game run of
/// the equip path did -- `equipped 0/18`, with a reason that pointed at the player chain.
///
/// It is two hops because the work is split across the thunk and its implementation, and reading
/// only the thunk shows one:
///
/// ```text
/// 0x1401ac510  mov rcx,[rcx+0x10]   ; SetEquip's thunk: inventory -> the manager
///              jmp 0x1401a7b80
/// 0x1401a7b80  cmp edx,0x29
///              ja  gestures
///              mov rcx,[rcx+0x10]   ; and again: the manager -> the bag
/// ```
///
/// So this constant is applied TWICE. It is one number and two dereferences, and the doc says so
/// because the number being the same is what made one of them easy to miss.
pub const ITEM_BAG_LIST_OFFSET: usize = 0x10;

/// How many `+0x10` hops separate `ItemInventory2` from its `BagList`. **Two.**
///
/// A named count rather than a repeated `hop(hop(x))`, so the reason there are two is attached to
/// the number. See [`ITEM_BAG_LIST_OFFSET`].
pub const ITEM_BAG_LIST_HOPS: usize = 2;

/// Where the inventory entry array starts inside the bag. `+0x28`.
pub const ITEM_ENTRY_ARRAY_OFFSET: usize = 0x28;

/// Bytes per inventory entry. `0x28`.
pub const ITEM_ENTRY_STRIDE: usize = 0x28;

/// How many entries the array holds. `3840`, from the constructor's `0xeff` countdown.
pub const ITEM_ENTRY_COUNT: usize = 3840;

/// `ItemEntry + 0x14`, `u32`. The `ItemParam` row id -- the same number [`ITEM_GIVE`] takes.
pub const ITEM_ENTRY_ITEM_ID_OFFSET: usize = 0x14;

/// `ItemEntry + 0x1C`, `u16`. The entry's own handle; `0xFFFF` means none.
pub const ITEM_ENTRY_HANDLE_OFFSET: usize = 0x1C;

/// `ItemEntry + 0x1F`, `u8`. Bit `0x02` is set while the entry is equipped somewhere.
pub const ITEM_ENTRY_FLAGS_OFFSET: usize = 0x1F;

/// The bit in [`ITEM_ENTRY_FLAGS_OFFSET`] meaning "equipped".
pub const ITEM_ENTRY_FLAG_EQUIPPED: u8 = 0x02;

/// `ItemEntry + 0x20`, `u16`. How many are in the stack.
pub const ITEM_ENTRY_QUANTITY_OFFSET: usize = 0x20;

/// **Flat slot index -> the INTERNAL index [`ITEM_SET_EQUIP`] takes.** 52 entries.
///
/// Read from the game's own table at `0x1410c0b90`, which `MapFlatSlotToInternal` (`0x140154510`)
/// indexes. `-1` is a flat position belonging to no category; nothing uses those.
///
/// # THE WEAPON HANDS ARE SWAPPED, and nothing complains if you get it wrong
///
/// Flat `0` is Left Hand 1 and flat `1` is Right Hand 1, but they map to internal `1` and `0`.
/// Combined with the internal->ChrAsm remap at `0x1410c44e0` (`[1,0,3,2,5,4]`) this means
/// **internal slot 0 is the RIGHT hand**. Equipping a build's main weapon into internal 0 believing
/// it to be the left puts every weapon in the wrong hand, silently, on a character that otherwise
/// looks correct.
pub const ITEM_SLOT_FLAT_TO_INTERNAL: [i32; 52] = [
    1, 0, 3, 2, 5, 4, // weapons: LH1 RH1 LH2 RH2 LH3 RH3, hands swapped
    6, 7, 8, 9, // armour: head chest hands legs
    -1, -1, // no category
    14, 15, 16, 17, // ammo: two arrow slots then two bolt slots
    10, 11, 12, 13, // rings
    18, 19, 20, 21, 22, 23, 24, 25, 26, 27, // hotbar, ten of them
    28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, // attunement, fourteen
    42, 43, 44, 45, 46, 47, 48, 49, // gestures
];

/// FLAT slot of the first weapon. The planner's own `lh1 rh1 lh2 rh2 lh3 rh3` order, position for
/// position -- read off the form's field ids, not guessed from what tends to sit in slot zero.
pub const ITEM_SLOT_WEAPON_FLAT_BASE: usize = 0;

/// FLAT slot of the head armour piece, then chest, hands, legs -- the planner's `head chest hands
/// legs` order, position for position.
pub const ITEM_SLOT_ARMOUR_FLAT_BASE: usize = 6;

/// FLAT slot of the first ring. Four, matching the planner's `ring-1 .. ring-4`.
pub const ITEM_SLOT_RING_FLAT_BASE: usize = 16;

/// FLAT slot of the first hotbar item. Ten, matching the planner's `item-1 .. item-10`.
pub const ITEM_SLOT_HOTBAR_FLAT_BASE: usize = 20;

/// FLAT slot of the first attunement position. Fourteen, matching `spell-1 .. spell-14`.
pub const ITEM_SLOT_SPELL_FLAT_BASE: usize = 30;

/// Internal slot of the first WEAPON. Six follow, and [`ITEM_SLOT_FLAT_TO_INTERNAL`] swaps the hands.
pub const ITEM_SLOT_WEAPON_BASE: u32 = 0;

/// Internal slot of the HEAD armour piece. Then chest, hands, legs.
///
/// Proved by the equip function's own gate: internal 6 requires the armour's `ArmorParam+0x4F == 2`,
/// 7 requires 3, 8 requires 4, 9 requires 5.
pub const ITEM_SLOT_ARMOUR_BASE: u32 = 6;

/// Internal slot of the first RING. Four of them.
pub const ITEM_SLOT_RING_BASE: u32 = 10;

/// Internal slot of the first HOTBAR item. **Ten**, not ELDEN RING's arrangement.
pub const ITEM_SLOT_HOTBAR_BASE: u32 = 18;

/// How many hotbar slots there are. `10`, from the predicate `ecx-0x12 <= 9`.
pub const ITEM_SLOT_HOTBAR_COUNT: usize = 10;

/// Internal slot of the first ATTUNEMENT position. Fourteen.
pub const ITEM_SLOT_SPELL_BASE: u32 = 28;

/// The most attunement positions a character can have. `14`.
pub const ITEM_SLOT_SPELL_COUNT: usize = 14;

/// **Recompute the attunement slot count from the character's attunement.** RVA `0x001b40f0`.
///
/// `fn(BagList*)`. Sets [`ITEM_BAG_ATTUNEMENT_SLOTS_OFFSET`] to `bonus + PlayerParam[0x36]`, capped
/// at [`ITEM_SLOT_SPELL_COUNT`], then walks the spell slots summing each spell's cost and
/// unequipping everything past the budget.
///
/// # Writing the stats does NOT do this, and that is measured
///
/// [`PLAYER_PARAM_SET_ALL_STATS`] recomputes the effective stats, the derived block and the soul
/// level, and reapplies the HP and stamina caps. It does not touch the bag. So a character written
/// straight to attunement 30 still has whatever slot count it had before -- and a blank mule
/// character has ZERO. A real run wrote attunement 30 and then read `attunement gives 0 slot(s)`,
/// dropping every spell in the build as over budget.
///
/// Call it after the stats and before attuning anything. It is what the game runs when attunement
/// changes; the unequip-over-budget half is its job, not a side effect to fear.
///
/// Not Arxan-redirected.
pub const ITEM_BAG_RECALC_ATTUNEMENT: u32 = 0x001b_40f0;

/// The five bytes [`ITEM_BAG_RECALC_ATTUNEMENT`] must begin with. `push rbx/rbp/rsi/rdi/r14`.
pub const ITEM_BAG_RECALC_ATTUNEMENT_PROLOGUE: [u8; 5] = [0x40, 0x53, 0x55, 0x56, 0x57];

/// **How many attunement slots the character actually has.** `BagList + 0x259ec`, `u8`.
///
/// # Attunement is a BUDGET, not a count of positions
///
/// `RecalcAttunementSlots` (`0x1401b40f0`) sets this to `bonus + PlayerParam[0x36]`, capped at
/// [`ITEM_SLOT_SPELL_COUNT`], then walks the spell slots summing each spell's own cost and
/// unequipping everything past the budget. **A spell can cost more than one slot.**
///
/// Attuning into a position at or past this value UNEQUIPS that position instead of equipping --
/// silently. So a caller must read this and stop, or a build naming fourteen spells loses most of
/// them with nothing said. It must also be read AFTER the stats are written, or it sizes against
/// the old attunement.
///
/// Corroborated by the community table's "Additional Magic Slots" AOB, which is byte-for-byte the
/// `movzx ecx,[rsi+0x259ed]; movsx eax,[rax+0x2e]` pair in that function.
pub const ITEM_BAG_ATTUNEMENT_SLOTS_OFFSET: usize = 0x259EC;

/// **`ItemSpawn + 0x00` is a MODE, not an unknown.** `u8`; the rest of the dword is ignored.
///
/// `0` grants normally. `1` refuses if the item is already held (`0x1401a9779`). `2` spends the
/// bag's secondary slot budget. The community tables call it unknown and always write `0`, which is
/// right for a build import and is right by accident.
pub const ITEM_SPAWN_MODE_NORMAL: u32 = 0;

/// **What to put in `ItemSpawn + 0x08`: `-1`, all bits set.** Maximum durability.
///
/// # This was briefly changed to zero, and zero means BROKEN
///
/// A static reading found the game's own caller at `0x140198a9e` zero-filling `+0x04..+0x0F` and
/// concluded that the community tables' `-1 = maximum` convention was unsupported. So this became
/// `0.0`, and the next run handed the player a full set of gear at zero durability.
///
/// The static reading was not wrong about that one caller. It was wrong as an argument for changing
/// a value that WORKED: `-1` had been granting usable gear, and the only evidence offered against it
/// was a different function's behaviour. Runtime evidence beats a static inference about what a
/// value ought to be, and there was no runtime evidence for zero.
///
/// It is typed as an INTEGER here because that is what it is. The old code declared the field
/// `f32` and wrote `f32::from_bits(-1i32 as u32)` -- a NaN, which happens to be the same four bytes
/// and so happened to work. Same bytes, honest type.
pub const ITEM_SPAWN_DURABILITY_MAX: i32 = -1;

/// Bytes in one `ItemSpawn`, the element [`ITEM_GIVE`] takes an array of.
///
/// Layout, corroborated by the two community writers and by the callee's own `add rbx,0x10` stride:
/// `+0x00` unknown `u32` (both writers store `0`), `+0x04` `i32` item id, `+0x08` `f32` durability,
/// `+0x0C` `u16` quantity, `+0x0E` `u8` reinforce, `+0x0F` `u8` infusion.
pub const ITEM_SPAWN_SIZE: usize = 0x10;

// ---------------------------------------------------------------------------------------------
// THE ESTUS FLASK.
//
// The flask's upgrade level is NOT an item id and NOT a shop transaction. It is two bytes on the
// flask's own inventory entry, written by a plain method on `ItemInventory2` that reads nothing but
// the inventory and two param tables. The Emerald Herald's dialogue is the TRIGGER, not the work:
// her script command at `0x140464ced` is, in its entirety, `get(level); set(level + 1)` on the
// functions below. Nothing in the chain touches a menu, an NPC, a talk state or `PlayerCtrl`, which
// is why a pause-menu row can do the same thing.
//
// Every RVA here was traced through its thunks and disassembled to the end in
// `darksoulsii-deobf.bin`, and none of them is Arxan-redirected.
// ---------------------------------------------------------------------------------------------

/// `ItemInventory2::SetEstusProperty`. RVA `0x001ac5a0`.
/// `fn(ItemInventory2*, const u8* property, i32 level) -> bool`.
///
/// # `property` is a POINTER to a byte, and a wrong one writes the wrong field
///
/// The body does `movsx r8, byte [rdx]` and then indexes `entry + r8 + 0x25`. That is a SIGNED
/// extension, so the `0xFF` [`ESTUS_PROPERTY_INDEX`] returns for a key it does not know would land
/// on `entry + 0x24` -- the CHARGE COUNT -- and write a level into it. Never pass an index without
/// checking it against [`ESTUS_PROPERTY_NOT_FOUND`] first.
///
/// # What it does, read to the end
///
/// `entry = [list + 0x40]`; null returns false. The entry's item id is put through the list's own
/// `vtable[0x48]`, which is `ItemParam[id] + 0x40 == 450` -- the flask's identity, and the reason
/// `60155010/20/30` can never bind here (they are `420`). It then clamps `level` into the
/// `[min, max]` of `EstusFlaskMaxReinforceParam` for this property, writes `entry + 0x25 + property`
/// and, **only for property 0**, moves the charge count by the difference in uses the two levels
/// buy. Finally it mirrors the charge count and both level bytes into the save record.
///
/// # `false` means "nothing changed", which is not the same as "failed"
///
/// A level already equal to the one asked for returns false having done nothing. Read the level
/// back with [`ESTUS_GET_LEVEL`] rather than trusting the bool.
///
/// # Not the sibling one address later
///
/// `0x001ac5b0` reaches the same body behind `cmp byte [inner + 0x10144], 2; jne return false` --
/// a gate twelve bytes past [`ITEM_GIVE_ERROR_OFFSET`]. Character creation calls the gated one; the
/// Herald's script calls this one.
pub const ESTUS_SET_PROPERTY: u32 = 0x001a_c5a0;

/// `ItemInventory2::GetEstusPropertyLevel`. RVA `0x001abc40`.
/// `fn(ItemInventory2*, const u8* property) -> u8`.
///
/// Reads `entry + 0x25 + property` -- so `1..12` for the uses axis and `1..6` for the effect axis.
///
/// **Zero means there is no flask.** The getter answers `0` when the flask's list slot is unbound
/// or holds something that is not the flask, and the game's own add path seeds a new flask at the
/// param minimum, which is `1`. So zero is not a level.
///
/// The two bytes it reads are a UNION. On a weapon or armour entry (`entry + 0x1E <= 5`) the same
/// bytes carry reinforcement and infusion in their low nibbles. Only read them on the flask.
pub const ESTUS_GET_LEVEL: u32 = 0x001a_bc40;

/// `ItemInventory2::GetEstusCharges`. RVA `0x001abc60`. `fn(ItemInventory2*) -> u8`.
///
/// `entry + 0x24` -- uses remaining right now, which is the number under the flask in the HUD.
pub const ESTUS_GET_CHARGES: u32 = 0x001a_bc60;

/// `ItemInventory2::IsEstusPropertyAtMax`. RVA `0x001ac1d0`.
/// `fn(ItemInventory2*, const u8* property) -> bool`.
///
/// The game's own "is this axis finished" predicate: the level compared against
/// `EstusFlaskMaxReinforceParam`'s max for that property. **Worth calling instead of comparing
/// against a hardcoded 12**, because that number lives in a param a regulation mod can change.
pub const ESTUS_IS_MAX: u32 = 0x001a_c1d0;

/// `ItemInventory2::RefillEstus`. RVA `0x001ac370`. `fn(ItemInventory2*)`.
///
/// Sets the charge count to whatever the current uses level buys -- the bonfire's own refill.
/// [`ESTUS_SET_PROPERTY`] already moves the count by the difference when the level rises, so this
/// is what makes the result the same whether or not the player had been drinking.
pub const ESTUS_REFILL: u32 = 0x001a_c370;

/// The five bytes each of the Estus thunks must begin with. `mov rcx,[rcx+0x10]`, then a tail-jump.
///
/// Every one of them is the same two-hop shape as [`ITEM_SET_EQUIP`], and the two hops are the same
/// two that reach [`ITEM_BAG_LIST_OFFSET`] -- so they all take `ItemInventory2`, the object
/// [`ITEM_GIVE`] takes.
pub const ESTUS_THUNK_PROLOGUE: [u8; 5] = [0x48, 0x8b, 0x49, 0x10, 0xe9];

/// The game's own property-key to table-index lookup. RVA `0x001ad140`. `fn(u32 key) -> u8`.
///
/// A six-instruction leaf that scans the two-entry table at `0x14156b030` (`{const wchar_t* name;
/// u32 key}`, stride `0x10`, sentinel at `0x14156b050`) and returns `(row - base) >> 4`, or
/// [`ESTUS_PROPERTY_NOT_FOUND`]. It reads no game state, so it is safe to call at any time.
///
/// The mapping is the identity on this build. Call it anyway -- it is what the game's own callers
/// do, and it is the only thing that would notice a regulation that reordered the table.
pub const ESTUS_PROPERTY_INDEX: u32 = 0x001a_d140;

/// The seven bytes [`ESTUS_PROPERTY_INDEX`] must begin with. `lea r8,[rip+0x13bdee9]`.
///
/// Unusually load-bearing for a prologue check: the displacement IS the address of the property
/// table, so a match confirms the function is reading the table this documentation describes.
pub const ESTUS_PROPERTY_INDEX_PROLOGUE: [u8; 7] = [0x4c, 0x8d, 0x05, 0xe9, 0xde, 0x3b, 0x01];

/// [`ESTUS_PROPERTY_INDEX`]'s answer for a key it does not know. `0xFF`.
///
/// **Must be rejected rather than passed on.** See [`ESTUS_SET_PROPERTY`] -- it is sign-extended
/// into an index, so it addresses the byte BEFORE the levels.
pub const ESTUS_PROPERTY_NOT_FOUND: u8 = 0xFF;

/// The number of uses -- the axis Estus Flask Shards raise. Property key `0`.
///
/// Named in the binary: the table row's name pointer is `0x1410c3e40`, the UTF-16 string
/// `使用回数`, "number of uses".
pub const ESTUS_PROPERTY_USES: u32 = 0;

/// How much each use heals -- the axis Sublime Bone Dust raises. Property key `1`.
///
/// Named in the binary at `0x1410c3e50`: `効果量`, "effect amount". The same setter drives it; there
/// is no separate bone-dust function, and the in-game path (`0x14017f420`, the item-use handler)
/// differs only in writing a presentation flag on the player first.
pub const ESTUS_PROPERTY_EFFECT: u32 = 1;

/// What to ask [`ESTUS_SET_PROPERTY`] for when the intent is "as high as this game allows". `99`.
///
/// # Why not 12
///
/// The maxima live in `EstusFlaskMaxReinforceParam` (`12` uses and `6` effect as shipped), which a
/// regulation mod can change, and the setter clamps into that param's own range before it writes.
/// So asking for more than any real maximum and letting the game decide is exactly right, and it
/// keeps a number out of this file that would silently become a LIMIT on a modded install.
///
/// The comparisons are SIGNED, so this must stay positive; a negative would clamp UP to the minimum.
pub const ESTUS_LEVEL_ASK: i32 = 99;

/// The one Estus Flask a character can hold. `60155000`.
///
/// The three neighbours the item catalogue also calls `Estus Flask` are not upgrade states -- the
/// upgrade level is a byte on the entry, and the held id never changes. They are rows of
/// `EstusFlaskLvDataParam` whose ids are `60155000 + (row - 1)`, reachable only at effect levels
/// 11, 21 and 31 in a game that caps the effect level at 6. `ItemParam[id] + 0x40` is `450` for
/// this id and `420` for those three, and `450` is what the flask's list checks, so they could not
/// bind even if something granted them.
pub const ESTUS_FLASK_ITEM_ID: i32 = 60155000;

/// `PlayerParam::AddSouls`. RVA `0x0038ab40`. `fn(PlayerParam*, u32 amount)`.
///
/// **This is how soul memory is raised through the game rather than written.** One call updates
/// THREE counters together -- souls held ([`PLAYER_PARAM_SOULS_HELD_OFFSET`]) and both soul-memory
/// fields ([`PLAYER_PARAM_SOUL_MEMORY_OFFSETS`]) -- and then fires the change notifications
/// `0x11`, `0x14`, `0x12`. Doing it by hand means three writes and no notification.
///
/// Verified: prologue `48 83 ec 28`, the soul-memory read at `0x14038abc8` is
/// `44 8b 81 f4 00 00 00`, and the saturation cap at `0x14038ab76` is `b8 ff c9 9a 3b`.
///
/// **Add-only and saturating.** `edx` is compared unsigned, so there is no negative amount, and
/// every counter clamps at [`PLAYER_PARAM_SOULS_CAP`]. So THIS function cannot lower soul memory.
///
/// # The game does have a path that lowers it, and this used to say otherwise
///
/// The claim here was "the game offers no path that lowers it". That is false.
/// [`PLAYER_PARAM_RESTORE_FROM_RECORD`] assigns all three counters ABSOLUTELY -- clamped at the top
/// against [`PLAYER_PARAM_SOULS_CAP`] and not at the bottom -- and `0x14038bc49` zeroes the second
/// soul-memory field outright. Monotonicity is a property of the paths a PLAYER can reach, not of
/// the class.
///
/// A build importer still only ever raises, and now for a reason that is a choice rather than a
/// mistaken impossibility: `soul memory >= what the level cost` is the invariant, and lowering a
/// level keeps it satisfied. There is nothing to reduce.
///
/// **It can silently do nothing, twice over.** A status flag on the player
/// (`PlayerCtrl -> +0xB8 -> +0x4B8 & 0x0800000000000000`) makes it return before touching anything
/// AND before notifying; and each counter has its own skip byte
/// ([`PLAYER_PARAM_SOUL_COUNTER_GUARDS`]) that suppresses just that one. So a caller must read the
/// counters back rather than trusting the call.
pub const PLAYER_PARAM_ADD_SOULS: u32 = 0x0038_ab40;

/// `PlayerParam::RestoreFromRecord`. RVA `0x0038ad20`. `fn(PlayerParam*, const SavedRecord*)`.
///
/// **The only path in the class that ASSIGNS the soul counters rather than adding to them**, and
/// the reason [`PLAYER_PARAM_ADD_SOULS`] no longer claims lowering is impossible.
///
/// It is the character-load path. Read in full at `0x14038ad20`: the covenant from `record + 0x9C`
/// into `PlayerParam + 0x1AC`, then a call to [`PLAYER_PARAM_SET_ALL_STATS`] -- the same function
/// this repo already calls for stats -- then souls held from `record + 0x1C`, soul memory from
/// `record + 0x20` and `record + 0x24`. Each of the three is clamped against
/// [`PLAYER_PARAM_SOULS_CAP`] at the TOP only, guarded by its own byte from
/// [`PLAYER_PARAM_SOUL_COUNTER_GUARDS`], and then written absolutely. A smaller number goes in
/// unchanged.
///
/// # Recorded as a fact, NOT as a route to call
///
/// It takes a whole saved-character record and writes far more than the counters, so using it to
/// lower soul memory would mean fabricating that record -- which is the one thing this project does
/// not do. It is here so the next reader does not re-derive "there is no such function" from
/// `AddSouls` alone, which is exactly how the wrong claim got written the first time.
pub const PLAYER_PARAM_RESTORE_FROM_RECORD: u32 = 0x0038_ad20;

/// The four bytes [`PLAYER_PARAM_ADD_SOULS`] must begin with. `sub rsp,0x28`, then `mov rax,[rcx]`.
pub const PLAYER_PARAM_ADD_SOULS_PROLOGUE: [u8; 7] = [0x48, 0x83, 0xec, 0x28, 0x48, 0x8b, 0x01];

/// Where every soul counter saturates. `999_999_999` -- `mov eax,0x3b9ac9ff` at `0x14038ab76`.
pub const PLAYER_PARAM_SOULS_CAP: u32 = 0x3b9a_c9ff;

/// Per-counter skip bytes, in the order of souls-held then the two soul-memory fields.
///
/// Non-zero means [`PLAYER_PARAM_ADD_SOULS`] leaves that counter alone. They are not padding: the
/// constructor at `0x14038aa20` initialises each one beside its counter as `{u32 value; u8 guard;
/// pad[3]}`, and that record repeats as an array from `+0x104`.
pub const PLAYER_PARAM_SOUL_COUNTER_GUARDS: [usize; 3] = [0xF0, 0xF8, 0x100];

/// Bytes in the `StatBlock` embedded at [`PLAYER_PARAM_STAT_OFFSETS`]`[0]`. `0xCC`.
///
/// It spans `PlayerParam + 0x08 .. + 0xD3` and holds BOTH stat copies plus the derived block and
/// the soul level. Proved from the class's own constructor `0x14038c060`: eleven `u16` at `+0x00`,
/// eleven more at `+0x16`, a `0x9c`-byte `memset` at `+0x2c`, and a `u32` at `+0xc8`.
/// `0x2c + 0x9c = 0xc8`, `+ 4 = 0xcc`.
///
/// **This number is what settles which fields a stat write must maintain.** The game's own commit
/// path blits exactly `0xCC` bytes and stops; anything past `+0xD3` is a different member and not
/// this write's business.
pub const PLAYER_PARAM_STAT_BLOCK_BYTES: usize = 0xCC;

/// **Eleven `u16` that are NOT a third stat copy, and are deliberately left alone.** `+0xD4`.
///
/// # Why this offset has a constant at all when nothing writes it
///
/// It sits immediately after the stat block, is eleven `u16` wide, and is zeroed by the
/// `PlayerParam` constructor -- which is exactly what a third copy of the stats would look like
/// from the outside. This crate has already misread one neighbouring field that way (see
/// [`PLAYER_PARAM_EFFECTIVE_STATS_OFFSET`]), so it is recorded WITH ITS ANSWER, to stop the next
/// reader re-deriving the same suspicion.
///
/// # What it is
///
/// The "points allocated per stat" array that follows an embedded `StatBlock`. The level-up request
/// object has the identical shape -- a `StatBlock` at `+0x00` and eleven `u16` zeroed at `+0xCC`,
/// the same relative offset -- and THERE the array is used: `0x1401fba30` increments
/// `[req + i*2 + 0xCC]` when a stat goes up and `0x1401fb890` decrements it, guarded so a player can
/// only take back points they put in. Those are the only per-element accesses to the array in the
/// whole image, and they are all on the request, never on `PlayerParam`.
///
/// # Why leaving it stale is correct
///
/// **In `PlayerParam` it is written exactly once, by the constructor, and never again** -- not by
/// the commit path (`0x14038b1a0` copies `0xCC` bytes and stops at `+0xD3`), and not by the
/// character-load path `0x14038ad20`, which restores the stats, souls, soul memory and covenant and
/// skips this. A loaded character has zeros here, same as a new one. So a mod that writes the stats
/// and leaves this alone is doing what the game does.
///
/// It has one reader, `0x14038b350`, which `memcmp`s it against a 22-byte set and recomputes the
/// stat block when they differ. Since the field is never written, that comparison is against 22
/// zero bytes forever -- a memoisation whose update side was never emitted. Vanilla behaviour;
/// nothing a mod does changes it.
///
/// There is no FOURTH stat-shaped array: `PlayerParam` is `0x1E0` bytes (allocation site
/// `0x14037f499`) and holds exactly three eleven-`u16` regions -- `+0x08`, `+0x1E`, and this one.
pub const PLAYER_PARAM_POINTS_PER_STAT_OFFSET: usize = 0xD4;

/// **The EFFECTIVE stat block: what the character's stats currently amount to.** `+0x1E`.
///
/// Eleven `u16`, immediately after the base block at [`PLAYER_PARAM_STAT_OFFSETS`].
///
/// # This was called a MIRROR, and that was wrong
///
/// The name came from reading only [`PLAYER_PARAM_SET_ALL_STATS`]'s opening, which writes each of
/// the eleven `u16` to two destinations. Verified bytes at `0x14038acba`:
///
/// ```text
/// 48 89 41 08   mov [rcx+0x08],rax     ; the base block
/// 48 89 41 1e   mov [rcx+0x1e],rax     ; ...and the same qword again, 0x16 further on
/// 4c 89 41 10   mov [rcx+0x10],r8
/// 4c 89 41 26   mov [rcx+0x26],r8
/// 44 89 49 18   mov [rcx+0x18],r9d
/// 44 89 49 2e   mov [rcx+0x2e],r9d
/// ```
///
/// Those writes are real and they do make the two ranges briefly identical. **But they are a SEED,
/// not a copy.** The same function then calls `0x14038d6d0`, which recomputes the effective block
/// FROM the base block and writes it back over `+0x1E`. So the two are equal only for a character
/// with nothing modifying its stats, and reading a difference as corruption is a false alarm.
///
/// **MEASURED IN GAME, 2026-08-28.** A character written to `int=38 faith=38` displayed `40`/`40`
/// in its own attribute menu, every other stat exact. The menu reads THIS block. Two stats carried
/// a `+2` from somewhere, and both ranges were already unequal before anything was written --
/// which is what makes "one of these is derived" the only reading that fits.
///
/// # What it is good for
///
/// Reading what the character's stats effectively ARE, and nothing else. It is NOT a tamper check,
/// it is NOT a write target, and a caller that wants the levelled stats wants
/// [`PLAYER_PARAM_STAT_OFFSETS`].
///
/// Neither community table maps this offset at all.
pub const PLAYER_PARAM_EFFECTIVE_STATS_OFFSET: usize = 0x1E;

/// Bytes covered by one copy of the stat block: eleven `u16`.
pub const PLAYER_PARAM_STAT_BLOCK_SIZE: usize = 22;

/// The commit path for a stat change: validate a request, recompute, blit, charge. RVA `0x0038b1a0`.
///
/// It calls a validator, calls `0x14038c0d0` for a freshly computed stat block, `memcpy`s **`0xCC`
/// bytes over `PlayerParam + 0x08`** -- covering both copies of the stat block and everything to
/// `+0xD3` -- then subtracts the level cost from souls held while leaving both soul-memory counters
/// alone, which is exactly right for spending souls.
///
/// **Recorded as a LEAD, not as a callable.** The `request` object in `rdx` has an unknown layout,
/// and whether it can be built without the attribute menu having built it is the open question that
/// decides whether a headless level-up is possible at all. Calling this with a malformed request is
/// not a thing to try casually: it blits 204 bytes into the live character.
pub const PLAYER_PARAM_COMMIT_STATS: u32 = 0x0038_b1a0;

/// **Set all eleven stats and let the game derive everything else.** RVA `0x0038aca0`.
///
/// `fn(PlayerParam*, *const [u16; 11])`. This is how a build is applied, and it is the reason
/// nothing here writes a soul level.
///
/// # THE SOUL LEVEL IS NOT A FIELD YOU SET, IT IS A SUM
///
/// `0x14038e310` computes `level = max(1, sum(stats[0..8]) - 53)` and writes it to
/// [`PLAYER_PARAM_SOUL_LEVEL_OFFSET`]. The `53` is `9 * 6 - 1`: a Deprived starts every stat at 6,
/// `9 * 6 = 54`, and `54 - 53 = 1`. So a character's level is a function of its nine stats and
/// there is no way to disagree with it -- writing a level would be writing a cached sum. Cheaply
/// checkable: soulsplanner build 253's stats total 203, and `203 - 53 = 150`, the level the planner
/// shows.
///
/// The two `u16` at index 9 and 10 (`+0x1A`, `+0x1C`) are NOT in that sum. Read them out of the
/// character and write them back unchanged; nothing here knows what they are.
///
/// # What one call does, from the disassembly
///
/// Loads all 22 bytes out of `rdx` and writes them to BOTH `+0x08` and
/// [`PLAYER_PARAM_EFFECTIVE_STATS_OFFSET`], fetches the character context through `PlayerCtrl`
/// vtable slot `0x130`, calls `0x14038d6d0` to recompute the effective stats, the derived block AND
/// the soul level, then tail-calls `0x14038be90` to reapply the HP and stamina caps to the live
/// character.
///
/// **The write to `+0x1E` is overwritten by that recompute** -- it is a seed, not a second copy.
/// See [`PLAYER_PARAM_EFFECTIVE_STATS_OFFSET`], which was misread as a mirror for exactly this
/// reason.
///
/// **That is the entire argument for using this instead of [`PLAYER_PARAM_COMMIT_STATS`].** Both
/// end in the same recompute; this one takes 22 bytes of plain `u16` with no invariants, and the
/// other takes a 0x104-byte plan with a seven-clause validator, a souls ledger and a
/// `levels_bought == planned_level - base_level` identity to satisfy. Fewer ways to build a
/// character that looks right and is wrong.
///
/// Not Arxan-redirected. Six existing callers.
pub const PLAYER_PARAM_SET_ALL_STATS: u32 = 0x0038_aca0;

/// The five bytes [`PLAYER_PARAM_SET_ALL_STATS`] must begin with. `mov [rsp+8],rbx`.
pub const PLAYER_PARAM_SET_ALL_STATS_PROLOGUE: [u8; 5] = [0x48, 0x89, 0x5c, 0x24, 0x08];

/// Stats in one `PlayerParam` block, INCLUDING the two that are not levelable.
///
/// [`PLAYER_PARAM_SET_ALL_STATS`] reads all eleven; [`PLAYER_PARAM_STAT_OFFSETS`] names the nine
/// that a level buys.
pub const PLAYER_PARAM_STAT_COUNT: usize = 11;

/// What the soul-level sum subtracts. `level = max(1, sum(nine stats) - 53)`.
///
/// From `0x14038e310`. See [`PLAYER_PARAM_SET_ALL_STATS`] for why this is `9 * 6 - 1`.
pub const PLAYER_PARAM_SOUL_LEVEL_BIAS: u32 = 53;

/// Opens the attribute menu. RVA `0x001992c0`. `fn(menuMgr, mode, *const [f32; 8])`.
///
/// `mode` `1` pushes menu `0x19` (Level Up) and `0` pushes `0x1a` (Reallocate Stats); any other
/// value is a silent no-op. `menuMgr` is `[[GAME_MANAGER_IMP + 0x70] + 0x50]`, and the third
/// argument is a **16-byte-aligned** scratch buffer whose first four floats are copied from
/// `PlayerCtrl + 0x90`, the player's world position -- the callee reads it with `movaps`, so an
/// unaligned buffer faults.
///
/// **It opens the UI. It does not level anything**, and a human still picks the stat. Recorded
/// because it is the only level path any community table has, and because it routes through
/// [`GAME_MANAGER_FRONTEND_ROOT_OFFSET`] -- machinery `ds2-menu-row` already touches.
pub const FE_OPEN_ATTRIBUTE_MENU: u32 = 0x0019_92c0;

/// `SaveLoadSystem::RequestSave`. RVA `0x002e7410`. `fn(saveLoadSystem, kind)`, `kind = 2`.
///
/// `saveLoadSystem` is `[GAME_MANAGER_IMP + `[`SAVE_LOAD_SYSTEM_OFFSET`]`]`. How a change is
/// persisted without waiting for a bonfire.
///
/// # It RETURNS VOID, and it does not perform a save
///
/// Transcribed in full, because reading it as "save now, tell me if it worked" is the mistake it
/// invites:
///
/// ```asm
/// 0x1402e7410:  cmp  edx,0xe                 ; kind 14 is its own flag and nothing else
///               jne  0x1402e741d
///               mov  BYTE PTR [rcx+0x1a9],1
///               ret
/// 0x1402e741d:  cmp  edx,[rcx+0x68]          ; keep the LOWEST kind asked for
///               jge  0x1402e7425
///               mov  [rcx+0x68],edx
/// 0x1402e7425:  mov  BYTE PTR [rcx+0x1a2],1  ; "a save is wanted"
///               cmp  edx,0x2
///               jne  0x1402e7438
///               mov  BYTE PTR [rcx+0x1a3],1  ; and kind 2 sets a second flag
/// 0x1402e7438:  repz ret
/// ```
///
/// Three byte writes and a `min`. **There is no return value to check** -- a caller that branches on
/// RAX is branching on whatever the last call left there -- and the save itself happens later, from
/// `GameManagerImp`'s master update, exactly like the shutdown byte `ds2-menu-row` writes. So the
/// only way to know a save HAPPENED is to watch something else: the interlock at
/// [`SAVE_LOAD_SYSTEM_STATE_OFFSET`], or the file on disk.
///
/// Not an Arxan redirect. `scripts/ds2-arxan-chain.py` reports `UNKNOWN` at hop 0 only because its
/// prologue table does not carry `83 fa` (`cmp edx, imm8`); the entry is ordinary code rather than
/// the five-byte `e9` stub a redirected entry keeps in the deobfuscated image.
pub const SAVE_LOAD_REQUEST_SAVE: u32 = 0x002e_7410;

/// The three bytes [`SAVE_LOAD_REQUEST_SAVE`] must begin with, or a caller refuses to call it.
///
/// `cmp edx,0xe` -- the kind-14 special case, which is the first thing the function does.
pub const SAVE_LOAD_REQUEST_SAVE_PROLOGUE: [u8; 3] = [0x83, 0xfa, 0x0e];

/// The `kind` [`SAVE_LOAD_REQUEST_SAVE`] is asked for when the point is "persist the character now".
///
/// `2` is the value that sets BOTH flags the function can set (`+0x1a2` and `+0x1a3`), and it is the
/// kind the pump at `0x1402e6230` accepts alongside `4`.
pub const SAVE_LOAD_REQUEST_KIND_CHARACTER: u32 = 2;

// =================================================================================================
// THE INVENTORY TAB'S SORT DIALOG, AND THE OBJECT THAT OWNS IT
//
// DARK SOULS II ALREADY SHIPS INVENTORY SORTING. It is not a feature to build; it is a feature
// reachable only from the button FromSoftware chose, and DS2 exposes no per-action rebinding for
// menus -- `win32onlymessage.fmg` 10332..10341 is the complete list of rebindable menu actions and
// "Sort" is not among them; it rides on one of two generic Function keys. Everything here exists so
// a mod can offer the SAME dialog on a different button.
//
// The option table lives at VA `0x14155ddf0` as 15 `{u32 FE_ITEM_PARAM_TYPE, u32 fmgId}` pairs,
// sliced per category by the function at `0x1400349d0`, and the values it sorts by come from a
// ~95-case switch (`0x140032460` and siblings) -- key `0x5C` is the sum of all seven attack
// components, which is a real total attack rating. None of that is needed to REBIND the dialog, so
// none of it is transcribed here. See `docs/DS2-INVENTORY-SORT.md`.
// =================================================================================================

/// **The sort dialog, opened. RVA `0x00074_7e0`. `fn(this)` and nothing else.**
///
/// `this` is a live [`FE_INVENTORY_GROUP_VTABLE`] object. The function reads the current category
/// through its own vtable slot `+0x140`, asks `0x1400349d0` for that category's slice of the option
/// table, builds one row per entry, and shows the dialog headed by `common.fmg` 80059901 -- "How
/// should the list be sorted?". Selecting a row runs the game's own apply path.
///
/// # It takes ONE argument, which is why this mod is small
///
/// Verified from its own prologue and body: `push rbp; push rbx; lea rbp,[rsp-0x888];
/// sub rsp,0x988`, then `cmp qword [rcx+0x58],0` / `mov rbx,rcx` and a `jne` to the epilogue. `RDX`
/// and `R8` are dead on entry -- both are overwritten by `lea` before any read. So a caller needs
/// the group pointer and nothing else: no job object, no functor, no synthesised input.
///
/// # It guards itself
///
/// `[this+0x58]` is non-zero while the group already has a dialog up, and the function returns
/// having done nothing in that case. A hotkey that fires while the sort dialog (or any other child
/// dialog) is open is therefore refused BY THE GAME rather than by the mod's own guess at whether
/// now is a good time.
pub const FE_INVENTORY_SORT_DIALOG_OPEN: u32 = 0x0007_47e0;

/// The five bytes [`FE_INVENTORY_SORT_DIALOG_OPEN`] must begin with, or the mod refuses to call it.
///
/// `push rbp; push rbx; lea rbp,...`. Not Arxan-redirected -- the first byte is `0x40`, and an
/// Arxan-redirected entry keeps its five-byte `e9` stub in the deobfuscated image.
pub const FE_INVENTORY_SORT_DIALOG_OPEN_PROLOGUE: [u8; 5] = [0x40, 0x55, 0x53, 0x48, 0x8d];

/// `FeGroupInGameMenuInventory2`'s primary vtable. RVA `0x010b_1b38`, 130 slots.
///
/// **This is the identity check, and it is the reason no ctor hook needs to trust a name.** The
/// constructor writes this pointer to `[this]` (`lea rax,[rip+0x1040185]; mov [rsi],rax` at
/// `0x1400719ac`), plus three secondary vtables at `+0x50`, `+0xc8` and `+0x20a8`. A cached pointer
/// whose first qword is no longer this value is not the inventory group any more.
pub const FE_INVENTORY_GROUP_VTABLE: u32 = 0x010b_1b38;

/// `FeGroupInGameMenuInventory2::ctor`. RVA `0x0007_16f0`. `fn(this, ?, ?) -> this`.
///
/// Hooked only to learn WHERE the live group is. The object is heap-allocated (`0x3EE8` bytes) by
/// the factory at `0x1400727b0`, which is itself a vtable slot of the `LoadAndExecJobSequence`
/// inside `FeGroupInGameMenuInventory2::CreateExecJob` -- so the pause menu's Inventory tab creates
/// it on demand and there is no static pointer to read instead.
///
/// Its later arguments are not used by anything here and are deliberately not documented: a detour
/// that only records `RCX` and tail-calls the original does not need to know them.
pub const FE_INVENTORY_GROUP_CTOR: u32 = 0x0007_16f0;

/// The five bytes [`FE_INVENTORY_GROUP_CTOR`] must begin with. `mov [rsp+0x8],rbx`.
pub const FE_INVENTORY_GROUP_CTOR_PROLOGUE: [u8; 5] = [0x48, 0x89, 0x5c, 0x24, 0x08];

/// `FeGroupInGameMenuInventory2::v0` -- the scalar deleting destructor. RVA `0x0007_25d0`.
///
/// `fn(this, u8 flags) -> this`; bit `0` of `flags` means "and free the memory". Slot 0 of
/// [`FE_INVENTORY_GROUP_VTABLE`].
///
/// **Hooked for the same reason the constructor is, in the opposite direction.** Without it a mod
/// holds a pointer to freed memory from the moment the player closes the pause menu, and the
/// vtable check that would catch it is a read of that freed memory. Clearing the cache here is what
/// makes the check a belt rather than the only strap.
pub const FE_INVENTORY_GROUP_DTOR: u32 = 0x0007_25d0;

/// The five bytes [`FE_INVENTORY_GROUP_DTOR`] must begin with. `mov [rsp+0x8],rbx`.
pub const FE_INVENTORY_GROUP_DTOR_PROLOGUE: [u8; 5] = [0x48, 0x89, 0x5c, 0x24, 0x08];

/// `[group + 0x58]`, non-zero while the group already has a child dialog up.
///
/// Read only to LOG why a press did nothing. [`FE_INVENTORY_SORT_DIALOG_OPEN`] tests it itself and
/// refuses; this constant exists so the refusal is legible in the log rather than silent.
pub const FE_INVENTORY_GROUP_BUSY_OFFSET: usize = 0x58;

// ---------------------------------------------------------------------------------------------
// The equip screen's item picker -- the SAME sort, on a list the game never gave a button.
//
// `FeGroupItemEquip` is the list that opens when a slot on the Equipment screen is chosen. It is a
// sibling of `FeGroupInGameMenuInventory2` and of `FeItemBoxMenu` (the storage box): all three
// derive from the same base, all three carry secondary vtables at `+0x50` and `+0xc8`, and their
// `+0x50` vtables agree slot for slot through `+0x28` -- which is what makes `[this+0x58]` mean the
// same thing in each and lets ONE dialog opener serve all of them.
//
// THE SORT IS ALREADY THERE. `FeIngameItemSelectMenu::v57` (`0x140097400`, this class's vtable slot
// `+0x148`) rebuilds the list by calling the shared builder `0x140036080`, and that builder reads
// the per-category sort key at `0x140036108` -- the same key the Inventory tab's dialog writes. So
// a sort chosen in the Inventory tab ALREADY reorders this list; what the equip screen lacks is
// only the dialog to choose one without leaving the screen.
//
// Two of the three siblings ship an opener (`0x1400747e0` inventory, `0x1400c2ad0` storage box) and
// they are near-identical: both read `[this+0x58]`, both call vtable slot `+0x140` for the
// category, both slice the option table through `0x1400349d0`, both hand `this+0x50` to the dialog,
// and BOTH build their rows from the same closure -- vtable `0x1410b1ec0`, member function
// `0x1400ba300`. That shared closure is the proof the machinery is generic rather than per-class:
// the game already reuses one Inventory-named functor for a different class's list.

/// `FeGroupItemEquip`'s primary vtable. RVA `0x010b_46c8`.
///
/// The constructor writes it to `[this]` at `0x14008c17f`, with `0x1410b4848` at `+0x50`,
/// `0x1410b4890` at `+0xc8` and `0x1410b48a0` at `+0x320`. (The fourth differs from the Inventory
/// group's `+0x20a8`; nothing here touches it.)
pub const FE_EQUIP_GROUP_VTABLE: u32 = 0x010b_46c8;

/// `FeGroupItemEquip::ctor`. RVA `0x0008_c0d0`. **`fn(this, ?, u32, ?) -> this` -- FOUR arguments.**
///
/// The argument count is not cosmetic and was read rather than assumed: the entry instruction is
/// `mov [rsp+0x18],r8d` (spilling the third into its home slot) and `0x14008c0fc` is `mov rdi,r9`,
/// so `R9` is live on entry. A three-argument detour would forward three of the four and hand the
/// constructor a garbage `R9`. Nothing above the fourth home slot is read, so there are no stack
/// arguments.
pub const FE_EQUIP_GROUP_CTOR: u32 = 0x0008_c0d0;

/// The five bytes [`FE_EQUIP_GROUP_CTOR`] must begin with. `mov [rsp+0x18],r8d`.
///
/// Exactly five, which is the minimum MinHook needs -- and the whole instruction, so the trampoline
/// re-executes it against the detour's own home slot with `R8` restored to the caller's value.
pub const FE_EQUIP_GROUP_CTOR_PROLOGUE: [u8; 5] = [0x44, 0x89, 0x44, 0x24, 0x18];

/// `FeGroupItemEquip::v0` -- the scalar deleting destructor. RVA `0x0008_dc70`.
///
/// Slot 0 of [`FE_EQUIP_GROUP_VTABLE`], `fn(this, u32 flags) -> this`, same shape as the Inventory
/// group's. Hooked for the same reason: without it the record outlives the picker.
pub const FE_EQUIP_GROUP_DTOR: u32 = 0x0008_dc70;

/// The five bytes [`FE_EQUIP_GROUP_DTOR`] must begin with. `mov [rsp+0x8],rbx`.
pub const FE_EQUIP_GROUP_DTOR_PROLOGUE: [u8; 5] = [0x48, 0x89, 0x5c, 0x24, 0x08];

// ---------------------------------------------------------------------------------------------
// EACH LIST'S OWN PER-FRAME UPDATE, which is where a button has to be sampled.
//
// `ds2-menu-row` hooks `FeGroupInGameTopSelect::v2` (`FE_INGAME_TOP_SELECT_UPDATE`) -- the pause
// menu's TAB STRIP -- and that is the right place for a caption on the strip and the wrong place
// for a keypress inside a list. A run on 2026-08-29 pressed the bound key repeatedly on the equip
// picker and the log recorded two opens and ZERO refusals: presses that produced no line at all,
// which means the sampler never ran while the key was down. `GetAsyncKeyState`'s high bit answers
// "down at this instant", so a sampler that runs rarely misses taps entirely rather than late.
//
// The fix is to sample from the update of the list actually on screen. Slot index 2 of the primary
// vtable is the per-frame update throughout this family -- it is where `FeGroupInGameTopSelect::v2`
// sits, and `ds2-menu-row` has already proven that slot runs every frame for its class.

/// `FeItemSelectMenu::v2` -- the Inventory tab's own per-frame update. RVA `0x000b_bea0`.
///
/// Slot index 2 of [`FE_INVENTORY_GROUP_VTABLE`].
pub const FE_INVENTORY_GROUP_UPDATE: u32 = 0x000b_bea0;

/// The five bytes [`FE_INVENTORY_GROUP_UPDATE`] must begin with. `push rbp; push rbx; push rdi`.
pub const FE_INVENTORY_GROUP_UPDATE_PROLOGUE: [u8; 5] = [0x40, 0x55, 0x53, 0x57, 0x48];

/// `FeIngameItemSelectMenu::v18` -- the equip picker's own per-frame update. RVA `0x0009_2f00`.
///
/// Slot index 2 of [`FE_EQUIP_GROUP_VTABLE`]. The name differs from the Inventory tab's because
/// RTTI attributes it to a deeper base, but the SLOT is the same and the slot is what matters.
pub const FE_EQUIP_GROUP_UPDATE: u32 = 0x0009_2f00;

/// The five bytes [`FE_EQUIP_GROUP_UPDATE`] must begin with. `mov [rsp+0x20],rbx`.
pub const FE_EQUIP_GROUP_UPDATE_PROLOGUE: [u8; 5] = [0x48, 0x89, 0x5c, 0x24, 0x20];

/// Both updates take **four arguments**: `fn(this, f32 delta /*XMM1*/, ptr /*R8*/, ptr /*R9*/)`.
///
/// **`R8` is live and must be forwarded**, which is what separates these from the tab strip's
/// update that `ds2-menu-row` declares as `fn(this, f32)`. The Inventory tab's reads `[r8]` at
/// `0x1400bbec6` and the picker's reads `[r8+4]` at `0x140092f2a`, so a two-argument detour would
/// hand them whatever `R8` happened to hold after the detour's own prologue.
pub const FE_ITEM_LIST_UPDATE_ARGUMENT_COUNT: usize = 4;

// ---------------------------------------------------------------------------------------------
// THE WORLD: characters, where they are, and who among them is a person.
//
// Everything in this section was derived for `ds2-invasion-path` on 2026-09-22 and none of it
// came from Elden Ring. The anchors are MSVC RTTI class names, which this image carries 5271 of:
// `scripts/ds2-rtti-vtables.py` maps a class name to its vtable, and the vtable is what makes an
// object's class checkable at runtime rather than guessable.

/// Offset of `CharacterManager` in [`GAME_MANAGER_IMP`]. `+0x18`.
///
/// From the curated `GameManagerImp` type in the Ghidra project (field ordinal 3), cross-checked
/// against 447 call sites in the image that load `[0x1416148f0]` and immediately dereference
/// `+0x18`.
pub const GAME_MANAGER_CHARACTER_MANAGER_OFFSET: usize = 0x18;

/// `CharacterManager -> entity_list` **begin**. `+0x10`, a `CharacterCtrl**`.
///
/// The roster of every character the map currently holds -- the player, every NPC, and every
/// remote player in the session.
pub const CHARACTER_MANAGER_ENTITY_BEGIN_OFFSET: usize = 0x10;

/// `CharacterManager -> entity_list` **end**. `+0x18`, one past the last `CharacterCtrl*`.
///
/// **This is a begin/end pair, not a pointer and a count.** Element count is
/// `(end - begin) / 8`. Read out of three independent iteration sites rather than assumed from
/// the shape of the struct:
///
/// ```text
/// 0x14021c38b   mov rbx,[rsi+0x10]   cmp rbx,[rsi+0x18]   je ...   mov rdi,[rbx]
/// 0x140463a11   mov rdi,[rbx+0x18]   mov rbx,[rbx+0x10]   cmp rbx,rdi
/// 0x14048302a   mov rbx,[rdi+0x10]   mov rdi,[rdi+0x18]   cmp rbx,rdi
/// ```
///
/// Getting this wrong is not a compile error and not a crash: a `+0x18` read as a count would be
/// a pointer-sized number in the billions, and the sweep would walk off the end of the heap.
pub const CHARACTER_MANAGER_ENTITY_END_OFFSET: usize = 0x18;

/// `CharacterManager + 0x50` is an ARRAY BASE, and the Ghidra project's `player_ctrl` label on it
/// is **wrong**. Recorded so the next reader does not make the same mistake twice.
///
/// `0x14035b670` settles it in three instructions:
///
/// ```text
/// 0x14035b697   cmp  QWORD PTR [rcx+rax*8+0x50],0    ; is this slot free?
/// 0x14035b6b5   mov  eax,DWORD PTR [rdx+0x50]        ; index, read off the CHARACTER
/// 0x14035b6c5   mov  QWORD PTR [rcx+rax*8+0x50],rdx  ; register it in that slot
/// ```
///
/// A single `player_ctrl` pointer is not indexed by `rax*8`. What lives here is a registry of
/// characters keyed by a per-character index, and slot 0 holding the player in the ordinary case
/// is exactly what makes the mislabel survive: read it as a pointer and it usually answers with
/// something plausible.
///
/// **The local player is [`PLAYER_CTRL_OFFSET`] on `GameManagerImp`**, which is verified --
/// `PLAYER_PARAM_GET` (`0x1401ab660`) is eight instructions long and its first hop is that
/// offset. `ds2-invasion-path` uses that one. This constant exists only to be read by anyone
/// tempted by the field name, and nothing depends on it.
pub const CHARACTER_MANAGER_REGISTRY_OFFSET: usize = 0x50;

/// `PlayerCtrl`'s primary vtable. RVA `0x010e_4bb8`.
///
/// **This is how a player is told from an NPC**, and the reason it works is that DS2 builds
/// remote players out of the same class as the local one. Two factories allocate `0x4a0` bytes
/// and call the `PlayerCtrl` constructor at `0x14037ebe0`, and they differ only in the name they
/// give the result:
///
/// | factory | name it formats | what it builds |
/// | --- | --- | --- |
/// | `0x140357920` | `L"Player_%06u"` | the local player |
/// | `0x1403572e0` | `L"NetworkPlayer_%06u"` / `L"GhostPlayer_%06u"` | a remote player, or a bloodstain replay |
///
/// Every other character in the roster is a `CharacterCtrl` (constructor `0x1403114f0`, vtable
/// `0x010df218`) or a subclass of it that is not this one. So `*chr == base + this RVA` is an
/// exact membership test, not a guess about a type byte -- and unlike a `chr_type` field it
/// cannot be confused by a session kind nobody catalogued.
///
/// The bloodstain replay phantom shares the class, so it is a player by this test. It is also a
/// thing worth drawing a line to, so that is not treated as an error.
pub const PLAYER_CTRL_VTABLE: u32 = 0x010e_4bb8;

/// `CharacterCtrl`'s primary vtable. RVA `0x010d_f218`. Recorded for the roster census's log
/// line, which counts what it rejected by class rather than reporting only what it accepted.
pub const CHARACTER_CTRL_VTABLE: u32 = 0x010d_f218;

/// `CharacterCtrl -> position`. `+0x90`, four `f32` -- x, y, z, and a `w` nothing here reads.
///
/// Established through the accessor rather than by staring at floats: slot `+0x148` of
/// [`CHARACTER_CTRL_VTABLE`] is `0x140312b10`, whose entire body copies sixteen bytes from
/// `this+0x90`. [`PLAYER_CTRL_VTABLE`] inherits that slot unchanged, so one offset serves every
/// character in the roster.
///
/// **Y is up.** The engine's own navigation code treats component 1 as the height: the route
/// reader at `0x140bb5ff0` compares `node[1]` against `next[1]` to decide whether a step is a
/// descent.
///
/// That the field is a POSITION and not some other vector is its role at `0x14042fe80`: the
/// navmesh controller calls this slot, adds the character's velocity scaled by a time constant,
/// and hands the sum to the pathfinder as the agent's current world point. A velocity added to
/// it is only meaningful if it is where the character is.
///
/// `+0xA0` is a second `f32x4` behind slot `+0x150` (`0x140312a80`). It is **not identified**
/// and nothing here reads it. Recording that it exists is cheaper than the next reader
/// rediscovering it and assuming it is the position.
pub const CHARACTER_CTRL_POSITION_OFFSET: usize = 0x90;

// ---------------------------------------------------------------------------------------------
// THE NAVIGATION STACK.
//
// `docs/PORTING.md` filed `er-invasion-path` under "no DS2 analogue (Havok-AI navmesh)". Half of
// that is right and the conclusion is wrong. There is no Havok AI here -- the Havok in this image
// is 2013.2/2014.1 animation and physics -- but DS2 ships its OWN navigation stack, and it is
// RTTI-named throughout:
//
//   NvNavigationSystem 0x1411e2980   NvNaviGraphWorld 0x1411e28e8   NvRoutePlanner 0x1411e2c30
//   NvRouteNavigator   0x1411e2c60   NvNaviNodePathFindingTask 0x1411e2fd8
//   NvNaviPolyNearestSearchTask 0x1411e3038   ChrAiNavimeshCtrl 0x1410ede30
//
// The request/poll shape is the same one Elden Ring's `CSHkAiWorld` has. THE WHOLE CHAIN IS HERE
// NOW: the world-position-to-graph-id snap is below under THE SNAP, the planner factory and the
// tick that steps it under THE GAME TICK, and `ds2-invasion-path`'s `navquery` module calls them
// in that order. The arrow remains as the fallback for when the planner answers
// [`NV_ROUTE_PLANNER_FLAG_FAILED`], which is the same degraded mode the Elden Ring crate uses
// when its navmesh cannot answer.

/// Offset of `NvNavigationSystem` in [`GAME_MANAGER_IMP`]. `+0xBC0`.
///
/// Named in the Ghidra project's `GameManagerImp` type (field ordinal 382) and confirmed by every
/// one of the 21 `mov rcx,[rax+0xbc0]` sites in the image: each is preceded by a load of
/// `0x1416148f0` and followed by a call into the `0x140bad000..0x140bb8000` navigation module.
pub const GAME_MANAGER_NAV_SYSTEM_OFFSET: usize = 0xBC0;

/// `NvRoutePlanner -> route`. `+0x48` -- the finished polyline, once the flags say so.
///
/// From `0x14042ee40`, the navmesh controller's destination step: when the goal is unchanged and
/// [`NV_ROUTE_PLANNER_FLAGS_OFFSET`] reports a result, it passes `planner + 0x48` to
/// `0x140bb5cd0`, which binds it into the navigator at [`NV_ROUTE_NAVIGATOR_ROUTE_OFFSET`].
pub const NV_ROUTE_PLANNER_ROUTE_OFFSET: usize = 0x48;

/// `NvRoutePlanner -> flags`. `+0x30`, one byte.
///
/// The bits, all three read off the writers rather than inferred from behaviour:
///
/// | bit | set by | meaning |
/// | --- | --- | --- |
/// | `0x01` | `0x140bb4090` (request), `0x140bb40c0` | a search is pending |
/// | `0x02` | the search steps | a route is ready at [`NV_ROUTE_PLANNER_ROUTE_OFFSET`] |
/// | `0x04` | the search steps | the search finished without a route |
///
/// `0x140bb4090` writes `flags = (flags & 0xf1) | 1`, which is what makes "pending" mean pending:
/// requesting clears the two result bits in the same instruction that sets the request bit.
pub const NV_ROUTE_PLANNER_FLAGS_OFFSET: usize = 0x30;

/// [`NV_ROUTE_PLANNER_FLAGS_OFFSET`] bit meaning "a search is pending".
pub const NV_ROUTE_PLANNER_FLAG_PENDING: u8 = 0x01;

/// [`NV_ROUTE_PLANNER_FLAGS_OFFSET`] bit meaning "a route is ready".
pub const NV_ROUTE_PLANNER_FLAG_READY: u8 = 0x02;

/// [`NV_ROUTE_PLANNER_FLAGS_OFFSET`] bit meaning "the search answered, and the answer is no".
///
/// **TEST THIS BEFORE [`NV_ROUTE_PLANNER_FLAG_READY`], ALWAYS.** All three ways a search can end
/// without a route -- `0x140bb4310` (the endpoints are in different parts objects and no gate
/// joins them), `0x140bb4610` (same parts object, no node pair) and `0x140bb4a60` (an endpoint
/// id is [`NAVI_GRAPH_ID_NONE`]) -- finish with the same instruction:
///
/// ```text
/// or  byte ptr [planner + 0x30], 6
/// ```
///
/// which sets READY *and* FAILED in one write. A poller that asks "is READY set?" first therefore
/// believes every failure, and goes on to read `planner + 0x48` -- which those paths just left at
/// whatever it was, because the failure paths free the scratch arrays and never build a route.
/// The order is not a style preference; it is the difference between falling back to the arrow
/// and dereferencing a stale pointer in someone's invasion.
pub const NV_ROUTE_PLANNER_FLAG_FAILED: u8 = 0x04;

/// `NvRoutePlanner -> start`. `+0x34`, a packed navigation-graph id.
///
/// **THIS CONSTANT USED TO BE CALLED `NV_ROUTE_PLANNER_GOAL_OFFSET` AND HELD `0x34`, AND BOTH
/// HALVES OF THAT WERE WRONG TOGETHER.** `+0x34` is the START; the goal is at
/// [`NV_ROUTE_PLANNER_GOAL_OFFSET`], sixteen bits further along. The old name with the old value
/// is the one mistake that cannot be caught by a crash: a planner asked to route from the
/// destination to the player answers perfectly well, and the line it draws runs the right shape
/// in the wrong direction.
///
/// The correction is read off the engine's only caller. `0x14042ece0(ChrAiNavimeshCtrl*, const
/// float4* destination)` is "go to this world point", and it ends:
///
/// ```text
/// eax = 0x14042d1f0(ctrl)                                  ; the AGENT'S OWN node id
/// r8d = 0x14042c9a0(ctrl, destination, 10.0, 0x40, NULL)   ; the DESTINATION snapped
/// 0x14042ee40(ctrl, eax, r8d, 200.0)  ->  0x140bb4090(planner, eax, r8d, capability, 200.0)
/// ```
///
/// and `0x140bb4090` stores its second argument at `+0x34`. `0x14042d1f0` is unambiguous about
/// which is which: it caches its answer in the controller at `+0x78` and recomputes it by
/// snapping the character's own position through vtable slot `+0x148`. The agent's own node is
/// the start of the route the agent is about to walk.
///
/// # The id itself
///
/// `NvRoutePlanner::Update` (`0x140bb4110`, slot 2 of its vtable) feeds both ids to
/// `0x140bb2620(parts_table, id | 0x1ffff)` and dispatches on what comes back:
///
/// ```text
/// bits  0..14   index within the parts object
/// bits 15..16   kind   (0 = poly, 0x8000 = gate)
/// bits 17..29   the parts key the hash table is asked for
/// bits 30..31   a validity pair; both clear means usable
/// ```
///
/// [`NAVI_GRAPH_ID_NONE`] is the "no node here" answer and must never be requested.
pub const NV_ROUTE_PLANNER_START_OFFSET: usize = 0x34;

/// `NvRoutePlanner -> goal`. `+0x38`, a packed navigation-graph id, same encoding as
/// [`NV_ROUTE_PLANNER_START_OFFSET`].
///
/// Third argument of `0x140bb4090`, and the snapped DESTINATION at the only call site -- see
/// [`NV_ROUTE_PLANNER_START_OFFSET`] for the derivation, which establishes both offsets at once
/// because it reads one call and one store instruction per field.
///
/// `NvRoutePlanner::Update` reads this one second (`this[7]._vfptr` in the decompiler's numbering)
/// and falls into `0x140bb4a60` when either id is [`NAVI_GRAPH_ID_NONE`]. That function frees the
/// two scratch arrays and does `flags |= 6` -- so an unsnappable endpoint reports READY and
/// FAILED together, exactly like the two real failure paths, and a poller that tests FAILED
/// first needs no special case for it. See [`NV_ROUTE_PLANNER_FLAG_FAILED`].
pub const NV_ROUTE_PLANNER_GOAL_OFFSET: usize = 0x38;

/// `NvRoutePlanner -> capability`. `+0x3c`, the movement-capability mask the search filters edges
/// with.
///
/// Fourth argument of `0x140bb4090`. The AI builds it in `0x14042ed50`: a size class in bits 0..2
/// from the switch on `[[[ctrl+8]+0x38]+0x40]+4`, then ten feature bits (`0x300`, `0x40`, `0x80`,
/// `0x400`, `0x20`, `0x10`, `0x2000`, `0x4000`, `0x8`) each set from a boolean the
/// `ChrAiNavimeshCtrl` carries. `0x140bb4310` hands it to the parts object's vtable slot `+0x28`
/// to turn it into a cost-table index.
///
/// See [`NV_ROUTE_PLANNER_CAPABILITY_DEFAULT`] for what a caller without a `ChrAiNavimeshCtrl`
/// can honestly pass.
pub const NV_ROUTE_PLANNER_CAPABILITY_OFFSET: usize = 0x3c;

/// The capability a caller with no `ChrAiNavimeshCtrl` may pass. `0`.
///
/// **Not a guess and not a placeholder: it is a value `0x14042ed50` itself returns.** That
/// function's `switch` has a `default: uVar1 = 0` arm, taken for every size-class enum outside
/// `2..=7`, and every one of its ten feature bits is off when the corresponding controller
/// boolean is clear. So `0` is the mask the engine produces for a small agent with no special
/// movement, and the search treats it as such rather than as a sentinel.
///
/// The local player has no `ChrAiNavimeshCtrl` to read a truer mask from -- that object belongs
/// to the AI module, and a player is not driven by it. Inventing one to read ten booleans out of
/// is exactly the fabrication bd `ds2-call-the-games-own-functions` forbids.
pub const NV_ROUTE_PLANNER_CAPABILITY_DEFAULT: u32 = 0;

/// `NvRoutePlanner -> max cost`. `+0x40`, `f32`, the search's budget.
///
/// Fifth argument of `0x140bb4090`, arriving on the stack: the callee reads `[rsp+0x28]`, which
/// is the fifth-argument slot once the `call` has pushed a return address.
pub const NV_ROUTE_PLANNER_MAX_COST_OFFSET: usize = 0x40;

/// The larger of the two budgets the engine itself passes. `999.0`, at `0x1410cb67c`.
///
/// `0x14042e890` -- the controller step that takes a node id, decodes it to a world triangle and
/// routes there -- passes this. `0x14042ece0`, the short pursuit step, passes `200.0` from
/// `0x1410b0d40` instead.
///
/// `ds2-invasion-path` uses this one because its target is another player somewhere on the map
/// rather than an AI's next few metres, and a budget that is too small does not fail loudly: the
/// planner simply reports [`NV_ROUTE_PLANNER_FLAG_FAILED`] and the overlay falls back to the
/// arrow, which looks exactly like "there is no way to walk there".
pub const NV_ROUTE_MAX_COST_LONG_RANGE: f32 = 999.0;

/// The budget the AI's short pursuit step passes. `200.0`, at `0x1410b0d40`. Recorded so the
/// choice of [`NV_ROUTE_MAX_COST_LONG_RANGE`] is visible as a choice between two engine values.
pub const NV_ROUTE_MAX_COST_PURSUIT: f32 = 200.0;

/// `NvRouteNavigator -> route`. `+0x28`.
///
/// `0x140bb5cd0` copies a finished route here and then sets the cursor at `+0x64` to
/// `(segment_count - 1) << 16`, which is what establishes both the index packing in
/// [`NV_ROUTE_SEGMENT_STRIDE`]'s documentation and the fact that a route is stored goal-first and
/// walked backwards.
pub const NV_ROUTE_NAVIGATOR_ROUTE_OFFSET: usize = 0x28;

/// `NvRoute -> segments`. `+0x10`, an array of [`NV_ROUTE_SEGMENT_STRIDE`]-byte records.
pub const NV_ROUTE_SEGMENTS_OFFSET: usize = 0x10;

/// `NvRoute -> segment count`. `+0x18`, `i32`.
pub const NV_ROUTE_SEGMENT_COUNT_OFFSET: usize = 0x18;

/// Bytes per route segment. `0x60`.
///
/// The whole layout below comes from `0x140bb3bd0`, the engine's own "position of route node N",
/// and from `0x140bb3b20`, its "flags of route node N". Both index the same way, and the way is
/// worth spelling out because it is not the obvious one:
///
/// - A node index is **packed**: `(segment << 16) | point`, and `0xFFFFFFFF` means "the last
///   segment, point 0".
/// - Points within a segment are stored **in reverse**: node `point` is
///   `points[count - 1 - point]`.
/// - A segment whose point count is `<= 0` contributes exactly one node, its own
///   [`NV_ROUTE_SEGMENT_POINT_OFFSET`].
pub const NV_ROUTE_SEGMENT_STRIDE: usize = 0x60;

/// A segment's own point. `+0x20` within the segment, four `f32`.
///
/// Used when [`NV_ROUTE_SEGMENT_POINT_COUNT_OFFSET`] is not positive -- a segment with no
/// expanded polyline still has a position.
pub const NV_ROUTE_SEGMENT_POINT_OFFSET: usize = 0x20;

/// A segment's expanded polyline. `+0x40`, a pointer to 16-byte points.
pub const NV_ROUTE_SEGMENT_POINTS_OFFSET: usize = 0x40;

/// A segment's per-point flags. `+0x48`, a pointer to `u32`, parallel to
/// [`NV_ROUTE_SEGMENT_POINTS_OFFSET`] and indexed the same reversed way.
pub const NV_ROUTE_SEGMENT_FLAGS_OFFSET: usize = 0x48;

/// How many points a segment's polyline holds. `+0x50`, `i16`.
///
/// Signed, and the engine tests `0 < count` before trusting the pointers -- so a non-positive
/// value is the documented "this segment has no polyline" case rather than an impossibility.
pub const NV_ROUTE_SEGMENT_POINT_COUNT_OFFSET: usize = 0x50;

// ---------------------------------------------------------------------------------------------
// THE SNAP: WORLD POSITION -> NAVIGATION-GRAPH ID.
//
// bd `ds2-mods-rs-4yd` was filed on the premise that this step is an asynchronous job
// (`NvNaviPolyNearestSearchTask`) and that driving it by hand would mean fabricating an engine
// update context. THAT PREMISE IS FALSE. The snap is an ordinary synchronous call returning the
// id in `eax`, and the whole of it is spelled out in one function -- `0x14037be30`, twenty-eight
// instructions -- which does nothing else:
//
//   mov  rbx,[0x1416148f0]                     ; GameManagerImp
//   mov  rax,[rbx+0x38]                        ; MapManager
//   mov  ecx,[rax+0x170]                       ; the map area
//   call 0x140bab1f0                           ; -> key
//   mov  rcx,rbx ; call 0x14039a9f0            ; -> NvNaviGraphWorld ([[gm+0xBC0]+0x10])
//   mov  edx,esi ; call 0x140badb90            ; -> the graph data for that area
//   call [vtable+0x148]                        ; -> a pointer to the character's position
//   mov  r9d,0x28 ; movss xmm2,[0x1410ad5ec]   ; filter, radius 20.0
//   mov  qword [rsp+0x20],0                    ; out_dist: NULL is allowed
//   call 0x140babf90                           ; -> the graph id, or 0xffffffff
//
// `0x14042c9a0(ChrAiNavimeshCtrl*, pos, radius, filter, out_dist)` is the same chain packaged for
// an AI agent; it takes the map area from the character instead of from `MapManager` and is not
// usable by a caller who only has a position.
//
// Every address in this section was checked with `scripts/ds2-arxan-chain.py` and none is
// redirected; the four leaf functions report `UNKNOWN` only because they begin with their real
// work rather than with a standard prologue, and their first bytes match the disassembly above.

/// Offset of `MapManager` in [`GAME_MANAGER_IMP`]. `+0x38`.
///
/// From `0x14037be8f`: `mov rax,[rbx+0x38]` on the `GameManagerImp` just loaded from
/// `0x1416148f0`, immediately dereferenced at [`MAP_MANAGER_AREA_OFFSET`] and handed to
/// [`NAVI_GRAPH_KEY_FROM_AREA`]. Named `MapMan` in the Ghidra project's `GameManagerImp` type.
pub const GAME_MANAGER_MAP_MANAGER_OFFSET: usize = 0x38;

/// `MapManager -> current area`. `+0x170`, read as a `u32` and masked to six bits downstream.
///
/// From `0x14037be98`: `mov ecx,[rax+0x170]` is the only thing done with the `MapManager` before
/// [`NAVI_GRAPH_KEY_FROM_AREA`] is called on it.
pub const MAP_MANAGER_AREA_OFFSET: usize = 0x170;

/// `u32 area -> u32 key`. RVA `0x00ba_b1f0`, five instructions, no memory access:
/// `(area & 0x3f) << 24 | 0xffffff`.
///
/// The result is what [`NAVI_GRAPH_DATA_FOR_KEY`] compares against `[[data+0x28]+0x1c]`. Two
/// things worth knowing before trusting a `-1` check on it: it CANNOT return `-1` (the largest
/// value it can produce is `0x3fffffff`), and [`NAVI_GRAPH_DATA_FOR_KEY`] re-applies the same
/// mask itself, so the key form is a convention rather than a requirement.
pub const NAVI_GRAPH_KEY_FROM_AREA: u32 = 0x00ba_b1f0;

/// `GameManagerImp* -> NvNaviGraphWorld*`. RVA `0x0039_a9f0`.
///
/// Three instructions: returns `[[gm + 0xBC0] + 0x10]`, or `0` when
/// [`GAME_MANAGER_NAV_SYSTEM_OFFSET`] is null. **It does not null-check its own argument**, so
/// the caller must, which is why `ds2-invasion-path` reads the `GameManagerImp` pointer with a
/// fault-safe read and refuses on null before getting here.
pub const NAVI_GRAPH_WORLD_FROM_GAME_MANAGER: u32 = 0x0039_a9f0;

/// `(NvNaviGraphWorld*, u32 key) -> graph data*`. RVA `0x00ba_db90`.
///
/// A linear scan of the pointer array at `world + 0x28`, `[world + 0x68]` entries long, keeping
/// the one whose `[[entry + 0x28] + 0x1c]` equals `key & 0x3f000000 | 0xffffff`. Returns `0`
/// when the area is not loaded -- which is the ordinary answer during a map transition, not a
/// failure.
pub const NAVI_GRAPH_DATA_FOR_KEY: u32 = 0x00ba_db90;

/// `(graph data*, const float* pos, f32 radius, u32 filter, f32* out_dist) -> u32 id`.
/// RVA `0x00ba_bf90`.
///
/// `rcx`, `rdx`, `xmm2`, `r9d`, `[rsp+0x20]`. Walks the sub-graphs at `data + 0x28`
/// (`[[data+0x28]+8]` of them), calling `0x140bac070` on each and keeping the nearest hit;
/// returns [`NAVI_GRAPH_ID_NONE`] when none is within `radius`.
///
/// `out_dist` may be `NULL` -- the function tests it -- and `0x14037bef8` passes exactly that.
/// When it is not null it receives the SQUARED distance, not the distance: the `sqrtf` in the
/// loop narrows the search radius and is never written back.
///
/// # This must run on the game thread
///
/// `0x140bac070` lazy-initialises a `DLKR` allocator through `DAT_1416681a8` on first use and
/// then panics `"Tried to create container with incompatible heap."` if the container it builds
/// does not match. Two threads racing that initialisation is a torn global followed by an abort,
/// and the abort is inside the game's own panic handler rather than anywhere this code could
/// catch it.
pub const NAVI_GRAPH_NEAREST_ID: u32 = 0x00ba_bf90;

/// The radius `0x14037be30` snaps with. `20.0`, read from `0x1410ad5ec`.
///
/// The `ChrAiNavimeshCtrl` paths use `10.0` (`0x1410ad5e8`) instead. The larger one is used here
/// because a player standing on a ledge, a staircase or a corpse can be further off the navmesh
/// than a walking AI ever is, and a snap that fails costs the whole route.
pub const NAVI_GRAPH_SNAP_RADIUS_METERS: f32 = 20.0;

/// The filter `0x14037be30` snaps with. `0x28`.
///
/// The bits are read off `0x140bac070`, which is the only consumer:
///
/// | bit | effect when SET |
/// | --- | --- |
/// | `0x07` | minimum poly class; a poly passes only when `poly & 7` is strictly greater |
/// | `0x08` | accept polys carrying flag `0x100`, which are otherwise rejected |
/// | `0x10` | do not reject polys whose `class & 0x78` is `0x40` |
/// | `0x20` | likewise |
///
/// So `0x28` is `0x20 | 0x08` with a class minimum of zero: the most permissive snap the engine
/// offers. That is the right one for a player, who is not an AI agent with a movement class and
/// should be placed on whatever navmesh is under their feet.
pub const NAVI_GRAPH_SNAP_FILTER: u32 = 0x28;

/// The "there is no node here" graph id. `0xffffffff`.
///
/// Returned by [`NAVI_GRAPH_NEAREST_ID`] when nothing is in range, and by `0x14042c9a0` when the
/// area is not loaded. Requesting a route with it is safe but pointless -- see
/// [`NV_ROUTE_PLANNER_GOAL_OFFSET`], which explains why it comes back as a FAILED search rather
/// than as silence.
pub const NAVI_GRAPH_ID_NONE: u32 = 0xffff_ffff;

// ---------------------------------------------------------------------------------------------
// THE GAME TICK, and why it is this function rather than one of its callers.
//
// `ds2-invasion-path` needs a seam that runs on the thread the game's own logic runs on, because
// both things it wants to do from one -- the snap above and the SFX spawn below -- read and
// write unlocked engine state. `Present` is not that seam (`ds2-mods-rs-3al`, blocker (a)).
//
// `NvNavigationSystem::Update` is called from four game-state tick handlers:
// `0x1401bd750` (state 0x0a), `0x1401bf3b3` (state 0x1c), `0x1401bfe46` and `0x1401c2454`, each
// as one line in a long sequence of `if (subsystem) subsystem->Update(delta)`. Hooking the
// callee rather than the callers means ONE detour instead of four, and it arrives with the
// `NvNavigationSystem` already in `rcx` and already known non-null -- every caller tests it
// first. It also cannot run at the title screen or during a load, which is exactly the window in
// which neither the navmesh nor the SFX system exists.

/// `NvNavigationSystem::Update(NvNavigationSystem* rcx, f32 delta /*xmm1*/)`. RVA `0x00ba_eb20`.
///
/// **The delta is a float in `xmm1`, not an integer in `edx`.** Every call site loads it with
/// `movaps xmm1,xmm6`. A detour declared with an integer second parameter would compile, run,
/// and silently hand the engine whatever `xmm1` happened to hold -- which is also why this must
/// NOT go through `ds2-hook`'s union, whose shared signature is four `usize` and whose dispatcher
/// is free to clobber the volatile `xmm1`.
///
/// What it does, in order: builds a vector of the live objects on the intrusive list at
/// [`NV_NAVIGATION_SYSTEM_LIST_HEAD_OFFSET`], unlinking and releasing any whose
/// [`NV_NAVIGATION_NODE_RETIRED_OFFSET`] byte is set; calls each survivor's vtable slot `+0x10`
/// with the shared context at [`NV_NAVIGATION_SYSTEM_CONTEXT_OFFSET`], repeating until they all
/// return true or 0x80 passes have gone by; then forwards the delta to the graph world.
pub const NV_NAVIGATION_SYSTEM_UPDATE: u32 = 0x00ba_eb20;

/// First thirteen bytes of [`NV_NAVIGATION_SYSTEM_UPDATE`], checked before the detour goes in.
///
/// `mov [rsp+8],rcx ; push rbp ; push r12 ; push r13 ; mov rbp,rsp`. The first instruction alone
/// is the five bytes MinHook needs and is position-independent, so the trampoline is a plain
/// copy; the rest is carried because `48 89 4c 24 08` on its own is one of the most common
/// sequences in the image and would match almost anything if the RVA ever drifted.
pub const NV_NAVIGATION_SYSTEM_UPDATE_PROLOGUE: [u8; 13] = [
    0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x41, 0x54, 0x41, 0x55, 0x48, 0x8b, 0xec,
];

/// `NvNavigationSystem -> NvNaviGraphWorld`. `+0x10`.
///
/// The same pointer [`NAVI_GRAPH_WORLD_FROM_GAME_MANAGER`] returns, reached the other way.
/// `0x140bae8d0` passes it to `0x140bb4080` to bind a fresh planner to the world, and
/// `NvNavigationSystem::Update` forwards the frame delta to its vtable slot `+0x18`.
pub const NV_NAVIGATION_SYSTEM_GRAPH_WORLD_OFFSET: usize = 0x10;

/// `NvNavigationSystem -> search context`. `+0x18`.
///
/// The second argument every listed object's `Update` receives. `NvRoutePlanner::Update` passes
/// it to `0x140bb9610` to allocate search-task nodes, so this is the pool the A* runs out of.
/// Recorded because it is the "update context the engine supplies" that bd
/// `ds2-call-the-games-own-functions` warns against fabricating -- and the point of linking a
/// planner into the list is that the engine supplies it for you.
pub const NV_NAVIGATION_SYSTEM_CONTEXT_OFFSET: usize = 0x18;

/// `NvNavigationSystem -> head of the intrusive update list`. `+0x20`.
///
/// `0x140bae8d0` pushes onto the front: `new->next = head; count += 1; head = new`.
pub const NV_NAVIGATION_SYSTEM_LIST_HEAD_OFFSET: usize = 0x20;

/// `NvNavigationSystem -> length of that list`. `+0x28`, `i32`.
///
/// Incremented by `0x140bae8d0` and decremented by `NvNavigationSystem::Update` as it unlinks a
/// retired node. Read by `ds2-invasion-path` for one thing only: proving a planner it created
/// was actually linked.
pub const NV_NAVIGATION_SYSTEM_LIST_COUNT_OFFSET: usize = 0x28;

/// The `next` pointer of a listed navigation object. `+0x10`.
///
/// `NvNavigationSystem::Update` walks `puVar12[2]`; `0x140bae8d0` writes `plVar3[2]`. Same field.
pub const NV_NAVIGATION_NODE_NEXT_OFFSET: usize = 0x10;

/// "Take me off the list." `+0x18`, one byte.
///
/// `0x140baefc0(navsys, obj)` is four instructions -- `mov byte [rdx+0x18],1 ; ret` -- and does
/// not touch the list. The unlink, the reference release and the destructor all happen on the
/// next `NvNavigationSystem::Update`, which is what makes teardown safe from anywhere the tick
/// is not currently running.
pub const NV_NAVIGATION_NODE_RETIRED_OFFSET: usize = 0x18;

/// `NvNavigationSystem* -> NvRoutePlanner*`. RVA `0x00ba_e8d0`. The whole factory in one call.
///
/// It allocates `0xa0` bytes from the allocator at `navsys + 0x40`, runs the constructor
/// `0x140bb3cf0`, calls `0x140bb4080(planner, navsys->graph_world)` to fill the `+0x28` the
/// constructor leaves null, and links the result onto
/// [`NV_NAVIGATION_SYSTEM_LIST_HEAD_OFFSET`]. Returns `0` if the allocation fails.
///
/// **That last step is the entire reason to use it.** A planner on that list is stepped by
/// `NvNavigationSystem::Update` for free, with the context the engine supplies, every frame,
/// without this crate scheduling anything. The alternative -- constructing one privately and
/// calling its `Update` by hand -- is the fabricated context bd `ds2-call-the-games-own-functions`
/// forbids.
pub const NV_NAVIGATION_SYSTEM_CREATE_ROUTE_PLANNER: u32 = 0x00ba_e8d0;

/// `(NvNavigationSystem*, object*)`. RVA `0x00ba_efc0`. Marks a listed object for removal.
///
/// See [`NV_NAVIGATION_NODE_RETIRED_OFFSET`]: it sets one byte and returns. The object stays
/// valid until the next tick unlinks it.
pub const NV_NAVIGATION_SYSTEM_RETIRE: u32 = 0x00ba_efc0;

/// `(NvRoutePlanner*, u32 start, u32 goal, u32 capability, f32 max_cost)`. RVA `0x00bb_4090`.
///
/// Eight instructions, no allocation, no call: it stores the four values at
/// [`NV_ROUTE_PLANNER_START_OFFSET`], [`NV_ROUTE_PLANNER_GOAL_OFFSET`],
/// [`NV_ROUTE_PLANNER_CAPABILITY_OFFSET`] and [`NV_ROUTE_PLANNER_MAX_COST_OFFSET`], then does
/// `flags = (flags & 0xf1) | 1` at [`NV_ROUTE_PLANNER_FLAGS_OFFSET`] -- clearing both result bits
/// in the same instruction that sets the request bit, which is what makes "pending" mean pending.
///
/// The search itself happens on the planner's `Update`, so this is safe to call from the tick
/// detour before or after the original; it is NOT safe from another thread, because the planner
/// it writes is on a list the tick is walking.
pub const NV_ROUTE_PLANNER_REQUEST: u32 = 0x00bb_4090;

// ---------------------------------------------------------------------------------------------
// THE SFX SPAWN, for the Prism Stone trail. bd `ds2-mods-rs-3al` has the full account; what is
// here is the part `ds2-invasion-path` calls.

/// Offset of `KatanaSfxSystem` in [`GAME_MANAGER_IMP`]. `+0xbc8`.
///
/// From `0x140446f08`: `mov rbx,[rax+0xbc8]` on the pointer loaded from `0x1416148f0`, named
/// `KatanaSfxSystem` in the Ghidra project's `GameManagerImp` type.
pub const GAME_MANAGER_SFX_SYSTEM_OFFSET: usize = 0xbc8;

/// `KatanaSfxSystem -> ready`. `+0x46`, one byte.
///
/// From `0x140446f18`: `cmp byte [rbx+0x46],0 ; je <return>` -- the fire-and-forget spawner
/// refuses to do anything when it is zero, before touching any other field. Treated the same way
/// here.
pub const KATANA_SFX_SYSTEM_READY_OFFSET: usize = 0x46;

/// `KatanaSfxSystem -> quality level`. `+0x300`, `u32`.
///
/// **A value of 3 or more silently drops spawns.** `0x140beb670` opens with
/// `if (3 <= sys->quality && sys->vtable[0x50](sys, id)) return empty_ctrl;` -- the caller gets a
/// control block that looks exactly like a successful one and no effect appears. The engine
/// raises this under load, so a trail that works in a corridor can stop working in a boss arena
/// for reasons that are not in this crate. `ds2-invasion-path` logs the byte on the first spawn
/// of a session precisely so that case is diagnosable without a debugger.
pub const KATANA_SFX_SYSTEM_QUALITY_OFFSET: usize = 0x300;

/// The quality level at which spawns start being dropped. `3`.
pub const KATANA_SFX_QUALITY_DROP_THRESHOLD: u32 = 3;

/// `KatanaSfxSystem -> high-quality variants enabled`. `+0x305`, one byte.
///
/// When non-zero, `0x140beb670` tries `id + 20000` before `id` and then `id + 10000` -- the SOTFS
/// remaster's higher-detail effects. Which is why a caller passes the BASE id and not a variant.
pub const KATANA_SFX_SYSTEM_HIGH_QUALITY_OFFSET: usize = 0x305;

/// The largest id the `+20000` remaster lookup is attempted for. `9999`.
///
/// `0x140beb670` tests `if (9999 < id)` and skips straight to a direct lookup. Base ids are all
/// below it; `ds2-mods-rs-3al` catalogues the usable range as `40..8557`, which is a survey of
/// where ids appear in the game's params rather than a bound this executable enforces.
pub const KATANA_SFX_BASE_ID_MAX: u32 = 9999;

/// `(KatanaSfxSystem*, ctrl* out, u32 sfx_id, const f32[8]* pos_and_dir, u32, u8, u8) -> ctrl*`.
/// RVA `0x00be_b670`.
///
/// The core spawn. `0x140beb590` is the same thing behind a `float4x4`: it copies row 3 of the
/// matrix as the position, derives a direction from the matrix with `0x140005c00`, normalises it,
/// and calls this. Taking the eight floats directly removes a matrix layout from the things that
/// can be wrong -- the first four are the world position, the second four a unit direction.
///
/// The trailing three arguments are `0`, `0xff`, `0` at every call site, `0x140446ee0` included.
///
/// `out` must point at [`KATANA_SFX_CTRL_BYTES`] of zeroed, 16-byte-aligned storage the caller
/// owns; the function constructs two `FX4CG::FXCGSfxCtrl` sub-objects in it and returns it.
///
/// **Game thread only.** The create path underneath reads and writes
/// [`KATANA_SFX_SYSTEM_QUALITY_OFFSET`], two live vectors, an xorshift RNG and the failed-lookup
/// red-black tree at `sys + 0x2b8`, with no lock anywhere, and all 46 static call sites are game
/// logic.
pub const KATANA_SFX_SPAWN: u32 = 0x00be_b670;

/// `(ctrl*)`. RVA `0x00a0_60f0`. The `FXCGSfxCtrl` destructor.
///
/// **It is called on each 0x30-byte HALF of the control block, not on the block.** `0x140446ee0`
/// tears its block down with two calls, at `+0x30` and then at `+0x00`; so does `0x140beb670`
/// for its own temporaries. Calling it once on the whole thing leaves the other half linked.
///
/// What it actually does: restores the `FFX::FXSfxCtrl` vtable pointer at `+0x00` and unlinks the
/// block from the effect node's controller list -- the head is `node + 0xf8`, the links are
/// [`KATANA_SFX_CTRL_PREV_OFFSET`] and [`KATANA_SFX_CTRL_NEXT_OFFSET`]. It reads
/// [`KATANA_SFX_CTRL_NODE_OFFSET`] and `+0x18` to find the node and does nothing when both are
/// zero.
///
/// **It unlinks. It does not stop the effect.** [`KATANA_SFX_STOP`] is the stop, and it comes
/// first. Skipping this afterwards is not a leak but a dangling write: a block still on the
/// node's list is memory the engine will write through later.
pub const KATANA_SFX_CTRL_DESTROY: u32 = 0x00a0_60f0;

/// `(ctrl* block_of_two)`. RVA `0x0014_1530`. **Stops a spawned effect.**
///
/// bd `ds2-mods-rs-3al` blocker (c) said this was unknown and that every marker placed would
/// therefore be permanent for the session. It is not unknown. Twenty-two instructions, a clean
/// prologue, not Arxan-redirected, and it takes the whole [`KATANA_SFX_CTRL_BYTES`] block --
/// looping twice over the two [`KATANA_SFX_CTRL_HALF_BYTES`] halves and issuing the same three
/// calls on each:
///
/// ```text
/// 0x140a34eb0(half, 0)      ; detach: nulls the effect's follow-transform source
/// 0x140a069f0(half, 1, 0)   ; set flag bit 0 on the node and its whole child subtree
/// 0x140a06350(half)         ; the despawn -- returns the node to its pool, clears the
///                           ; "alive" bit 30 of node+0x58, and zeroes ctrl+8..+0x28
/// ```
///
/// All three are Arxan thunks; the chains were walked with `scripts/ds2-arxan-chain.py` and end
/// in game code. `0x140141530` itself is not redirected. It is issued as one unit at all sixteen
/// call sites -- `0x1403c8360`, a distance-culled emitter, pairs it against `0x140beb590`
/// exactly the way `ds2-invasion-path` does.
///
/// **Order: stop, then [`KATANA_SFX_CTRL_DESTROY`].** Not because the reverse fails today -- the
/// destructor does not clear [`KATANA_SFX_CTRL_NODE_OFFSET`], so a stop after it would still
/// find the node -- but because the destructor is the only thing that takes the block off the
/// node's list, and between the two calls that node may be recycled into somebody else's effect.
/// Stopping through a stale pointer kills the wrong thing.
///
/// **Game thread only.** It takes no lock and touches the FX manager's per-bucket lists, its
/// active-node list, its pool arrays and its deferred-destroy chain -- all of which the FX update
/// pass walks every frame. A search of these bodies for `lock`, `cmpxchg`, `xchg [mem]` and any
/// wait call finds nothing.
///
/// Safe on a block that never spawned anything: each of the three primitives returns immediately
/// when the node pointers are null, which is what a throttled spawn leaves behind.
pub const KATANA_SFX_STOP: u32 = 0x0014_1530;

/// `FXSfxCtrl -> effect node`. `+0x10`, and `+0x18` for the second one.
///
/// Read by [`KATANA_SFX_CTRL_DESTROY`] as `param_1[2]` and `param_1[3]` and by every primitive
/// [`KATANA_SFX_STOP`] calls. **Both zero means the block controls nothing**, which is what a
/// spawn dropped by [`KATANA_SFX_SYSTEM_QUALITY_OFFSET`] produces -- `0x140beb670` returns a
/// block built by `0x140127240`, which constructs both halves empty and looks exactly like a
/// success to its caller. Testing this is the only way to tell them apart.
pub const KATANA_SFX_CTRL_NODE_OFFSET: usize = 0x10;

/// `FXSfxCtrl -> previous controller` in the effect node's list. `+0x20`.
pub const KATANA_SFX_CTRL_PREV_OFFSET: usize = 0x20;

/// `FXSfxCtrl -> next controller` in the effect node's list. `+0x28`.
pub const KATANA_SFX_CTRL_NEXT_OFFSET: usize = 0x28;

/// `FXSfxCtrl -> alive`. Bit 30 (`0x4000_0000`) of the word at **`node + 0x58`**, where `node` is
/// [`KATANA_SFX_CTRL_NODE_OFFSET`]. Set means the effect is still playing.
///
/// **This is the only honest answer to "is it still there?"** -- and it is what separates an
/// effect that LINGERS from one that flashed once, which is the property a marker trail needs and
/// which nothing in this workspace had established.
///
/// Read off `0x140a067c0`, an ordinary handle method, whose first act is:
///
/// ```text
/// node = ctrl->[0x10];
/// if (node && (~(*(u32*)(node + 0x58) >> 30) & 1)) {
///     ctrl->[8] = ctrl->[0x10] = ctrl->[0x18] = ctrl->[0x20] = ctrl->[0x28] = 0;
///     return;                       // the handle has just disowned a dead effect
/// }
/// ```
///
/// So every handle method self-nulls on a clear bit, which is why a dangling handle is not
/// possible here and why reading the bit directly is safe: the worst case is that the engine
/// has already zeroed [`KATANA_SFX_CTRL_NODE_OFFSET`] and there is nothing to read.
///
/// `0x140141530`'s third primitive clears it, which is what makes the stop observable.
pub const KATANA_SFX_NODE_ALIVE_OFFSET: usize = 0x58;

/// The bit at [`KATANA_SFX_NODE_ALIVE_OFFSET`] that means "still playing". `0x4000_0000`.
pub const KATANA_SFX_NODE_ALIVE_BIT: u32 = 0x4000_0000;

/// `KatanaSfxSystem -> ids that did not resolve`. `+0x2b8`, an MSVC `std::map`-shaped red-black
/// tree keyed by `u32`.
///
/// **Membership is the difference between "not in this map" and "spawned and invisible"**, which
/// is otherwise the same absence on the ground and the most expensive confusion in the whole
/// feature to resolve by looking.
///
/// `0x140beb400(sys, id)` is the insert, and reading it gives the entire layout. It is a
/// `lower_bound` descent followed by the usual found-test, so a read-only `contains` is the same
/// walk with the insert arm removed:
///
/// ```text
/// head = *(usize*)(sys + 0x2b8);         // _Myhead; its +0x08 is the ROOT, not a node
/// node = head->[KATANA_SFX_MISSING_PARENT_OFFSET];
/// while (!node->_Isnil) { node = (node->key < id) ? node->_Right : node->_Left; ... }
/// ```
///
/// A hit bumps a counter at `+0x20` and clears a byte at `+0x24` rather than inserting again, so
/// the tree records how many times each id was asked for.
pub const KATANA_SFX_MISSING_IDS_OFFSET: usize = 0x2b8;

/// `_Left` of a node in the [`KATANA_SFX_MISSING_IDS_OFFSET`] tree. `+0x00`.
pub const KATANA_SFX_MISSING_LEFT_OFFSET: usize = 0x00;

/// `_Parent` of a node -- and, on the head node, the tree's ROOT. `+0x08`.
pub const KATANA_SFX_MISSING_PARENT_OFFSET: usize = 0x08;

/// `_Right` of a node. `+0x10`.
pub const KATANA_SFX_MISSING_RIGHT_OFFSET: usize = 0x10;

/// `_Isnil` -- non-zero on the head and on the leaf sentinels. `+0x19`.
///
/// `+0x18` is `_Color`, which nothing here reads. The pair is the standard MSVC `_Tree_node`
/// prefix, which is why the key lands at `+0x1c` rather than at `+0x1a`.
pub const KATANA_SFX_MISSING_ISNIL_OFFSET: usize = 0x19;

/// The `u32` id a node holds. `+0x1c`.
pub const KATANA_SFX_MISSING_KEY_OFFSET: usize = 0x1c;

/// Depth at which a walk of the [`KATANA_SFX_MISSING_IDS_OFFSET`] tree gives up. `64`.
///
/// A red-black tree of `n` keys is at most `2*log2(n+1)` deep, so sixty-four admits about a
/// billion entries -- far past anything real. It is there because the tree is read while the
/// game is free to rebalance it, and a torn read must end the walk rather than spin inside a
/// cycle on the game's own simulation thread.
pub const KATANA_SFX_MISSING_MAX_DEPTH: usize = 64;

/// The engine's own "did this control block get anything?" predicate. RVA `0x00a0_6580`.
///
/// **Recorded so nobody calls it.** `ds2-invasion-path` tests the same thing by reading two
/// fields, and this constant exists to document that the two are equivalent rather than to be
/// used.
///
/// It is a five-byte `jmp` into Arxan-shattered code, and walking the chain with
/// `scripts/ds2-arxan-chain.py` shows the whole predicate:
///
/// ```text
/// 0x140a06580  jmp 0x141b873fb
/// 0x141b873fb  cmp qword [rcx+0x10], 0     ; KATANA_SFX_CTRL_NODE_OFFSET
/// 0x141c5abbd  cmovne rbx, <other return>  ; the flags pick which address to return to
/// 0x141b692ad  cmp qword [rcx+0x18], 0     ; the second node slot
/// ```
///
/// Two null tests on fields any caller can read for itself, reached through three stack-swapping
/// Arxan fragments. Calling it would mean a hand-written prototype over shattered code to learn
/// something two `safe_read_usize` calls already say.
pub const KATANA_SFX_CTRL_IS_EMPTY: u32 = 0x00a0_6580;

/// The Prism Stone's seven SFX ids: `833 ..= 839`.
///
/// **This is what `marker_effect_id` wants.** bd `ds2-mods-rs-3al` recorded the id as
/// unobtainable from the executable; the chain that yields it is `ItemParam` row `60450000` ->
/// `SpEffectActiveItem.emevd` event `60450000` -> instruction bank `100120` index `2`, whose
/// entry in the dispatch table at `0x14156fe10` is named `L"七色石発射"` -- "seven-colour-stone
/// launch" -- and whose factory `0x140216010` installs the vtable of
/// `.?AVSpEffectActionImpl_ThrowColorStone@@`.
///
/// That class's execute method (`0x140216c40`) draws `rng % 7` from `GameManagerImp`'s xorshift
/// state into the item bag's colour byte, and the item-pack glow spawner reads it back at
/// `0x1401e69b4`:
///
/// ```text
/// movzx eax, byte ptr [rbp+0x39]              ; the colour, 0..6
/// lea   rcx, [rip + ...]                      ; the image base
/// mov   eax, dword ptr [rcx + rax*4 + 0x10c7b58]
/// ```
///
/// and the seven dwords at `0x1410c7b58` are `833 834 835 836 837 838 839`, followed by
/// `0x3dcccccd` (`0.1f`), so the table is exactly seven long. `0x1401e69bf` is its only xref.
/// `sfx9999.ffxbnd.dcx` carries `f0000833.ffx` through `f0000839.ffx` and nothing in
/// `sfx9999_Append.ffxbnd.dcx` matches `f002083x`, so the `+20000` probe finds nothing for these
/// and falls through to the base id -- which is what [`KATANA_SFX_SPAWN`] is given.
///
/// **They linger**, and the evidence is engine-side rather than a claim about the asset: the
/// effect is bound to a persistent ground entity's transform (`entity+0xd0`) rather than to a
/// projectile; the spawner retains its controllers in `entity+0x128` and re-spawns only when
/// those slots are empty; turning the glow off fades it over a whole second; and `0x1401e0490`
/// sweeps the live item-pack list turning the glow back on. A one-shot burst needs none of that.
/// The prism bag is created with all eight item slots zeroed, so it is never picked up.
///
/// A trail should pick ONE of the seven and keep it, so the trail reads as one thing. Which one
/// is a matter of taste; the engine chooses at random per throw.
pub const PRISM_STONE_SFX_IDS: [u32; 7] = [833, 834, 835, 836, 837, 838, 839];

/// Where [`PRISM_STONE_SFX_IDS`] was read from. RVA `0x010c_7b58`, seven `u32`.
///
/// Recorded so the next reader can re-derive the ids from the image rather than trusting the
/// array above, which is a transcription.
pub const PRISM_STONE_SFX_ID_TABLE: u32 = 0x010c_7b58;

/// Bytes of caller-owned storage `0x140beb670` writes into. `0x68`.
///
/// Two `FX4CG::FXCGSfxCtrl` at `0x30` each plus the `param_2[0xc] = 0` at `+0x60`. Must be
/// 16-byte aligned: the constructors store vtable pointers with aligned moves.
pub const KATANA_SFX_CTRL_BYTES: usize = 0x68;

/// Bytes per `FXCGSfxCtrl` sub-object inside that block. `0x30`.
///
/// The stride [`KATANA_SFX_CTRL_DESTROY`] must be called at, twice, descending.
pub const KATANA_SFX_CTRL_HALF_BYTES: usize = 0x30;

// ---------------------------------------------------------------------------------------------
// THE CAMERA, and the one thing in this section that is FOUND rather than declared.
//
// The layout below is static, read out of two constructors and the function that fills them. What
// is NOT static is which of the several camera objects a given frame is drawn through, and that
// is not papered over with a guess: `ds2-invasion-path`'s `camera` module enumerates the
// candidates named here and keeps the one whose projection matrix matches the shape
// `0x140001a90` emits AND that puts the local player somewhere a viewport could show them. A
// wrong candidate fails both tests by a mile; no candidate passing means the overlay draws
// nothing and says so.

/// Offset of `CameraManager` in [`GAME_MANAGER_IMP`]. `+0x20`.
///
/// Named in the Ghidra project's `GameManagerImp` type (field ordinal 4) and confirmed by the 41
/// sites in the image that load `[0x1416148f0]` and immediately dereference `+0x20`.
pub const GAME_MANAGER_CAMERA_MANAGER_OFFSET: usize = 0x20;

/// The three `CameraOperator` pointers `CameraManager` holds: free, player, in-game.
///
/// `+0x18`, `+0x20`, `+0x28`, named in the Ghidra project's `CameraManager` type as
/// `free_cam_operator`, `player_cam_operator` and `ingame_cam_operator`. Which one is driving a
/// given frame depends on whether a cutscene, a menu or ordinary play is on screen, so all three
/// are candidates and the matrices decide.
pub const CAMERA_MANAGER_OPERATOR_OFFSETS: [usize; 3] = [0x18, 0x20, 0x28];

/// `CameraOperator -> view`. `+0x10`, sixteen `f32`, row-major -- **for the class whose
/// constructor is `0x140ae8180`, which is NOT the object the game draws through.** Measured:
/// the live one keeps its view at [`CAMERA_OPERATOR_VIEW_OFFSET_MEASURED`].
///
/// **World to camera.** Not the other way round, and the difference is the whole projection: at
/// `0x140493294` the engine calls the 4x4 INVERSE at `0x140002380` on the camera's transform and
/// stores the result here. Inverting a matrix to reach camera space is only necessary if the
/// input mapped camera space out to the world, so what lands at `+0x10` is the inverse of that --
/// a view matrix, usable directly.
///
/// The constructor at `0x140ae8180` fills `+0x10..+0x4F` and `+0x50..+0x8F` with identity rows
/// from `0x141596b20`, which is why a camera that has never been driven reads as two identities
/// rather than as garbage -- and why the recogniser in `ds2-invasion-path` has a test that an
/// identity matrix is not mistaken for a projection.
pub const CAMERA_OPERATOR_VIEW_OFFSET: usize = 0x10;

/// Where the view matrix actually is on the object the game draws through. `+0x20`.
///
/// **MEASURED IN GAME 2026-09-22**, on the fourth live run of `ds2-invasion-path`, and it
/// contradicts the static derivation above. The log line, written by the DLL itself:
///
/// ```text
/// camera: drawing through operator[0] obj=0x7ffff03aa640 view=+0x020 proj=+0x050
/// ```
///
/// `operator[0]` is the pointer at `CAMERA_MANAGER_OPERATOR_OFFSETS[0]` -- `CameraManager+0x18`,
/// the one Ghidra names `free_cam_operator`. So the POINTER was right and the LAYOUT was not:
/// the projection is at `+0x50` exactly as derived, and the view is sixteen bytes further along
/// than the class whose constructor was read.
///
/// The two are not reconcilable by arithmetic -- a 64-byte matrix at `+0x10` and one at `+0x20`
/// overlap, so at most one of them is the view. The consistent reading is that this object is a
/// different class from the one `0x140ae8180` constructs, which is already known to happen in
/// this family: `IngameCameraOperator`'s constructor chains to `0x140b455a0` instead. Which
/// class, and where its `+0x20` comes from, is not established.
///
/// **The search stays in `ds2-invasion-path` despite this measurement**, and the reason is that
/// one run is one run. The overlay tries this offset, and the others, and lets the projection's
/// shape and the player's own position decide -- so a second camera class with a third layout
/// costs nothing. When two sessions have agreed, the search can go and this constant can be the
/// whole answer.
pub const CAMERA_OPERATOR_VIEW_OFFSET_MEASURED: usize = 0x20;

/// `CameraOperator -> projection`. `+0x50`, sixteen `f32`, row-major.
///
/// Built by `0x140001a90(out, fov, aspect, near, far)` and stored here by `0x140493308`:
///
/// ```text
/// [ cot(fov/2)/aspect  0           0              0 ]
/// [ 0                  cot(fov/2)  0              0 ]
/// [ 0                  0           f/(f-n)        1 ]
/// [ 0                  0           -n*f/(f-n)     0 ]
/// ```
///
/// Left-handed, row-vector, so `clip = [x y z 1] * view * projection` and `clip.w` is the
/// camera-space depth. **`fov` is VERTICAL and in radians**: the builder multiplies it by the
/// `0.5` at `0x1410ac694`, takes `cos/sin` for the cotangent, and divides only the X term by
/// `aspect`. Getting that backwards stretches an overlay horizontally by about 1.78 at 16:9,
/// which is why the convention is recorded rather than assumed.
pub const CAMERA_OPERATOR_PROJECTION_OFFSET: usize = 0x50;

/// First of the camera states embedded in `CameraManager`. `+0x50`.
///
/// The constructor at `0x140491738` takes `this + 0x50` and loops six times, calling the
/// initialiser at `0x140001000` and advancing by [`CAMERA_MANAGER_SLOT_STRIDE`] each time. Each
/// slot begins with an identity 4x4 written by `0x140ae7870`, a second block, and a `u32` at
/// `+0x80`.
///
/// **What these slots are for is not established.** They are candidates rather than a finding:
/// they are the right size and shape to hold a view and a projection, they live on the object
/// that owns the cameras, and the recogniser can tell in one comparison whether any of them
/// actually does. Enumerating them costs six fault-safe reads a session and closes the case
/// either way; asserting what they are without reading their writer would not.
pub const CAMERA_MANAGER_SLOT_BASE: usize = 0x50;

/// Bytes per embedded camera state. `0x90`, from the `add rdi,0x90` at `0x14049174c`.
pub const CAMERA_MANAGER_SLOT_STRIDE: usize = 0x90;

/// How many of them there are. Six, from the `mov esi,5` / `dec esi` / `jns` loop at
/// `0x14049173f`, which runs for `esi` of 5 down to 0 inclusive.
pub const CAMERA_MANAGER_SLOT_COUNT: usize = 6;

/// The two matrix offsets inside one [`CAMERA_MANAGER_SLOT_STRIDE`] slot. `+0x00` and `+0x40`.
///
/// `0x140001000` calls `0x140ae7870`, which writes an identity into `+0x00..+0x3F`, then calls
/// `0x140ae80d0` for the rest and finally zeroes the `u32` at `+0x80`.
pub const CAMERA_MANAGER_SLOT_MATRIX_OFFSETS: [usize; 2] = [0x00, 0x40];

/// `sizeof(CameraManager)`. `0x458`, from the curated type in the Ghidra project.
///
/// Used as the bound of a SEARCH rather than as an offset, which is the only reason a size taken
/// from a curated type is safe to trust here: too small and a candidate is missed, too large and
/// the extra reads are fault-safe and fail the shape test. Neither outcome is a wrong answer.
pub const CAMERA_MANAGER_SIZE: usize = 0x458;

/// The camera hierarchy is NOT what the field names suggest, and this is the note that says so.
///
/// `CameraManager`'s `free_cam_operator` / `player_cam_operator` / `ingame_cam_operator` at
/// [`CAMERA_MANAGER_OPERATOR_OFFSETS`] are camera CONTROLLERS, and at least one of them is not a
/// `CameraOperator` at all: `IngameCameraOperator`'s constructor (`0x1404949e0`) chains to
/// `0x140b455a0`, while `CameraOperator`'s own constructor is `0x140ae8180`. Different base,
/// different layout, no matrices at `+0x10`/`+0x50`.
///
/// What DOES own the matrices is whatever `0x140493030` is called on, and both of its call sites
/// (`0x140493747`, `0x140493b62`) pass their own `this` unchanged -- so the owner's matrices are
/// at `this+0x10` and `this+0x50` exactly as [`CAMERA_OPERATOR_VIEW_OFFSET`] says. Ghidra types
/// one of those callers `FUN_140493340(FreeCameraOperator *this)`.
///
/// Rather than assert which pointer on `CameraManager` reaches it -- a field name on this struct
/// has already been wrong once, see [`CHARACTER_MANAGER_REGISTRY_OFFSET`] -- `ds2-invasion-path`
/// searches the object and lets the projection matrix's own shape decide. The winner is logged,
/// and pinning it here is the follow-up that search exists to make possible.
pub const CAMERA_OPERATOR_OWNER_IS_SEARCHED: () = ();

// ============================================================================================
// DLUID -- THE INPUT DEVICE LAYER
//
// Everything below was read out of three virtual methods that occupy the SAME vtable slot (23,
// byte offset 0xb8) on the three `DLUID::*Device<DLKR::DLSingleThreadingPolicy>` classes. The
// vtables themselves came from MSVC RTTI, via `scripts/ds2-rtti-vtables.py 'DLUID'`:
//
//     .?AV?$PadDevice@VDLSingleThreadingPolicy@DLKR@@@DLUID@@       vtable=0x141271aa8
//     .?AV?$MouseDevice@VDLSingleThreadingPolicy@DLKR@@@DLUID@@     vtable=0x141272158
//     .?AV?$KeyboardDevice@VDLSingleThreadingPolicy@DLKR@@@DLUID@@  vtable=0x141271e98
//
// and slot 23 of each was read straight out of `darksoulsii-deobf.bin`. Slots 0..22 are either
// shared base implementations (identical qwords across all three) or per-class housekeeping;
// slot 23 is the only one each class overrides with a body that calls an input API.
//
// WHY THIS SLOT IS THE STAGE THE GAME ACTUALLY READS. Each of the three bodies is the one place
// a Win32/DirectInput/XInput call is turned into the numbers the rest of the engine consumes,
// and each one writes those numbers into fields of its own device object. Downstream (the
// `DLUI`/`DLUID` mapper: `DLUserInputMapperImpl`, `A2AMappingContext`, `VirtualAnalogKeyInfo<M>`)
// reads the device object, never the API. So a value written into the device object AFTER the
// original body has run is indistinguishable from a value the hardware produced -- and a value
// written BEFORE it is overwritten, which is the same edge `../er-mods-rs`'s `pad_inject` learned
// the hard way on Elden Ring.
//
// ALL THREE ENTRIES ARE CLEAN PROLOGUES, not Arxan redirects, checked one at a time:
//
//     $ python3 scripts/ds2-arxan-chain.py 0x140f05540   # PadDevice
//     $ python3 scripts/ds2-arxan-chain.py 0x140f074a0   # MouseDevice
//     $ python3 scripts/ds2-arxan-chain.py 0x140f06dd0   # KeyboardDevice
//     ... NOT REDIRECTED (clean prologue at the entry)
//
// WHAT IS *NOT* ESTABLISHED HERE, and must not be read into it: which game action each axis is
// bound to. The offsets below say "this float is pad axis 3", because that is what the XInput
// and DirectInput branches both write there. They do NOT say "axis 3 turns the camera" -- that
// is a mapper binding, it is a runtime fact, and `ds2-input-harness` measures it rather than
// assuming it. See that crate's `drive` module.
// ============================================================================================

/// `DLUID::PadDevice<DLKR::DLSingleThreadingPolicy>`'s per-frame poll. RVA `0x00f05540`,
/// VA `0x140f05540`.
///
/// Vtable slot 23 of `0x141271aa8`. The body has three arms, and the reason this crate records
/// device-object offsets rather than an API is that all three converge on the same fields:
///
/// * **XInput.** `XInputGetState([this+0x19c], &state)` at `0x140f05ac2` -- one of only two call
///   sites of the import in the whole image, and the only per-frame one (the other,
///   `0x140ef6347`, is the 0..3 port enumeration). `sThumbLX/LY/RX/RY` are divided by
///   `0x1410dec28` and scaled by `0x1410acb14` into the axis floats below; the triggers are
///   divided by `0x1410ad0cc`; `wButtons` is stored as a `u16`.
/// * **DirectInput joystick.** `IDirectInputDevice8::GetDeviceState(0x50, this+0x148)` through
///   vtable byte offset `0x48` (slot 9) on the device at [`PAD_DEVICE_DINPUT_DEVICE_OFFSET`],
///   at `0x140f05bcb`. `0x50` is `sizeof(DIJOYSTATE)`. Each of the six axes is then written only
///   if its bit is set in [`PAD_DEVICE_AXIS_MASK_OFFSET`], into the SAME six floats.
/// * **A third backend** reached through a function pointer on the device (an 0x78-byte report
///   with 8-bit, 0x80-centred axes). It normalises into the same fields again.
///
/// Prologue, for the same reason every other hook site here records one -- so a detour refuses
/// rather than patching something that moved:
/// `48 89 7c 24 18` (`mov [rsp+0x18],rdi`).
pub const PAD_DEVICE_POLL: u32 = 0x00f0_5540;

/// First five bytes at [`PAD_DEVICE_POLL`], read from `darksoulsii-deobf.bin`.
pub const PAD_DEVICE_POLL_PROLOGUE: [u8; 5] = [0x48, 0x89, 0x7c, 0x24, 0x18];

/// `IDirectInputDevice8*` for a DirectInput joystick, or null. `PadDevice+0x100`.
///
/// From `mov rcx,QWORD PTR [rdi+0x100]` at `0x140f05b93`, the first instruction of the
/// non-XInput arm, immediately followed by a null test that returns.
pub const PAD_DEVICE_DINPUT_DEVICE_OFFSET: usize = 0x100;

/// The raw `DIJOYSTATE` the joystick arm's `GetDeviceState` fills. `PadDevice+0x148`, `0x50`
/// bytes. From `lea r8,[rdi+0x148]` / `mov edx,0x50` at `0x140f05bbc`.
pub const PAD_DEVICE_DIJOYSTATE_OFFSET: usize = 0x148;

/// Length of [`PAD_DEVICE_DIJOYSTATE_OFFSET`]: the `0x50` the poll passes to `GetDeviceState`.
pub const PAD_DEVICE_DIJOYSTATE_BYTES: usize = 0x50;

/// Which of the six axes this DirectInput joystick reports. `PadDevice+0x368`, one bit each.
///
/// From the six `test BYTE PTR [rdi+0x368],<bit>` at `0x140f05bd6`, `0x140f05c27`, `0x140f05c70`,
/// `0x140f05cb1` and the two that follow -- bits `0x01`, `0x02`, `0x04`, `0x08`, `0x10`, `0x20`
/// guarding the writes to `+0x1a4`, `+0x1a8`, `+0x1ac`, `+0x1b0`, `+0x1b4`, `+0x1b8` in that
/// order. This is the evidence that the axis floats are one array of six.
///
/// Only the DirectInput arm consults it; the XInput arm writes its four unconditionally.
pub const PAD_DEVICE_AXIS_MASK_OFFSET: usize = 0x368;

/// The pad's button bitmask, `u16`. `PadDevice+0x198`.
///
/// From `mov WORD PTR [rdi+0x198],ax` at `0x140f05b85`, where `ax` is `XINPUT_GAMEPAD.wButtons`
/// loaded by `movzx eax,WORD PTR [rbp-0x75]` -- offset 4 of the `XINPUT_STATE` buffer at
/// `[rbp-0x79]`.
pub const PAD_DEVICE_BUTTONS_OFFSET: usize = 0x198;

/// XInput user index, `i32`, negative when no XInput pad owns this device. `PadDevice+0x19c`.
///
/// From `mov ecx,DWORD PTR [rdi+0x19c]` / `test ecx,ecx` / `js` at `0x140f05ab0` -- the branch
/// that chooses the DirectInput arm over the XInput one.
pub const PAD_DEVICE_XINPUT_PORT_OFFSET: usize = 0x19c;

/// Base of the six normalised axis floats. `PadDevice+0x1a4`.
///
/// XInput writes four of them (`movss [rdi+0x1a4]`, `[rdi+0x1a8]`, `[rdi+0x1b0]`, `[rdi+0x1b4]`
/// at `0x140f05af6`, `0x140f05b20`, `0x140f05b3b`, `0x140f05b47`). The DirectInput arm writes all
/// six, one per bit of the axis mask, at `+0x00`, `+0x04`, `+0x08`, `+0x0c`, `+0x10`, `+0x14`
/// from this base -- which is what establishes that these are ONE array of six and not four
/// fields with a gap.
pub const PAD_DEVICE_AXES_OFFSET: usize = 0x1a4;

/// How many floats [`PAD_DEVICE_AXES_OFFSET`] covers.
pub const PAD_DEVICE_AXIS_COUNT: usize = 6;

/// Index of the left stick's X axis within [`PAD_DEVICE_AXES_OFFSET`]. XInput `sThumbLX`.
pub const PAD_AXIS_LEFT_X: usize = 0;
/// Index of the left stick's Y axis. XInput `sThumbLY`.
pub const PAD_AXIS_LEFT_Y: usize = 1;
/// Index 2 is written by the DirectInput arm (a joystick Z axis) and by nothing on the XInput
/// path. Recorded so the array is not silently treated as four elements.
pub const PAD_AXIS_DINPUT_Z: usize = 2;
/// Index of the right stick's X axis. XInput `sThumbRX` -- `movss [rdi+0x1b0]`, which is
/// `0x1a4 + 3*4`.
pub const PAD_AXIS_RIGHT_X: usize = 3;
/// Index of the right stick's Y axis. XInput `sThumbRY` -- `movss [rdi+0x1b4]`.
pub const PAD_AXIS_RIGHT_Y: usize = 4;
/// Index 5 is DirectInput-only, same as [`PAD_AXIS_DINPUT_Z`].
pub const PAD_AXIS_DINPUT_RZ: usize = 5;

/// The magnitude a hard-over axis float reaches: `1.0`.
///
/// **Derived from the image's own constants, not assumed.** All three arms normalise to the same
/// range by different arithmetic, and the three agreeing is the evidence:
///
/// * XInput: `sThumb / [0x1410dec28] * [0x1410acb14]`, and those two floats are `65535.0` and
///   `2.0`. `sThumbLX` spans `-32768..=32767`, so the quotient spans `-1.00002..=0.99998`.
/// * DirectInput joystick: `((raw - min) / (max - min) - [0x1410ac694]) * [0x1410acb14]` with
///   `0x1410ac694 == 0.5`, i.e. a `0..=1` fraction re-centred and doubled -- `-1.0..=1.0`. Axis 1
///   uses `[0x1410ada3c] == -2.0` instead, so it is the same range with the sign flipped.
/// * The third backend: `(byte / [0x1410ad0cc]) * [0x1410acb14] - [0x1410ac698]` with
///   `0x1410ad0cc == 255.0` and `0x1410ac698 == 1.0` -- again `-1.0..=1.0`.
///
/// So an injected value belongs in `-1.0..=1.0`, and anything outside it is a value no hardware
/// could have produced.
pub const PAD_AXIS_FULL_SCALE: f32 = 1.0;

/// Left trigger, normalised `0.0..=1.0`. `PadDevice+0x1c4`.
///
/// From `movss [rdi+0x1c4]` at `0x140f05b62`, fed by `bLeftTrigger / 0x1410ad0cc` (`255.0`).
pub const PAD_DEVICE_LEFT_TRIGGER_OFFSET: usize = 0x1c4;

/// Right trigger. `PadDevice+0x1c8`, from `movss [rdi+0x1c8]` at `0x140f05b7d`.
pub const PAD_DEVICE_RIGHT_TRIGGER_OFFSET: usize = 0x1c8;

/// `DLUID::MouseDevice<DLKR::DLSingleThreadingPolicy>`'s per-frame poll. RVA `0x00f074a0`.
///
/// Vtable slot 23 of `0x141272158`. Body: `GetDeviceState(0x14, this+0xf0)` through the device
/// vtable's byte offset `0x48`, then the three `LONG`s of that `DIMOUSESTATE2` are converted to
/// floats. `0x14` is `sizeof(DIMOUSESTATE2)` -- twenty bytes, three axes and eight buttons.
///
/// **This device is filled by DirectInput and by nothing else** -- the body has exactly one
/// source and it is the `GetDeviceState` above. `DarkSoulsII.exe` also imports no raw-input API
/// at all (no `RegisterRawInputDevices`, no `GetRawInputData`; its only `USER32` pointer calls
/// are `GetCursorPos`, `SetCursorPos` and `ShowCursor`), so DirectInput is the whole of the
/// mouse's route into the engine.
///
/// Prologue: `40 53 48 83 ec 20` (`push rbx` / `sub rsp,0x20`).
pub const MOUSE_DEVICE_POLL: u32 = 0x00f0_74a0;

/// First five bytes at [`MOUSE_DEVICE_POLL`].
pub const MOUSE_DEVICE_POLL_PROLOGUE: [u8; 5] = [0x40, 0x53, 0x48, 0x83, 0xec];

/// `IDirectInputDevice8*` the mouse is read through, or null. `MouseDevice+0xe8`.
///
/// From `mov rcx,QWORD PTR [rcx+0xe8]` at `0x140f074a9`, null-tested two instructions later.
pub const MOUSE_DEVICE_DINPUT_DEVICE_OFFSET: usize = 0xe8;

/// The raw `DIMOUSESTATE2` `GetDeviceState` fills. `MouseDevice+0xf0`, 20 bytes.
///
/// `lX`/`lY`/`lZ` at `+0x00`/`+0x04`/`+0x08`, then `rgbButtons[8]` at `+0x0c`. The buttons are
/// NOT copied out by the poll, so anything that wants them reads them here.
pub const MOUSE_DEVICE_RAW_STATE_OFFSET: usize = 0xf0;

/// Length of [`MOUSE_DEVICE_RAW_STATE_OFFSET`]: the `0x14` the poll passes to `GetDeviceState`.
pub const MOUSE_DEVICE_RAW_STATE_BYTES: usize = 0x14;

/// Mouse X delta for this frame, as a float. `MouseDevice+0x108`.
///
/// The poll's own conversion of `DIMOUSESTATE2.lX`; it is a RELATIVE count, not a coordinate.
pub const MOUSE_DEVICE_DELTA_X_OFFSET: usize = 0x108;

/// Mouse Y delta. `MouseDevice+0x10c`.
pub const MOUSE_DEVICE_DELTA_Y_OFFSET: usize = 0x10c;

/// Mouse wheel delta. `MouseDevice+0x110`, from `DIMOUSESTATE2.lZ`.
pub const MOUSE_DEVICE_WHEEL_OFFSET: usize = 0x110;

/// `DLUID::KeyboardDevice<DLKR::DLSingleThreadingPolicy>`'s per-frame poll. RVA `0x00f06dd0`.
///
/// Vtable slot 23 of `0x141271e98`. Body: `GetDeviceState(0x100, this+0xf0)` -- the 256-byte
/// DirectInput DIK table. That is the numbering `ds2-hotkey-config::keys` already carries
/// alongside Win32 virtual keys, and this is the read it was carried for.
///
/// Prologue: `40 53 48 83 ec 20`, the same shape as the mouse poll.
pub const KEYBOARD_DEVICE_POLL: u32 = 0x00f0_6dd0;

/// First five bytes at [`KEYBOARD_DEVICE_POLL`].
pub const KEYBOARD_DEVICE_POLL_PROLOGUE: [u8; 5] = [0x40, 0x53, 0x48, 0x83, 0xec];

/// `IDirectInputDevice8*` the keyboard is read through, or null. `KeyboardDevice+0xe8`.
pub const KEYBOARD_DEVICE_DINPUT_DEVICE_OFFSET: usize = 0xe8;

/// The 256-byte DIK table `GetDeviceState(0x100, ...)` fills. `KeyboardDevice+0xf0`.
pub const KEYBOARD_DEVICE_DIK_TABLE_OFFSET: usize = 0xf0;

/// Length of [`KEYBOARD_DEVICE_DIK_TABLE_OFFSET`]: the `0x100` the poll passes.
pub const KEYBOARD_DEVICE_DIK_TABLE_BYTES: usize = 0x100;

// ============================================================================================
// THE CAMERA'S MOUSE-LOOK -- AND THE CORRECTION IT IS
//
// `MOUSE_DEVICE_POLL` above is a real device and the engine really does read it, but it is NOT
// what turns this camera. Measured live 2026-09-22: `mouse 120 0 30` written into
// `DLUID::MouseDevice`'s normalised deltas left the camera's published yaw at exactly 87.13 for
// thirty frames. The whole chain is elsewhere, it is traced below, and every step of it was read
// out of the disassembly rather than inferred:
//
//   FUN_140af42b0                     the per-frame input update
//     -> parseInput(obj[0], dt)       0x140b08660 -- keyboard/general
//     -> parseCameraInput(obj[1], dt) 0x140b0c950 -- MOUSE-LOOK
//     -> SetCursorPos(...)            0x140af4525 -- the ONLY SetCursorPos call site in the image
//
//   parseCameraInput(cam, dt):
//     cam[2..3] = cam[0..1]           PREVIOUS := CURRENT, at the top, before anything polls
//     for device in cam[0x34]..cam[0x36]:
//         device->vtable[1](dt)       == WindowsMouseDevice::poll, 0x140b5c1f0
//         if device->vtable[3]() then remember it as the active one
//     FUN_140b0d0e0(cam, dt, active)  0x140b0d0e0:
//         *(u64*)cam = *(u64*)(device + 8)    CURRENT := the device's stored POINT
//         cam[0] += cam[4]; cam[1] += cam[5]  ... plus a fixed origin offset
//         cam[0xc]/[0xd] from device+0x14/+0x18   buttons
//         cam[8]        from device+0x10          wheel
//
// So **the camera's mouse input is the difference between two successive values of
// `WindowsMouseDevice+0x08`**, and that field is an ABSOLUTE client-space cursor position, not a
// delta: the poll calls `GetCursorPos` -> `ScreenToClient` -> `GetClientRect` and clamps the
// point into the client rect before storing it. `SetCursorPos` afterwards is the re-centring
// that keeps a relative look going without the real cursor escaping the window.
//
// WHY THE HOOK IS THE POLL AND NOT `GetCursorPos`. The import has exactly two callers -- this
// poll (`0x140b5c224`) and the one-shot init `FUN_140af3f60` (`0x140af40cb`), which seeds the
// starting position through `FUN_140b0c940` and never runs again. So an IAT detour at
// `GETCURSORPOS_IAT_THUNK` would be nearly as narrow. It is still the worse site, for a reason
// that is about WHERE THE VALUE ENDS UP: `GetCursorPos` returns a SCREEN point, which the poll
// then converts and clamps, so authoring there means reproducing `ScreenToClient` and the clamp
// to write the number the consumer will actually read. `+0x08` IS that number -- the last thing
// written before `FUN_140b0d0e0` copies it out -- so one store both authors the mouse and
// overwrites whatever the human's hand produced. Same principle the camera capture used: stand
// where the value goes, not where it is kept.
// ============================================================================================

/// `WindowsMouseDevice`'s per-frame poll. RVA `0x00b5c1f0`, VA `0x140b5c1f0`.
///
/// Vtable slot 1 of `0x1411dcc38` (MSVC RTTI, `scripts/ds2-rtti-vtables.py 'MouseDevice'`). Note
/// that `.?AVMouseDevice@@`'s vtable at `0x1411dcc10` and this one are one contiguous run --
/// `0x1411dcc10 + 5*8 == 0x1411dcc38` -- so the base has four virtuals and this class overrides
/// all of them.
///
/// **This is a different class from [`MOUSE_DEVICE_POLL`]**, which is
/// `DLUID::MouseDevice<DLKR::DLSingleThreadingPolicy>` and reads DirectInput. This one reads the
/// Win32 cursor, and it is the one the camera follows.
///
/// Not Arxan-redirected (`scripts/ds2-arxan-chain.py 0x140b5c1f0`).
///
/// Prologue: `48 89 5c 24 10` (`mov [rsp+0x10],rbx`).
pub const WINDOWS_MOUSE_DEVICE_POLL: u32 = 0x00b5_c1f0;

/// First five bytes at [`WINDOWS_MOUSE_DEVICE_POLL`].
pub const WINDOWS_MOUSE_DEVICE_POLL_PROLOGUE: [u8; 5] = [0x48, 0x89, 0x5c, 0x24, 0x10];

/// The clamped client-space cursor position: two `i32`, x then y. `WindowsMouseDevice+0x08`.
///
/// From `mov QWORD PTR [rbx+0x8],rax` at `0x140b5c288`, where `rax` is the packed pair built by
/// the two `cmovg` clamps at `0x140b5c25e` and `0x140b5c275`. `FUN_140b0d0e0` reads exactly this
/// qword as `*(u64*)(device + 8)`.
///
/// **An absolute position, differenced by the consumer.** Authoring a constant here produces one
/// frame of motion and then nothing; a sustained turn needs a value that keeps moving. Nothing
/// re-clamps it after this store, so an authored position is not bounded by the client rect --
/// which is what makes an arbitrarily long turn possible through this field.
pub const WINDOWS_MOUSE_DEVICE_POSITION_OFFSET: usize = 0x08;

/// Wheel delta for this frame. `WindowsMouseDevice+0x10`, read by `FUN_140b0d0e0` as
/// `device+0x10`. From `mov eax,[rbx+0x28]` / `mov [rbx+0x10],eax` at `0x140b5c2d1`.
pub const WINDOWS_MOUSE_DEVICE_WHEEL_OFFSET: usize = 0x10;

/// Mouse buttons held, bits `0x1`/`0x2`/`0x4`. `WindowsMouseDevice+0x14`.
///
/// From the three `or DWORD PTR [rbx+0x14],<bit>` at `0x140b5c2a3`, `0x140b5c2b8`, `0x140b5c2cd`,
/// each guarded by a call to `0x140af4120`. Whatever that query reads, `+0x14` is the funnel --
/// `FUN_140b0d0e0` takes the buttons from here -- so blanking it after the poll suppresses a
/// click regardless of where the click came from.
pub const WINDOWS_MOUSE_DEVICE_BUTTONS_OFFSET: usize = 0x14;

/// A second per-frame `u32`, `device+0x2c` moved into `device+0x18` by the poll and read by
/// `FUN_140b0d0e0` into the camera object's `[0xd]` (the "just pressed" half of the button
/// state). Recorded so blanking covers it; nothing here authors it.
pub const WINDOWS_MOUSE_DEVICE_BUTTON_EDGE_OFFSET: usize = 0x18;

/// The `HWND` the poll converts and clamps against. `WindowsMouseDevice+0x20`, from
/// `mov rcx,QWORD PTR [rbx+0x20]` at `0x140b5c22a`, passed to `ScreenToClient`.
pub const WINDOWS_MOUSE_DEVICE_HWND_OFFSET: usize = 0x20;

/// The two accumulators the window procedure adds to and the poll drains and zeroes.
/// `WindowsMouseDevice+0x28` and `+0x2c`; the poll's `this[5] = 0` clears both.
pub const WINDOWS_MOUSE_DEVICE_ACCUMULATOR_OFFSET: usize = 0x28;

/// `parseCameraInput`. RVA `0x00b0c950`, VA `0x140b0c950`. A `USER_DEFINED` symbol in the
/// Ghidra project -- a name a human typed, not an inference.
///
/// Recorded as EVIDENCE rather than as a hook site: it is what establishes that the mouse-look
/// value is a per-frame difference, because its first act is `previous := current` and its last
/// is to pull a new `current` out of the active device. Nothing detours it.
pub const PARSE_CAMERA_INPUT: u32 = 0x00b0_c950;

/// The pull from the active mouse device into the camera-input object. RVA `0x00b0d0e0`.
///
/// Also evidence rather than a hook site: `*(u64*)cam = *(u64*)(device + 8)` is the single line
/// that makes [`WINDOWS_MOUSE_DEVICE_POSITION_OFFSET`] the field worth writing.
pub const CAMERA_INPUT_PULL_FROM_DEVICE: u32 = 0x00b0_d0e0;

/// The per-frame input update that calls `parseInput`, `parseCameraInput` and then the image's
/// only `SetCursorPos`. RVA `0x00af42b0`, VA `0x140af42b0`.
pub const INPUT_UPDATE: u32 = 0x00af_42b0;

/// The import thunk `DarkSoulsII.exe` calls `USER32!GetCursorPos` through. RVA `0x01aae3cc`.
///
/// From `call QWORD PTR [rip+0xf521a2]` at `0x140b5c224`, which resolves to `0x141aae3cc` -- the
/// same `USER32` first-thunk table [`SLEEP_IAT_THUNK`]'s `KERNEL32` neighbour lives in.
///
/// **Recorded and deliberately NOT hooked.** See this section's banner for why the poll is the
/// better site; this constant exists so the next person can see that the alternative was
/// identified and rejected on evidence rather than missed.
pub const GETCURSORPOS_IAT_THUNK: u32 = 0x01aa_e3cc;

// ============================================================================================
// THE CAMERA HAS NO STORED YAW -- A NEGATIVE RESULT, RECORDED SO IT IS NOT RE-SEARCHED
//
// The obvious way to turn the camera with no device connected is to find the angles the camera
// controller integrates and write them. **On this engine there are none.** The evidence:
//
// `FUN_140493030` (RVA 0x00493030, 770 bytes) is the function `CAMERA_OPERATOR_VIEW_OFFSET`
// already points at: it writes the world-to-camera matrix to `this+0x10` and the projection to
// `this+0x50`, and it is the last thing to touch them before a frame is drawn. It calls `sinf`
// and `cosf` exactly twice each, and BOTH pairs are fed by the same single float:
//
//     0x1404930bf   movss xmm0, DWORD PTR [rcx+0x10c]
//     0x1404930d9   call  cosf
//     0x1404930e7   movss xmm0, DWORD PTR [rbx+0x10c]
//     0x1404930ef   call  sinf
//     ... and the same pair again at 0x140493177 / 0x140493191 / 0x14049319f / 0x1404931a7
//
// One angle, not three. Everything else the builder consumes is a POSITION -- it reads vectors
// through `FUN_140002680` / `FUN_140002380` (look-at helpers) and `FUN_140001a90` (the
// projection builder). So `+0x10c` is the camera's ROLL about its own view axis, and heading and
// pitch are DERIVED from an eye position and a target position rather than stored.
//
// `NormalCameraOperator`'s update (`0x1404a2700`) is the same shape from the other end: it
// computes eye/target/up as 4-float vectors, smooths each toward its target with a per-frame
// lerp factor, and hands the results to the same look-at helpers. No angle is integrated
// anywhere in it.
//
// CONSEQUENCE FOR AN AGENT THAT WANTS TO TURN THE CAMERA:
//
// * Writing the view matrix at `+0x10` is NOT a camera turn for measurement purposes. It is the
//   builder's OUTPUT, rebuilt from scratch every frame, and -- fatally for the experiment this
//   repo is running -- `ds2-invasion-path` reads that same matrix, so rotating it would make the
//   overlay track by construction and prove nothing.
// * The real state is the eye position and whatever orbit parameter produces it. For the
//   third-person camera that is inside `ExFollowCameraOperator`'s update (`0x14049afb0`), which
//   is a pipeline of about fifteen single-argument stages. Two of them have been eliminated:
//   `0x14049bc00` fills the parameter block at `+0x3f4..+0x430` and `0x14049b1f0` copies that
//   block into `+0x138..+0x1f4` -- both are camera-parameter blending, not orbit state. The
//   remaining stages are unexamined and that is where the next search starts.
//
// Recorded here rather than left as a gap because "look for the camera's yaw" is exactly the
// search someone will start again otherwise, and it has now cost one round.
// ============================================================================================

/// The view/projection matrix builder. RVA `0x00493030`, VA `0x140493030`.
///
/// Writes the world-to-camera matrix to `this+`[`CAMERA_OPERATOR_VIEW_OFFSET`] and the projection
/// to `this+`[`CAMERA_OPERATOR_PROJECTION_OFFSET`]. Called from `0x140493780` and `0x140493340`,
/// both of which pass their own `this` unchanged -- which is what
/// [`CAMERA_OPERATOR_OWNER_IS_SEARCHED`] already recorded.
///
/// **Evidence, not a hook site.** See this section's banner: its only trigonometry is on
/// [`CAMERA_OPERATOR_ROLL_OFFSET`], which is what proves the camera's heading is derived from
/// positions rather than stored as an angle.
pub const CAMERA_VIEW_MATRIX_BUILDER: u32 = 0x0049_3030;

/// The camera's roll about its own view axis, in radians. `CameraOperator+0x10c`.
///
/// The ONLY field [`CAMERA_VIEW_MATRIX_BUILDER`] passes to `sinf`/`cosf`, at `0x1404930bf`,
/// `0x1404930e7`, `0x140493177` and `0x14049319f`. Writing it tilts the horizon; it does not
/// turn the camera, and it is recorded to make clear which angle this is and which it is not.
pub const CAMERA_OPERATOR_ROLL_OFFSET: usize = 0x10c;

/// `ExFollowCameraOperator`'s per-frame update. RVA `0x0049afb0`, VA `0x14049afb0`.
///
/// Vtable slot 4 of `0x1410f4a98`. Fifteen single-argument stages; the orbit state that decides
/// where the third-person camera sits relative to the player is in one of the unexamined ones.
/// The next search for a device-free camera turn starts here.
pub const EX_FOLLOW_CAMERA_UPDATE: u32 = 0x0049_afb0;

/// The `ExFollowCameraOperator` stage that fills the camera-parameter block at `+0x3f4..+0x430`.
/// RVA `0x0049bc00`. **Eliminated**: parameter blending, not orbit state.
pub const EX_FOLLOW_CAMERA_PARAM_BLEND: u32 = 0x0049_bc00;

/// The stage that copies `+0x3f4..+0x430` into `+0x138..+0x1f4` where a value is non-zero.
/// RVA `0x0049b1f0`. **Eliminated** for the same reason.
pub const EX_FOLLOW_CAMERA_PARAM_APPLY: u32 = 0x0049_b1f0;

// THE SAVE/LOAD DIRECTORY SPLIT
//
// A session asks for its container directory through a virtual, and the save class and the load
// class have their own overrides. Both are three lines with the same shape -- fetch a wide string,
// measure it, hand it to the session's string setter -- and differ only in the object they read it
// from. So nothing at runtime has to be decoded to tell a save from a load: the class that is
// asking is the answer, and the two answers live at two addresses.
//
// AND IT IS NOT ON THE PATH TO THE FILE, which a live run measured. Recorded because the pair is
// worth seeing and because the reasoning that led here has to stay visible: this was written as a
// replacement for a plan aimed at `SAVE_DIR_BUILD`, on the grounds that `SAVE_DIR_BUILD` runs once
// at session setup rather than per request. That is true and it is not a disqualification --
// `SAVE_DIR_BUILD`'s result is handed to `SL_REQUEST_SET_DIRECTORY`, which seats it on the storage
// worker, and the worker is what a container read opens. The split below moves a field the read
// does not consult; `load-answered=1` on a read that still failed is what that looks like.

/// `SaveLoad2::SLLoadSession`'s directory override. **The load side of the split.**
///
/// Reads its string from `this+0xe8` through `FUN_140a8a180`, then calls the session string setter
/// at `0x140a89050`. Its save-side twin is [`SL_SAVE_SESSION_DIRECTORY`].
pub const SL_LOAD_SESSION_DIRECTORY: u32 = 0x00a8_f8d0;

/// `SaveLoad2::SLSaveSession`'s directory override. **The save side**, recorded so a reader can see
/// the pair and check that the two really are distinct functions rather than one shared one.
///
/// Reads its string from `this[0x1d]` through `FUN_140a89cf0`. Nothing here hooks it: a save must
/// keep writing the player's own folder, and leaving it alone is how that is guaranteed.
pub const SL_SAVE_SESSION_DIRECTORY: u32 = 0x00a8_ec40;

/// `SaveLoad2::SLLoadSession`'s vtable.
///
/// Written by the class's two constructors, `FUN_140a8f6b0` and `FUN_140a8f7d0`, each storing it
/// twice in the usual ctor/dtor pattern.
pub const SL_LOAD_SESSION_VTABLE: u32 = 0x011b_64e0;

/// Where [`SL_LOAD_SESSION_DIRECTORY`] sits in [`SL_LOAD_SESSION_VTABLE`]: slot 3, `+0x18`.
///
/// **This is why the load redirect needs no code patch.** The override has no call sites at all --
/// its only references are this slot and an RTTI entry -- so it is reached exclusively through the
/// vtable, and arming the redirect is a pointer write into `.rdata`. The instruction stream is
/// untouched, which takes Arxan out of the question for this site the way a `.flo` table
/// substitution does for `ds2-menu-row`.
pub const SL_LOAD_SESSION_DIRECTORY_VTABLE_SLOT: usize = 3;

/// The session string setter both directory overrides finish with: an MSVC
/// `basic_string<wchar_t>::assign(const wchar_t *, size_t)` on the session's own storage.
///
/// `fn(session: *mut SLSession, chars: *const u16, len: usize)`. Its small-string-optimisation
/// layout is the usual one -- inline buffer until the capacity at `+0x18` exceeds seven, pointer
/// after that -- and calling it is how a replacement override hands back a path without touching
/// the game's allocator by hand, the same reasoning as [`WSTRING_ASSIGN`].
pub const SL_SESSION_STRING_SET: u32 = 0x00a8_9050;

// THE DIRECTORY A CONTAINER READ ACTUALLY OPENS
//
// Not the load session's virtual, and not the content's own string. The storage worker holds it,
// and exactly one function writes it: `FUN_140a899f0`, below. Read out of the `0x18` arm of the
// session pump at `0x1402e6230`, which is the whole of the game's own path to it:
//
// ```text
// case 0x18:                                     // session setup
//     FUN_140248db0(&dir, steamid);              // SAVE_DIR_BUILD -- builds "...\DarkSoulsII\<id>\"
//     if (!SLSystem->field_0x1a1) {              // the once-per-process latch
//         FUN_140a899f0(SLSystem->_x38, 0, dir); //   first session: index 0
//         SLSystem->field_0x1a1 = true;
//     } else {
//         FUN_140a899f0(SLSystem->_x38, 1, dir); //   every later session: index 1
//     }
// ```
//
// That is the ENTIRE consumer list of `SAVE_DIR_BUILD`'s result, which is what makes the chain
// closed: the launch-time redirect on `SAVE_DIR_BUILD` was measured reading a donor container end
// to end, and this is the only route its string can have taken to get there.

/// `void SetRequestDirectory(holder, u32 index, const wchar_t *path)` -- `0x140a899f0`.
///
/// **The seam an in-session redirect belongs on.** Three calls: find the worker for the holder's
/// id, set its directory, release it. The find and the release are a lock/unlock PAIR --
/// `FUN_140a8bfb0` takes `manager+0x50` and the worker's own `+0xb0` and leaves both held,
/// `FUN_140a8c390` releases them -- so this function is called whole or not at all. Calling the
/// finder alone to read the directory back would wedge the save system on the next request.
///
/// `holder` is the VALUE at [`SL_REQUEST_HOLDER_OFFSET`], not its address: the function reads the
/// manager from `[holder]` and the worker id from `[holder+8]`.
pub const SL_REQUEST_SET_DIRECTORY: u32 = 0x00a8_99f0;

/// The five bytes [`SL_REQUEST_SET_DIRECTORY`] must begin with. `mov [rsp+8],rbx`.
///
/// Checked for the same reason `SAVE_LOAD_REQUEST_SAVE`'s is: an RVA is a number, and on a build
/// these offsets were not read from, this address is some other function that would accept the
/// call and leave a log line claiming a directory was set.
pub const SL_REQUEST_SET_DIRECTORY_PROLOGUE: [u8; 5] = [0x48, 0x89, 0x5c, 0x24, 0x08];

/// The worker-side half of the same write: `void SetWorkerDirectory(worker, u32 index, path)` --
/// `0x140a8d9b0`. Hooked to OBSERVE, never to change an answer.
///
/// It is the only writer of [`SL_WORKER_DIRECTORY_OFFSET`], so a detour here sees every directory
/// the game sets on itself as well as every one a mod sets, and is the only way to find out whether
/// a set landed -- the worker pointer is otherwise reachable only through the locked finder.
pub const SL_WORKER_SET_DIRECTORY: u32 = 0x00a8_d9b0;

/// The five bytes [`SL_WORKER_SET_DIRECTORY`] must begin with.
///
/// `40 57` is `push rdi` carrying a redundant REX prefix, not `57` -- a disassembly LISTING spells
/// that instruction the same either way, so the encoding has to be read as bytes. It was not, and
/// the prologue check refused the detour on the first run with
/// `saw=[40, 57, 41, 56, 41] want=[57, 41, 56, 41, 57]`, which is the check doing its whole job:
/// five bytes off by one would have been a trampoline into the middle of `push r15`.
pub const SL_WORKER_SET_DIRECTORY_PROLOGUE: [u8; 5] = [0x40, 0x57, 0x41, 0x56, 0x41];

/// The request holder inside a `SaveLoadSystem`. `[system + 0x38]`.
///
/// Ghidra names the field `_x38_SLRequestMan_` and the pump's `0x18` arm passes it straight to
/// [`SL_REQUEST_SET_DIRECTORY`]. It is a handle rather than the manager itself: `[holder]` is the
/// manager, `[holder+8]` is the `u32` id of the worker to act on.
pub const SL_REQUEST_HOLDER_OFFSET: usize = 0x38;

/// The container directory on a storage worker. `worker + 0x48`.
///
/// An MSVC `basic_string<wchar_t>` with the usual small-string layout, written by
/// [`SL_SESSION_STRING_SET`] from the tail of [`SL_WORKER_SET_DIRECTORY`]:
/// `lea rcx,[r14+0x48]` at `0x140a8da45`, after the length is measured by scanning for the
/// terminator.
pub const SL_WORKER_DIRECTORY_OFFSET: usize = 0x48;

/// The flag that makes [`SL_WORKER_SET_DIRECTORY`] do nothing. `worker + 0xad`.
///
/// `movzx edi,byte ptr [r14+0xad]` at `0x140a8da06`, and a non-zero value jumps the whole body --
/// the index write, the status reset and the string set all of it. **A set that is skipped is
/// silent**, which is why the observer reads this byte out and logs it: it is the difference
/// between "the directory was refused" and "the directory was never asked for".
pub const SL_WORKER_SET_SKIPPED_OFFSET: usize = 0xad;

/// The index a mid-session directory set passes: `1`.
///
/// The game passes `0` exactly once per process -- the `field_0x1a1` latch in the pump's `0x18` arm
/// -- and `1` for every session after it. Anything an in-session swap does is after that, so `1` is
/// what the game itself would pass at that moment. It lands at `worker+0x3c`.
pub const SL_REQUEST_DIRECTORY_INDEX_SESSION: u32 = 1;

/// The `SLLoadContent` a `SaveLoadSystem` builds its container requests from. `[system + 0x30]`.
///
/// Read off `SAVE_LOAD_SYSTEM_LOAD_SYSTEM_DATA` (`0x1402e72c0`), which touches it three times in
/// eleven instructions: `mov rcx,[rbx+0x30]; call 0x140a8a0d0` releases the previous request,
/// `mov rcx,[rbx+0x30]; mov edx,7; call 0x140a8a250` sets the container entry, and the funnel at
/// `0x140a86280` is handed the same pointer in `r9`. Every container read in the image goes through
/// that funnel, so this is the object whose directory a read opens.
pub const SAVE_LOAD_SYSTEM_CONTENT_OFFSET: usize = 0x30;

/// The container NAME inside an `SLLoadContent`. `content + 0x08`.
///
/// `FUN_140a8a180`, the accessor every session builder goes through, is two instructions --
/// `lea rax,[rcx+8]; ret` -- so the accessor is the offset. `FUN_140a8a6f0` hands the result to
/// `FUN_140a87d10` to compare against each registered container and to `FUN_140a86e70` to copy,
/// which is what a key is for.
///
/// # This was called `SL_CONTENT_DIRECTORY_OFFSET` and it is not a directory
///
/// A live run on 2026-09-23 read it and reported
/// `<unread content=0x00007ffffa809bc0 length=Some(356486873167) capacity=Some(7)>`. Those two
/// numbers are the whole correction: `356486873167` is `4f 00 46 00 53 00 00 00`, UTF-16 `"OFS\0"`,
/// and the `7` beside it is a length rather than a capacity. A seven-character string whose fifth,
/// sixth and seventh characters are `OFS` is `DS2SOFS` -- the save file's own name. So the string
/// starts at `content+0x10`, the field here is the key that names it, and no directory lives on
/// `SLLoadContent` at all.
///
/// The measurement was only legible because the reader reported its numbers instead of
/// `<unreadable>`; see `ds2_save_redirect::request_dir::ContentDirectory`.
pub const SL_CONTENT_NAME_OFFSET: usize = 0x08;

/// Where an `SLLoadSession` keeps the `SLLoadContent` it was built from. `session + 0xe8`.
///
/// `FUN_140a8f6b0`, the constructor, writes `param_1[0x1d] = loadContent`. The directory virtual
/// this crate used to hook reads through the same field, which is why it answers about the
/// container's identity rather than about a folder.
pub const SL_SESSION_CONTENT_OFFSET: usize = 0xe8;

/// `u32 GetSessionState(holder)` -- `0x140a89940`, the value the pump switches on.
///
/// It looks the worker up by the holder's id and returns [`SL_SESSION_STATE_DONE`] when there is
/// none, which is the same failed lookup that makes a directory set a no-op.
pub const SL_GET_SESSION_STATE: u32 = 0x00a8_9940;

/// Session setup: the pump arm that builds a directory and seats it on the worker.
///
/// The only arm of either pump (`0x1402e6230` at `0x1402e635c`, `0x1402e67f0` at `0x1402e6930`)
/// that calls [`SAVE_DIR_BUILD`] and [`SL_REQUEST_SET_DIRECTORY`]. It is a STATE and not a
/// construction argument: both session constructors take their kind from
/// [`SL_SESSION_BUILD_SAVE`]'s third parameter, and every one of that function's nine call sites
/// passes zero (`xor r8d,r8d`), so nothing asks for this state directly.
pub const SL_SESSION_STATE_SETUP: u32 = 0x18;

/// What [`SL_GET_SESSION_STATE`] answers when the worker lookup finds nothing: the pump reads it
/// as "the worker is done".
///
/// At the title, between requests, this is what the holder's id resolves to -- which is why a
/// directory set made there reports `set-made-no-write`.
pub const SL_SESSION_STATE_DONE: u32 = 0x14;

/// What [`SL_GET_SESSION_STATE`] answers when the holder has no manager at all.
pub const SL_SESSION_STATE_NO_MANAGER: u32 = 0x19;

/// The id a worker answers to, matched by the finder. `worker + 0xa8`.
///
/// `FUN_140a8d560` is the getter: lock `worker+0xb0`, read this, unlock.
pub const SL_WORKER_ID_OFFSET: usize = 0xa8;

/// The index [`SL_WORKER_SET_DIRECTORY`] writes alongside the directory. `worker + 0x3c`.
pub const SL_WORKER_INDEX_OFFSET: usize = 0x3c;

/// The status [`SL_WORKER_SET_DIRECTORY`] resets when its index is not 3. `worker + 0x98`.
pub const SL_WORKER_STATUS_OFFSET: usize = 0x98;

/// The value written to [`SL_WORKER_STATUS_OFFSET`] by that reset.
pub const SL_WORKER_STATUS_DIRECTORY_SET: u32 = 0x16;

/// The lock every worker field is read and written under. `worker + 0xb0`.
///
/// A vtable with acquire at `+0x10` and release at `+0x20`. [`SL_REQUEST_SET_DIRECTORY`] takes it
/// and the manager's own, and only its tail call releases them, which is why that function has to
/// be called whole.
pub const SL_WORKER_LOCK_OFFSET: usize = 0xb0;

/// `FUN_140a86280` -- builds a load session and registers it with the manager.
///
/// Three callers, all `SaveLoadSystem` methods: `loadSlot_0_andOtherSetup` (`0x1402e72c0`),
/// `loadSlotByIndex` (`0x1402e6ff0`) and `loadSlot22_andOtherSetup` (`0x1402e7170`).
pub const SL_SESSION_BUILD_LOAD: u32 = 0x00a8_6280;

/// `FUN_140a863a0(out, listener, u32 kind, content, ...)` -- the save-side twin.
///
/// Nine call sites across five `SaveLoadSystem` methods, and every one of them passes `0`  for
/// `kind`. That is the evidence behind [`SL_SESSION_STATE_SETUP`] being a state.
pub const SL_SESSION_BUILD_SAVE: u32 = 0x00a8_63a0;

/// `SaveLoad2::SLSaveSession`'s vtable, the save-side twin of [`SL_LOAD_SESSION_VTABLE`].
///
/// Read out of the image's RTTI the same way its twin was, with `scripts/ds2-rtti-vtables.py
/// 'SLSaveSession' --slot 0x18`, which prints slot 3 holding [`SL_SAVE_SESSION_DIRECTORY`]:
///
/// ```text
/// .?AVSLLoadSession@SaveLoad2@@  vtable=0x1411b64e0  this+0x0  [+0x18]=0x140a8f8d0
/// .?AVSLSaveSession@SaveLoad2@@  vtable=0x1411b6430  this+0x0  [+0x18]=0x140a8ec40
/// ```
///
/// **Swapping this is not the startup redirect.** A save that answers the staged copy is only
/// correct once the character being played CAME from that copy; armed any earlier it writes the
/// player's own character into somebody else's container. The one flow that arms it --
/// `ds2-save-file`'s in-session swap -- does so only after the game has entered the donor
/// character, which is the whole reason the two sides are separate constants.
pub const SL_SAVE_SESSION_VTABLE: u32 = 0x011b_6430;

/// Where [`SL_SAVE_SESSION_DIRECTORY`] sits in [`SL_SAVE_SESSION_VTABLE`]: slot 3, `+0x18`.
///
/// The same slot as [`SL_LOAD_SESSION_DIRECTORY_VTABLE_SLOT`], which is what it should be -- both
/// classes derive from `SaveLoad2::SLSession` and both override the same base virtual. Recorded
/// separately anyway, because "it must be the same slot" is the sort of reasoning that survives
/// right up until the build where it is not.
pub const SL_SAVE_SESSION_DIRECTORY_VTABLE_SLOT: usize = 3;

/// `SaveLoadSystem`'s request to re-read the container's **system data**: the ten character
/// records the title's LOAD GAME list is built from. RVA `0x002e_72c0`.
///
/// `bool loadSystemData(SaveLoadSystem *)`. Ghidra names it
/// `FUN_1402e72c0_loadSlot_0_andOtherSetup`; what it actually does, read out of its decompilation,
/// is set container entry **7** for loading and hand the request to the `SLRequestMan`:
///
/// ```c
/// if (SLSystem->_x38_SLRequestMan_ == 0 || SLSystem->_x8 != 0 || SLSystem->_xc != 0) return false;
/// FUN_140a8a0d0_zeroiseLoadSlots_andLoadContent__(SLSystem->SLLoadContent);
/// FUN_140a8a250_setSlotForLoading__(SLSystem->SLLoadContent, 7);
/// ... FUN_140a86280_unk6ArgFct(..., SLSystem->SLLoadContent, 2, 0) ...
/// SLSystem->_x8 = 4; SLSystem->_xc = 2;
/// return true;
/// ```
///
/// **This is the refresh the in-session character swap needs.** `FUN_1400f0f60` -- the vector
/// `FeSubStateTitleLoadDataList::v1` measures before it decides whether the list has anything in
/// it -- is built entirely out of `GameManagerImp->GameDataManager->savedata__`, walking ten
/// `0x1f0`-byte records and keeping the ones whose `+0x1d9` says occupied. Nothing in that path
/// re-reads the file. So pointing the loads at another container changes what the list says only
/// once that block has been filled again, and this is the call that fills it.
///
/// Its own guard is the interlock at [`SAVE_LOAD_SYSTEM_STATE_OFFSET`] /
/// [`SAVE_LOAD_SYSTEM_SUBSTATE_OFFSET`], so calling it while the game is mid-request returns
/// `false` and changes nothing -- which is what makes it safe to retry rather than schedule.
pub const SAVE_LOAD_SYSTEM_LOAD_SYSTEM_DATA: u32 = 0x002e_72c0;

/// The five bytes [`SAVE_LOAD_SYSTEM_LOAD_SYSTEM_DATA`] must begin with: `rex push rbx; sub rsp`.
///
/// `scripts/ds2-arxan-chain.py 0x1402e72c0` reports `NOT REDIRECTED (clean prologue at the entry)`,
/// so the bytes below are what the live process holds. Nothing detours this address; it is called.
/// The check is still worth making, because an RVA is a number, and a number that lands on the
/// wrong function in some other build would be called just as happily.
pub const SAVE_LOAD_SYSTEM_LOAD_SYSTEM_DATA_PROLOGUE: [u8; 5] = [0x40, 0x53, 0x48, 0x83, 0xec];

/// `SaveLoadSystem`'s per-frame **pump**: the call that finishes a request
/// [`SAVE_LOAD_SYSTEM_LOAD_SYSTEM_DATA`] started. RVA `0x002e_6230`.
///
/// `int pump(SaveLoadSystem *)`, Ghidra's `FUN_1402e6230_saveLoadSetup__`.
///
/// # A request is started by one call and finished by a different one, every frame
///
/// This was the missing half of the in-session swap, and its absence cost a live run. The start
/// entry point hands a request to the `SLSession` worker thread and writes the interlock
/// (`_x8 = 4; _xc = 2`); **nothing on the worker thread ever writes it back**. The clearing, and
/// the parse of what the worker read, happen here -- on the game thread, when this is called:
///
/// ```c
/// if (((_x8 - 2) & 0xfffffffd) != 0) return 4;            // nothing in flight
/// if (_x38_SLRequestMan == 0)        return 10;
/// switch (getSLSessionType(_x38)) {                       // 0x14 == the worker is done
///   case 4..10, 0xd:  ...; return 1;                      // STILL WORKING
///   case 0xb..0x12:   ...; return 1;                      // still working, other kinds
///   case 0x14:  _x8 = 0;                                  // <-- the interlock is cleared HERE
///               err = _x38->result;
///               if (err) { _xc = 0; ...; return <err code> }
///               switch (_xc) { case 2: parse container entry 7 into [this+0x18]; }
///               _xc = 0; return 0;                        // DONE, and the records are filled
/// }
/// ```
///
/// # Its only callers are two title substates, and neither is resident at the top menu
///
/// `get_xrefs_to` finds exactly two: `FeSubStateTitleSteamLoadSystemData::update`
/// (`0x1400fbdb0`, three call sites -- one per phase 1, 2 and 3) and
/// `FeSubStateTitleLoadProfile`'s poll (`0x1400fc5b0`). Both call it once per frame and hold their
/// phase while it answers `1`, which is what makes it safe to call every frame: that IS the
/// shipped call pattern, and an idle interlock returns `4` without touching anything.
///
/// So a re-read requested while `0x47 TopMenu` is up -- which is exactly where the in-session
/// character swap requests one -- is accepted, is performed by the worker, and then **waits
/// forever**, because the substate that would have collected it is not on screen. The first live
/// run of the swap logged precisely that: `re-read requested accepted=true` followed by fifteen
/// seconds of nothing and `swap ABANDONED -- the container re-read never finished`. The flow has
/// to pump it itself, because at the top menu it is the only thing that can.
pub const SAVE_LOAD_SYSTEM_PUMP: u32 = 0x002e_6230;

/// The five bytes [`SAVE_LOAD_SYSTEM_PUMP`] must begin with: `rex push rbp; push rsi; lea rbp`.
///
/// `scripts/ds2-arxan-chain.py 0x1402e6230` reports `NOT REDIRECTED (clean prologue at the entry)`
/// and prints `40 55 56 48 8d 6c 24 b1`, so these are the bytes the live process holds.
pub const SAVE_LOAD_SYSTEM_PUMP_PROLOGUE: [u8; 5] = [0x40, 0x55, 0x56, 0x48, 0x8d];

/// [`SAVE_LOAD_SYSTEM_PUMP`]'s answer when the worker has not finished yet: keep calling it.
///
/// The one status both shipped callers test by name (`if (iVar8 == 1) break;` -- hold the phase).
pub const SAVE_LOAD_SYSTEM_PUMP_WORKING: i32 = 1;

/// [`SAVE_LOAD_SYSTEM_PUMP`]'s answer when the request completed and its data was parsed in.
pub const SAVE_LOAD_SYSTEM_PUMP_DONE: i32 = 0;

/// [`SAVE_LOAD_SYSTEM_PUMP`]'s answer when the interlock was already clear: nothing was in flight.
///
/// `mov eax,0x4` at `0x1402e625d`, the first bail. Not an error and not a completion -- it is what
/// a pump call on an idle system says, which is why calling it unconditionally is harmless.
pub const SAVE_LOAD_SYSTEM_PUMP_IDLE: i32 = 4;

/// The five bytes [`SL_SESSION_STRING_SET`] must begin with. `mov [rsp+8],rbx`.
///
/// Checked before the call for the reason every other prologue here is: an RVA is a number, and on
/// a build these offsets were not read from, this address is some other function that would accept
/// the call and leave a log line claiming a directory was set.
pub const SL_SESSION_STRING_SET_PROLOGUE: [u8; 5] = [0x48, 0x89, 0x5c, 0x24, 0x08];

/// The state the session pump switches on. `worker + 0x98`.
///
/// Read by `FUN_140a8d6b0` under the worker's lock, returned by [`SL_GET_SESSION_STATE`], and used
/// as the index into the pump's two-level jump table at `0x1402e678c` / `0x1402e67a0`. Valid range
/// `0..=0x19`; anything above falls through.
pub const SL_WORKER_STATE_OFFSET: usize = 0x98;

/// What `SLSession`'s base constructor leaves the state at. `0x15`.
///
/// `FUN_140a8ce00` writes it alongside `worker+0x9c = 8` and
/// [`SL_WORKER_KIND_OFFSET`]` = 3`.
pub const SL_SESSION_STATE_CREATED: u32 = 0x15;

/// The state a directory set leaves behind. `0x16`.
///
/// [`SL_WORKER_SET_DIRECTORY`] writes it whenever its index is not `3`, and it is the only value
/// any of the four immediate stores to [`SL_WORKER_STATE_OFFSET`] in the save/load region write.
pub const SL_SESSION_STATE_DIRECTORY_SET: u32 = 0x16;

/// The kind the constructor is passed, and the value a directory set overwrites. `worker + 0x38`.
///
/// `FUN_140a8d760` is the setter; the base constructor leaves `3` at
/// [`SL_WORKER_INDEX_OFFSET`], which is what makes [`SL_WORKER_SET_DIRECTORY`]'s `index != 3`
/// test mean "a directory has been chosen".
pub const SL_WORKER_KIND_OFFSET: usize = 0x38;

// ---------------------------------------------------------------------------------------------
// THE ITEM CELL: ITS INFUSION MARK, AND THE STAT REQUIREMENT THE GAME ALREADY TESTS
//
// Read statically on 2026-09-23 from `darksoulsii-deobf.bin` (SOTFS build 9527516) with the
// Ghidra MCP daemon and `scripts/ds2-disasm.py`, and from the shipped layout with
// `scripts/ds2-ebl.py` + `scripts/ds2-flo.py`. **No game was launched for any of it, and none of
// it has been in front of a running game.** Addresses below are RVAs; add `0x140000000` for the
// VA the disassembly prints.
//
// Reproduce the layout numbers with:
//
//     python3 scripts/ds2-ebl.py extract /menu/02.febnd.dcx --out /tmp/menu02
//     python3 scripts/ds2-flo.py tree /tmp/menu02/l02_02_Inventory.flo --def 0x7a
//     python3 scripts/ds2-flo.py tree /tmp/menu02/l02_02_Inventory.flo --def 0x70
//
// THE QUESTION THIS BLOCK ANSWERS, because it was asked as "how does an infused weapon get a
// different icon" and the answer is that it does not get one. The item icon is the item's own id
// and nothing else: `FUN_140035e10` (`0x00035e10`) builds the icon key as `{kind 6, id}` where the
// id comes from `FUN_14003bd90` (`0x0003bd90`), which reads the u32 at inventory-entry `+0x18` and
// passes it through `FUN_14003c8c0` (a swap between two specific ids, not an infusion offset).
// Kind 6 falls to the default arm of the path builder `FUN_14048c3e0` (`0x0048c3e0`), which is
// `icon:/tex/Icon/IC_%010d.tpf`. **One texture per item id. No infusion component anywhere in it.**
//
// The infusion mark is a SEPARATE ELEMENT, and there are nine of them sitting in the cell all the
// time with at most one visible. See [`FE_ITEM_CELL_INFUSION_ELEMENT_BASE`].
// ---------------------------------------------------------------------------------------------

/// The inventory item-cell bind. `FUN_1400bc850`. RVA `0x000bc850`.
///
/// `fn(cell: *CellView, item: *FeItemData, showIcon: bool)`. Called once per visible row per
/// refresh from `FUN_1400bc2b0` (`ItemSelectDialog`'s list rebuild), which builds `cell` on its own
/// stack with `FUN_1400b7680` immediately before. It is the one place every drawable fact about an
/// item row is written: the icon texture, the name, the count, the durability gauge, the equipped
/// mark -- and the infusion mark.
///
/// Prologue `48 89 5c 24 18 55 56 57` -- its own. `scripts/ds2-arxan-chain.py 0x1400bc850`
/// terminates at hop 0 with `NOT REDIRECTED (clean prologue at the entry)`.
pub const FE_ITEM_CELL_BIND: u32 = 0x000b_c850;

/// The bytes at [`FE_ITEM_CELL_BIND`], re-read before the site is patched.
pub const FE_ITEM_CELL_BIND_PROLOGUE: [u8; 8] = [0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57];

/// The equipment screen's own infusion bind. `FUN_140095650`. RVA `0x00095650`.
///
/// `fn(container: *ElementAccessor, item: *FeItemData)` -- and the first argument is already the
/// infusion container's accessor, not a cell view, which is the whole difference from
/// [`FE_ITEM_CELL_BIND`]. Its body is that bind's infusion loop and nothing else:
///
/// ```text
/// for i in 0..0x10:
///     path  = 0x5f5c3e0 + i
///     shown = i == FUN_140034e70(item)          ; FE_ITEM_INFUSION_READ, the nibble
///     FUN_14001e270(FUN_140027c80(container, out, &path) + 8, shown)
/// ```
///
/// So it drives all sixteen ids, [`FE_ITEM_WARN_ELEMENT`] among them, and hides ours on every
/// bind for the same reason the inventory's loop does -- which is what makes a detour running
/// after the original the last word on the element here too.
///
/// **Why the equipment screen needed its own hook at all.** Its slot cells are `def 0x0128` and
/// `def 0x012c` of `l02_01_In-Game.flo`, laid out by `def 0x0133`: six weapon slots, four armour,
/// four rings. Both cells hold the nine-id container at child `[2]`, so the container detour
/// already gives them the badge -- but nothing on that screen went through `FUN_1400bc850`, so
/// nothing ever showed it. `FUN_140097150` is the refresh that walks the slot table at
/// `PTR_DAT_141561ef0`, resolves each slot down to element `0x5f5c3e2`, and calls this.
///
/// Prologue `48 89 5c 24 10 48 89 6c` -- its own. `scripts/ds2-arxan-chain.py 0x140095650`
/// terminates at hop 0 with `NOT REDIRECTED (clean prologue at the entry)`.
pub const FE_EQUIP_SLOT_BIND: u32 = 0x0009_5650;

/// The bytes at [`FE_EQUIP_SLOT_BIND`], re-read before the site is patched.
pub const FE_EQUIP_SLOT_BIND_PROLOGUE: [u8; 8] = [0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c];

/// The element id an equipment slot cell hangs its infusion container on, in `l02_01_In-Game.flo`.
///
/// `def 0x0128` child `[2]` and `def 0x012c` child `[2]`, both naming `def 0x0121` -- the same
/// nine-id container the inventory's cell holds. Recorded because it is the path
/// `FUN_140097150` resolves before calling [`FE_EQUIP_SLOT_BIND`], and therefore the reason that
/// bind's first argument is already inside the container.
pub const FE_EQUIP_SLOT_CONTAINER_ELEMENT: u32 = 0x05f5_c3e2;

/// The cell view's element accessors, built by `FUN_1400b7680` (`0x000b7680`) from the grid cell.
///
/// Every one of these is an id path resolved against the cell's own element, and the offsets are
/// what [`FE_ITEM_CELL_BIND`] then writes through. Read straight off `FUN_1400b7680`, whose whole
/// body is eight `FUN_140027c80` chains:
///
/// ```text
/// +0x090   0x5f5c3e0 / 0x5f5c3e0   the item icon      (the texture is set through its +0x40)
/// +0x120   0x5f5c3e0 / 0x5f5c3e1   a mark, from the entry's +0x1f bit 1
/// +0x1b0   0x5f5c3e0 / 0x5f5c3e2   a mark, from FUN_140039140
/// +0x240   0x5f5c3e1               the durability gauge (value through its +0x40)
/// +0x2d0   0x5f5c3e2               THE INFUSION CONTAINER
/// +0x360   ../ 0x5f5c420 / 0x5f5b9f2   the item name text
/// +0x3f0   ../ 0x5f5c421 / 0x5f5c5ad   the item count text
/// +0x480   0x5f5c3e0 / 0x5f5c3e3   a mark, from FeItemData byte +4
/// ```
pub const FE_ITEM_CELL_INFUSION_ACCESSOR_OFFSET: usize = 0x2d0;

/// An accessor's "visible" sub-object, the `this` [`FE_ELEMENT_SET_VISIBLE`] takes. `+0x08`.
///
/// Every `FUN_14001e270` call in [`FE_ITEM_CELL_BIND`] is on `accessor + 8`: `+0x128` for the
/// accessor at `+0x120`, `+0x1b8` for `+0x1b0`, `+0x248` for `+0x240`, `+0x488` for `+0x480`.
/// Four sites agreeing is what fixes it rather than one.
pub const FE_ELEMENT_ACCESSOR_VISIBLE_OFFSET: usize = 0x08;

/// `FUN_140027c80(parent, out, path)`. RVA `0x00027c80`. Resolve an id path to an accessor.
///
/// The one lookup the whole frontend builds element accessors with -- `FUN_1400b7680` calls it
/// eight times, [`FE_ITEM_CELL_BIND`]'s infusion loop calls it sixteen. `path` is a
/// `DLKR::DLFixedVector<u32, 8>`: elements at `path + (-path & 3)` stride `4`, count at
/// `path + 0x28`, which is the alignment idiom `0x1400bca4f`..`0x1400bca62` spells out.
pub const FE_ELEMENT_RESOLVE: u32 = 0x0002_7c80;

/// Byte offset of the count inside the path vector [`FE_ELEMENT_RESOLVE`] takes. `+0x28`.
///
/// `mov QWORD PTR [rbp+0x98],0x1` at `0x1400bca7d` against the vector based at `[rbp+0x70]`.
pub const FE_ELEMENT_PATH_COUNT_OFFSET: usize = 0x28;

/// How many bytes of zeroed scratch one id path needs. `0x30`, the count field plus its own size.
pub const FE_ELEMENT_PATH_SIZE: usize = 0x30;

/// `FUN_14001e270(visibleSubObject, bool)`. RVA `0x0001e270`. Show or hide an element.
pub const FE_ELEMENT_SET_VISIBLE: u32 = 0x0001_e270;

/// The sixteen element ids [`FE_ITEM_CELL_BIND`]'s infusion loop drives, starting here.
///
/// **This is the whole infusion-icon mechanism, and it is not an icon at all.** The loop at
/// `0x1400bca70`..`0x1400bcacc`, verbatim:
///
/// ```text
/// 0x1400bca78  call 0x140034e70          ; the infusion nibble -> [rsp+0x20]
/// 0x1400bca88  movzx r15d,BYTE PTR [rax] ; r15 = that nibble
/// 0x1400bca9d  add   eax,0x5f5c3e0       ; element id = base + i, with bl = i
/// 0x1400bcaa2  mov   DWORD PTR [rdi],eax ; the one-component path
/// 0x1400bcaa4  lea   rcx,[r14+0x2d0]     ; the infusion container's accessor
/// 0x1400bcab3  call 0x140027c80          ; resolve id under it
/// 0x1400bcab8  cmp   bl,r15b
/// 0x1400bcabf  sete  dl
/// 0x1400bcac2  call 0x14001e270          ; setVisible(i == infusion)
/// 0x1400bcac9  cmp   bl,0x10             ; sixteen slots
/// ```
///
/// Sixteen ids are driven and **nine are authored**: `l02_02_Inventory.flo`'s definition `0x0070`
/// holds exactly nine children carrying `0x5f5c3e9` down to `0x5f5c3e1`. Slot `0` -- no infusion --
/// has no element, which is why an uninfused weapon shows nothing, and slots `10..15` have none
/// either. An id no element answers resolves to an accessor that reports nothing, so the loop's
/// extra iterations are free.
///
/// So the answer to "how does an infusion change the icon" is that it does not: the icon is the
/// item id ([`FE_ITEM_CELL_BIND`]'s block comment), and the infusion is a tenth sibling element
/// that the layout already contains, switched on by index.
pub const FE_ITEM_CELL_INFUSION_ELEMENT_BASE: u32 = 0x05f5_c3e0;

/// How many slots that loop drives. `cmp bl,0x10` at `0x1400bcac9`.
pub const FE_ITEM_CELL_INFUSION_SLOTS: u32 = 0x10;

/// `FUN_140034e70(item, out)`. RVA `0x00034e70`. The infusion nibble, and the gate on it.
///
/// Four instructions' worth of answer, at `0x140034ea9`:
///
/// ```text
/// cmp   byte ptr [rax + 0x1e],0x1   ; the inventory entry's item TYPE
/// ja    not-infusable               ; -> 0
/// movzx eax,byte ptr [rax + 0x26]
/// and   al,0xf                      ; the low nibble is the infusion
/// ```
///
/// `rax` is the entry [`ITEM_INVENTORY_ENTRY_LOOKUP`] returned. So the infusion is
/// [`ITEM_ENTRY_INFUSION_OFFSET`] masked with [`ITEM_ENTRY_INFUSION_MASK`], and only item types
/// `0` and `1` can carry one -- which is the game's own definition of "a weapon or a shield" and
/// is reused as the gate on the mark this repo adds.
pub const FE_ITEM_INFUSION_READ: u32 = 0x0003_4e70;

/// `ItemInventory2`'s entry lookup. `FUN_1401abfb0(manager, handle)`. RVA `0x001abfb0`.
///
/// `manager` is `[[`[`GAME_MANAGER_IMP`]`] + `[`GAME_DATA_MANAGER_OFFSET`]`] + 0x10`, which is the
/// walk `0x140034e83`..`0x140034e9f` performs before every call. `handle` is the `u16` at
/// `FeItemData + 2`; `0xffff` means "no item" and the callers test for it first.
pub const ITEM_INVENTORY_ENTRY_LOOKUP: u32 = 0x001a_bfb0;

/// `GameDataManager` -> `ItemInventory2`. `+0x10`. `mov RCX,[RCX + 0x10]` at `0x140034e96`.
pub const GAME_DATA_MANAGER_ITEM_INVENTORY_OFFSET: usize = 0x10;

/// The `u16` handle inside a `FeItemData`. `+0x02`.
pub const FE_ITEM_DATA_HANDLE_OFFSET: usize = 0x02;

/// The handle value that means "no item". `0xffff`.
pub const FE_ITEM_DATA_NO_HANDLE: u16 = 0xffff;

/// The `u8` item type on an inventory entry. `+0x1e`.
pub const ITEM_ENTRY_TYPE_OFFSET: usize = 0x1e;

/// Highest item type that can carry an infusion. `1`, from `cmp byte ptr [rax+0x1e],1; jbe`.
pub const ITEM_ENTRY_TYPE_MAX_INFUSABLE: u8 = 1;

/// The `u8` holding the infusion in its low nibble. `+0x26`.
pub const ITEM_ENTRY_INFUSION_OFFSET: usize = 0x26;

/// The mask applied to it. `0x0f`, from `and al,0xf`.
pub const ITEM_ENTRY_INFUSION_MASK: u8 = 0x0f;

// --- the requirement test, which the game performs on every stat row it draws ---

/// `FUN_1400bcde0`. RVA `0x000bcde0`. **The comparison this repo reuses rather than reinvents.**
///
/// It decides which sequence a stat row in the item detail pane plays, and the unmet case is the
/// red one. Disassembled at `0x1400bcdfa`:
///
/// ```text
/// mov   rax,[0x1416148f0]        ; GAME_MANAGER_IMP
/// mov   rcx,[rax+0x22e0]         ; GAME_MANAGER_FRONTEND_ROOT_OFFSET
/// call  0x1404ffb20              ; -> *(u64*)(that + 0x138), the player's stat table
/// call  0x14003d750              ; -> FE_STAT_ROW_TABLE + key*12
/// movsx rcx,WORD PTR [rax+0x4]   ; the PLAYER-STAT INDEX for this column
/// js    not-a-requirement-row    ; negative -> this row compares two items instead
/// movss xmm0,[rdi+0x4]           ; the item's required value, as a float
/// lea   rdx,[rcx+rcx*2]          ; idx*3
/// movd  xmm1,[rsi+rdx*8]         ; stat table entry, stride 0x18, i32 at +0
/// cvtdq2ps xmm1,xmm1
/// comiss xmm0,xmm1
/// jbe   met                      ; required <= have
/// mov   DWORD PTR [rbx],0x98     ; UNMET: sequence 0x98, element 0x5f5c5b7
/// ```
///
/// So "the player fails this item's requirement" is, in the game's own words,
/// `required > playerStat[index]` -- strictly greater, on floats, against a table the game
/// maintains.
///
/// **This is the presentation check and not the mechanics check, and the two really are
/// different.** The damage penalty comes from `FUN_14034d3c0` (RVA `0x0034d3c0`), which computes a
/// continuous deficiency `sum(max(0, 1 - stat/required))` over the same four columns read straight
/// out of `WeaponParam` at `+0x18`/`+0x1a`/`+0x1c`/`+0x1e`, against the effective stat block at
/// `chrStatus + 0x16` (`FUN_14038d510`, nine bytes: `movzx eax,[rcx+rax*2+0x16]`), which is
/// `clamp(base + modifiers, 1, 99)`. This one is boolean, reads the frontend's own table, and is
/// what the number on the screen is coloured by.
///
/// **Two-handing does not reach this check, and the mechanism is not what it is usually called.**
/// DS2 does not scale Strength: `FUN_14034d3c0` halves the weapon's Strength requirement with an
/// integer shift, `shr cx,1` at `0x14034d44c`, for grip states `2` and `3`. The `1.5x` that does
/// exist belongs to power stance -- `FUN_140350170` (`0x00350170`), `mulss xmm0,[0x1410bd0a8]` at
/// `0x14035025c` -- and applies to grip states `4`, `5` and `6`. Neither reaches here: this
/// function takes no grip argument at all, so the detail pane's requirement numbers ignore both,
/// and so does anything built on this comparison.
///
/// **Not established: whether the table at [`FRONTEND_ROOT_PLAYER_STATS_OFFSET`] holds base or
/// modified stats.** Every xref to `FUN_1404ffb20` is a reader and the writer has not been found,
/// so "rings and spEffects are included" is unproven here, where the gameplay side's
/// `chrStatus + 0x16` block proves it. See `docs/DS2-ITEM-REQUIREMENTS.md`.
pub const FE_STAT_ROW_COLOUR: u32 = 0x000b_cde0;

/// `FUN_1404ffb20(frontendRoot)` -> the player's stat table. `+0x138`, in full:
/// `48 8b 81 38 01 00 00 c3` -- `mov rax,[rcx+0x138]; ret`.
pub const FRONTEND_ROOT_PLAYER_STATS_OFFSET: usize = 0x138;

/// Stride of one entry in that table. `0x18`, from `lea rdx,[rcx+rcx*2]` + `[rsi+rdx*8]`.
pub const PLAYER_STAT_STRIDE: usize = 0x18;

/// `DAT_14155def0`. RVA `0x0155def0`. `FE_ITEM_PARAM_TYPE` -> how to present that column.
///
/// `FUN_14003d750` (`0x0003d750`) is the accessor and bounds it at `0x60` entries of `12` bytes:
/// `if (0 <= key && key < 0x60) return base + key*3` on `undefined4*`. Its `+0x04` is an `i16`
/// player-stat index, `-1` on every column that is not a requirement.
///
/// **Exactly ten columns carry one**, dumped straight out of the image:
///
/// ```text
///  key  stat   what
///  0x11   8    armour: required Strength        0x33   8   weapon: required Strength
///  0x12   9    armour: required Dexterity       0x34   9   weapon: required Dexterity
///  0x13  10    armour: required Intelligence    0x35  10   weapon: required Intelligence
///  0x14  11    armour: required Faith           0x36  11   weapon: required Faith
///  0x42  10    ring: required Intelligence
///  0x43  11    ring: required Faith
/// ```
///
/// Which is corroborated from the other side by the detail pane's own row tables: the weapon pane
/// (`0x1415640c0`, 19 rows) opens with `0x33 0x34 0x35 0x36`, the armour pane (`0x141564090`, 10
/// rows) with `0x11 0x12 0x13 0x14`, and the ring pane (`0x141564078`, 5 rows) contains `0x42
/// 0x43`. Two readings, one answer, and **the stat indices are read from this table at runtime
/// rather than hardcoded here**, so the mapping stays the game's.
pub const FE_STAT_ROW_TABLE: u32 = 0x0155_def0;

/// Bytes per entry in [`FE_STAT_ROW_TABLE`]. `12`, from `lea rax,[rcx+rcx*2]` on a `u32` base.
pub const FE_STAT_ROW_TABLE_STRIDE: usize = 12;

/// The `i16` player-stat index inside one entry. `+0x04`. Negative means "not a requirement row".
pub const FE_STAT_ROW_STAT_INDEX_OFFSET: usize = 0x04;

/// Highest key [`FE_STAT_ROW_TABLE`] holds. `0x60` entries, so `0x5f`.
pub const FE_STAT_ROW_TABLE_ENTRIES: u32 = 0x60;

/// The four `FE_ITEM_PARAM_TYPE` keys that are a weapon's stat requirements.
///
/// In the order the detail pane lists them, which is the order of the player-stat indices
/// `8, 9, 10, 11`. `FUN_1400312e0`'s own cases say which bytes they are:
///
/// ```text
/// case 0x33: return *(u16*)(row + 0x70);   case 0x35: return *(u16*)(row + 0x74);
/// case 0x34: return *(u16*)(row + 0x72);   case 0x36: return *(u16*)(row + 0x76);
/// ```
pub const FE_ITEM_PARAM_WEAPON_REQUIREMENTS: [u32; 4] = [0x33, 0x34, 0x35, 0x36];

/// `FUN_14003c2d0(item, out)`. RVA `0x0003c2d0`. `FeItemData` -> the source descriptor.
///
/// Fills a union of three places an item can live -- `out[1]` the bag entry, `out[2]` a shop
/// entry, `out[6]` the allocator -- and always zeroes `out[0]`, which is the precondition
/// [`FE_ITEM_PARAM_ROWS`] tests. Sized by its own callers' stack temporaries: `0x50` bytes.
pub const FE_ITEM_DESCRIPTOR: u32 = 0x0003_c2d0;

/// How many bytes of zeroed scratch [`FE_ITEM_DESCRIPTOR`] writes. `0x50`.
pub const FE_ITEM_DESCRIPTOR_SIZE: usize = 0x50;

/// Index of the allocator inside that descriptor, in qwords. `6`.
pub const FE_ITEM_DESCRIPTOR_ALLOCATOR_SLOT: usize = 6;

/// `FUN_140035070(allocator, out, descriptor)`. RVA `0x00035070`. Descriptor -> the item's param
/// row. Returns nonzero on success and leaves the row in `out[0]`.
pub const FE_ITEM_PARAM_ROWS: u32 = 0x0003_5070;

/// How many bytes of zeroed scratch [`FE_ITEM_PARAM_ROWS`] writes. `out[0..=8]`, so `0x48`.
pub const FE_ITEM_PARAM_ROWS_SIZE: usize = 0x48;

/// `FUN_1400312e0(row, key)`. RVA `0x000312e0`. One `FE_ITEM_PARAM_TYPE` column out of a row.
///
/// A `switch (key - 1)` over `0x5e` cases (`ff ca / 83 fa 5e / 0f 87 ..` at the entry), every arm
/// of which is a plain load returning in `RAX`. **An integer return, which is what makes it safe
/// to call from a detour**: no xmm result, no allocation, no out-parameter.
///
/// It is reached in the shipped game through `FUN_14003c080` (`0x0003c080`), whose own callers
/// gate it to keys `0x42`/`0x43`; this repo calls it directly for the weapon requirement keys
/// instead of widening that gate.
pub const FE_ITEM_PARAM_COLUMN: u32 = 0x0003_12e0;

// --- the added mark, and the container it is added to ---

/// `FUN_140b50f20(ctx, doc, definition, parent)`. RVA `0x00b50f20`. The container builder.
///
/// It reads [`FLO_DEFINITION_CHILD_COUNT_OFFSET`] and [`FLO_DEFINITION_CHILDREN_OFFSET`] out of
/// `definition` and walks that many records through `FUN_140b50bc0`, and in the branch that
/// allocates a `FeComponentSprite` it hands the SAME pointer to the component's init
/// (`vtable[0x1a8](sprite, allocator, definition)`), which is where `+0x48` -- the field
/// `FUN_140b6bd80` bounds the display list by -- comes from. So substituting this ARGUMENT does
/// everything substituting [`FLO_FIND_DEFINITION`]'s RETURN does, for one container, without
/// touching a lookup another crate in this workspace already owns.
///
/// Prologue `40 55 41 54 41 56 41 57 48 83 ec 68`. `scripts/ds2-arxan-chain.py 0x140b50f20`
/// terminates at hop 0: `NOT REDIRECTED (clean prologue at the entry)`.
pub const FLO_BUILD_CONTAINER: u32 = 0x00b5_0f20;

/// The bytes at [`FLO_BUILD_CONTAINER`], re-read before the site is patched.
pub const FLO_BUILD_CONTAINER_PROLOGUE: [u8; 12] = [
    0x40, 0x55, 0x41, 0x54, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x68,
];

/// The nine element ids the infusion container holds, **in the order the file has them**.
///
/// This is the fingerprint the container is recognised by, and it is a fingerprint rather than a
/// definition index on purpose: the same container is authored three times over with three
/// different indices, and an index would catch one document of the three.
///
/// ```text
/// l02_02_Inventory.flo   def 0x0070   child defs 0x5f 0x61 0x63 0x65 0x67 0x69 0x6b 0x6d 0x6f
/// l02_03_equipment.flo   def 0x006b   child defs 0x5a 0x5c 0x5e 0x60 0x62 0x64 0x66 0x68 0x6a
/// l02_01_In-Game.flo     def 0x0121   child defs 0x110 .. 0x120
/// ```
///
/// All three carry these nine ids in this order. Nothing else in any of the three documents has
/// nine children carrying them.
pub const FLO_INFUSION_CONTAINER_IDS: [u32; 9] = [
    0x05f5_c3e9,
    0x05f5_c3e8,
    0x05f5_c3e7,
    0x05f5_c3e6,
    0x05f5_c3e5,
    0x05f5_c3e4,
    0x05f5_c3e3,
    0x05f5_c3e2,
    0x05f5_c3e1,
];

/// The element id the added mark carries. `0x5f5c3ef` -- slot `15` of the sixteen
/// [`FE_ITEM_CELL_BIND`] drives.
///
/// **Chosen because the game's own loop always turns it OFF.** The infusion nibble is masked to
/// `0..=9` by the nine authored elements and to `0..=15` by [`ITEM_ENTRY_INFUSION_MASK`], so
/// `i == 15` is a comparison that can succeed only if an entry carries a nibble no element
/// answers today. Every bind therefore hides this element before the mark's own detour, which
/// runs after the original, decides whether to show it. If that detour is ever absent the mark is
/// simply never shown -- the same inert-on-failure shape as `ds2-menu-row`'s action ids.
pub const FE_ITEM_WARN_ELEMENT: u32 = 0x05f5_c3ef;

/// Where the item icon sits inside a cell: `(left, top, right, bottom)` in cell-local units.
///
/// Three nested records and one quad, every one of them read out of `l02_02_Inventory.flo`:
///
/// ```text
/// cell def 0x007a  child[0] id 0x5f5c3e0 def 0x005d  at (0, 0)          scale (1, 1)
///   def 0x005d     child[1] id 0x5f5c3e0 def 0x0056  at (15.25, -9.65)  scale (0.810806, ..)
///     def 0x0056   child[0]              def 0x0055  at (0, 0)          scale (1, 1)
///       def 0x0055 child[1]              def 0x0054  at (0, 0)          scale (1, 1)
///         shape 0x0054, one quad, rect (0,0)-(64,128), offset (0,0)
/// ```
///
/// `0x0054` is the placeholder the item's own texture is bound into at runtime, so its rect is the
/// box the icon occupies rather than the art in it.
///
/// The scale is the half this was first written without, and the omission was visible on screen:
/// the badge derived from it hung below the icon instead of sitting in its corner. The quad is
/// `64 x 128`, but the record carrying it is scaled, so the box is `51.89 x 103.78` -- a bottom
/// edge of `94.13`, not `118.35`, which is `24.22` lower than the icon ever reaches.
///
/// It went unnoticed because `scripts/ds2-flo.py`'s `render` printed `xy=` and not `scale=`, so
/// four separate readings of these same records all returned a position and no scale. That script
/// prints both now.
///
/// Only the far edges move. `[0]` and `[1]` are the record's own origin, and a scale applies to
/// what a record contains rather than to where it sits.
pub const FE_ITEM_ICON_BOX: [f32; 4] = [
    15.25,
    -9.65,
    15.25 + 64.0 * FE_ITEM_ICON_SCALE,
    -9.65 + 128.0 * FE_ITEM_ICON_SCALE,
];

/// The scale on the record that carries the item icon's quad, in both documents that author an
/// item cell: `l02_02_Inventory.flo` `def 0x005d` child[1], and `l02_03_equipment.flo` `def 0x0058`
/// child[1]. The same number in both, read with `scripts/ds2-flo.py tree`.
pub const FE_ITEM_ICON_SCALE: f32 = 0.810806;

/// Where the infusion container sits inside a cell, in the same units.
///
/// `(51.40, 48.15)` in `l02_02_Inventory.flo` def `0x007a` child `[2]` AND in
/// `l02_03_equipment.flo` def `0x0075` child `[2]` -- the inventory list and the equip picker
/// place it identically. The pause menu's own two cells (`l02_01_In-Game.flo` defs `0x0128` and
/// `0x012c`) put it at `(53.90, 55.25)` and `(51.40, 52.80)`, so a mark positioned from this
/// constant lands up to `7.1` units off in those two. That is a few pixels and it is written down
/// rather than corrected, because correcting it means a per-document table and the mark is a
/// corner badge.
pub const FE_ITEM_INFUSION_CONTAINER_AT: [f32; 2] = [51.40, 48.15];

/// How far the added mark is inset from the icon's bottom-left corner.
pub const FE_ITEM_WARN_INSET: f32 = 2.0;

/// Where the cell's durability bar starts, which is the real bottom of the usable portrait.
///
/// The badge is anchored on this and not on [`FE_ITEM_ICON_BOX`]`[3]`, because the icon's box runs
/// *past* the bar: `94.13` against a bar starting at `71.85`. A badge placed against the box's
/// bottom edge therefore lands on the bar and below it -- which is what four rounds of screenshots
/// showed, the arrow sitting under the portrait rather than on it, with the box arithmetic correct
/// the whole time and anchored to the wrong edge.
///
/// Element `0x5f5c3e1`, the last child of the cell in both documents that author one:
///
/// ```text
/// l02_03_equipment.flo  def 0x0075 child[7]  def 0x0072  at (9.45, 71.85)
/// l02_02_Inventory.flo  def 0x007a child[7]  def 0x0077  at (9.45, 73.35)
/// ```
///
/// The lower of the two is taken, so the badge clears the bar in both rather than in one.
pub const FE_ITEM_CELL_BAR_TOP: f32 = 71.85;

/// Where that same bar starts horizontally, which is the game's own left margin inside a cell.
///
/// Both documents put it at `9.45`, and the badge's left edge is aligned to it rather than to
/// [`FE_ITEM_ICON_BOX`]`[0] + `[`FE_ITEM_WARN_INSET`], which sat `7.80` further right. The icon
/// box is not the visible tile: the cell's background shape (`0x004d` in equipment, `0x0052` in
/// inventory, the same quad in both) draws from `2.45` to `97.30`, so there is parchment to the
/// left of the icon box and the badge was stopping short of it.
///
/// The bar is the better anchor of the two edges available. `2.45` is the art's extreme edge and
/// runs under the cell's frame; `9.45` is where the game itself starts a full-width element inside
/// the same tile, so a badge on that line shares a margin with something already on screen.
pub const FE_ITEM_CELL_BAR_LEFT: f32 = 9.45;

/// The atlas rect the cloned infusion glyph ships with, which is the one thing that must be true
/// before [`FE_ITEM_WARN_SOURCE`] is written over it.
///
/// Shape `0x005e` in `l02_02_Inventory.flo`, `0x0059` in `l02_03_equipment.flo`, `0x010f` in
/// `l02_01_In-Game.flo` -- three different indices, ONE rect, and in all three the quad's own
/// offset is `(-934.70, -52.50)`, cancelling the rect exactly so the art lands on its record's
/// origin. All three sample **`waku_03`**, which is also where the game keeps its ✕; that is what
/// makes the swap a rect write rather than a texture swap. Checked with
/// `scripts/ds2-flo.py shape` on each document.
pub const FE_ITEM_WARN_SHIPPED_SOURCE: [f32; 4] = [934.70, 52.50, 960.30, 78.50];

/// **The game's own "you cannot use this" ✕**, as a rect in `waku_03`.
///
/// This is the mark the HUD already draws on an unusable quick-slot weapon, not art this repo
/// invented: `l01_05_L_key.flo` shape `0x002a`, one quad, source `(740.65, 164.05)-(769.65,
/// 195.55)`, drawn by def `0x0036` child `[6]` -- element `0x5f5c3e6` at `(180.70, 501.25)`,
/// depth `11`, over an item icon whose own box is `(183.15, 419.20)` plus `64 x 128`. So the game
/// puts it in the LOWER-LEFT of the icon it marks, which is where
/// [`FE_ITEM_WARN_OFFSET`] puts this one.
///
/// Found arithmetically rather than by eye: the ✕ was cut out of a screenshot of that HUD slot,
/// turned into a red-dominance template, and cross-correlated against every red blob
/// `scripts/ds2-atlas-find.py` reports in `waku_03`. This rect scores `+0.79`; the next best
/// candidate in the atlas scores `+0.35`.
pub const FE_ITEM_WARN_SOURCE: [f32; 4] = [740.65, 164.05, 769.65, 195.55];

/// The opaque extent of the ✕ inside [`FE_ITEM_WARN_SOURCE`], which is bigger than the ink.
///
/// `scripts/ds2-atlas-find.py waku_03.dds --red` labels the connected red blob at
/// `(745, 169)-(767, 191)`, `367` opaque pixels, mean `rgb(181, 44, 16)`. The quad's rect carries
/// `4.35` of transparent padding on the left and `4.95` on top, and `l01_05_L_key.flo` pays for it
/// the same way this does: its quad offset is `(-744.80, -168.70)` against a rect starting at
/// `(740.65, 164.05)`, overshooting by exactly the padding so the INK lands on the record's origin.
///
/// The mark is aligned on this and not on the rect, because a corner badge is aligned on what the
/// player can see.
pub const FE_ITEM_WARN_INK: [f32; 4] = [745.0, 169.0, 767.0, 191.0];

/// How far the ink sits inside [`FE_ITEM_WARN_SOURCE`]'s top-left corner.
pub const FE_ITEM_WARN_INK_INSET: [f32; 2] = [
    FE_ITEM_WARN_INK[0] - FE_ITEM_WARN_SOURCE[0],
    FE_ITEM_WARN_INK[1] - FE_ITEM_WARN_SOURCE[1],
];

/// The mark's size: the ✕'s ink, `22.00 x 22.00`, and not the padded rect around it.
pub const FE_ITEM_WARN_SIZE: [f32; 2] = [
    FE_ITEM_WARN_INK[2] - FE_ITEM_WARN_INK[0],
    FE_ITEM_WARN_INK[3] - FE_ITEM_WARN_INK[1],
];

/// The mark's translate, **relative to the infusion container**, which is what its record carries.
///
/// The icon's bottom-left corner inset by [`FE_ITEM_WARN_INSET`], minus the container's own origin.
/// Arithmetic rather than a literal so the three constants above stay the only measurements.
pub const FE_ITEM_WARN_OFFSET: [f32; 2] = [
    FE_ITEM_CELL_BAR_LEFT - FE_ITEM_INFUSION_CONTAINER_AT[0],
    FE_ITEM_CELL_BAR_TOP
        - FE_ITEM_WARN_SIZE[1]
        - FE_ITEM_WARN_INSET
        - FE_ITEM_INFUSION_CONTAINER_AT[1],
];

/// The destination rect `ds2-item-warn`'s `place` writes into the built component.
///
/// Anchored on [`FE_ITEM_WARN_SHIPPED_SOURCE`] and not on [`FE_ITEM_WARN_SOURCE`], which is the
/// one subtlety in the whole swap. `FUN_140b70200` copies the shape's quad rect into both the
/// destination array at `+0x50` and the source array at `+0x58`, and the cloned glyph's `.flo`
/// quad carries `(-934.70, -52.50)` against a rect starting at `(934.70, 52.50)` -- so something
/// downstream cancels the atlas origin and the shipped art lands on its record's origin.
/// Re-pointing the source at the ✕ changes which pixels are sampled and moves nothing, so the
/// destination stays measured from the rect that cancellation is built around.
///
/// It is not the per-quad matrix at `+0x48` doing the cancelling, or at least not by the time the
/// cell binds: see [`FE_TEXTURE_SHAPE_QUAD_MATRIX_TRANSLATE`], where a run found identity. Which
/// transform applies the glyph quad's offset is open, and it is the open half of this mark.
///
/// Its size is [`FE_ITEM_WARN_SOURCE`]'s. The draw (`0x140b6f200` -> `FUN_140b521c0`) builds four
/// vertices straight off the destination corners and maps the source rect onto them as UVs, so a
/// destination narrower than its source squashes the art.
pub const FE_ITEM_WARN_DEST: [f32; 4] = [
    FE_ITEM_WARN_SHIPPED_SOURCE[0] + FE_ITEM_WARN_OFFSET[0] - FE_ITEM_WARN_INK_INSET[0],
    FE_ITEM_WARN_SHIPPED_SOURCE[1] + FE_ITEM_WARN_OFFSET[1] - FE_ITEM_WARN_INK_INSET[1],
    FE_ITEM_WARN_SHIPPED_SOURCE[0] + FE_ITEM_WARN_OFFSET[0] - FE_ITEM_WARN_INK_INSET[0]
        + (FE_ITEM_WARN_SOURCE[2] - FE_ITEM_WARN_SOURCE[0]),
    FE_ITEM_WARN_SHIPPED_SOURCE[1] + FE_ITEM_WARN_OFFSET[1] - FE_ITEM_WARN_INK_INSET[1]
        + (FE_ITEM_WARN_SOURCE[3] - FE_ITEM_WARN_SOURCE[1]),
];

/// Which of the container's nine children the mark's record is cloned from. Child `0`.
///
/// The clone is still a clone, but it is no longer a clone for want of art. What it is for now is
/// the texture: a `FeComponentTextureShape` resolves its texture at draw time out of its shape
/// entry's quad (`quad+0x20` -> the document's texture table -> `+0x38` on the component), and
/// that entry is shared with every other user of the shape. Both rect arrays are per-component
/// copies, so a rect can be re-pointed for one badge without touching anything else -- and the ✕
/// is reachable that way only because the glyph being cloned samples the same atlas the ✕ lives
/// in.
///
/// `waku_03`, in all three documents that author an item cell. See [`FE_ITEM_WARN_SHIPPED_SOURCE`].
pub const FE_ITEM_WARN_CLONED_CHILD: usize = 0;

/// Depth the added record carries, relative to the last of the nine it joins.
///
/// The nine step by 2 (`1, 3, 5, .., 17`), so `+2` puts the mark above all of them. Draw order
/// among siblings follows the order they are attached in rather than this field -- see
/// [`FLO_RECORD_DEPTH_OFFSET`] -- and the mark is appended last either way.
pub const FE_ITEM_WARN_DEPTH_STEP: u16 = 2;

#[cfg(test)]
mod item_warn_tests {
    /// The mark is the ✕'s ink, measured with `scripts/ds2-atlas-find.py waku_03.dds --red`, and
    /// not the padded rect the quad names.
    #[test]
    fn the_size_is_the_ink_and_not_the_rect() {
        assert!((super::FE_ITEM_WARN_SIZE[0] - 22.0).abs() < 0.01);
        assert!((super::FE_ITEM_WARN_SIZE[1] - 22.0).abs() < 0.01);
        let padded = [
            super::FE_ITEM_WARN_SOURCE[2] - super::FE_ITEM_WARN_SOURCE[0],
            super::FE_ITEM_WARN_SOURCE[3] - super::FE_ITEM_WARN_SOURCE[1],
        ];
        assert!(
            padded[0] > super::FE_ITEM_WARN_SIZE[0] && padded[1] > super::FE_ITEM_WARN_SIZE[1],
            "if the rect were the ink, aligning on the ink would be pointless"
        );
    }

    /// The ink is inside the rect that samples it, which is what makes the inset a trim and not a
    /// crop of somebody else's art.
    #[test]
    fn the_ink_is_inside_its_source_rect() {
        let [left, top, right, bottom] = super::FE_ITEM_WARN_SOURCE;
        let ink = super::FE_ITEM_WARN_INK;
        assert!(ink[0] >= left && ink[1] >= top && ink[2] <= right && ink[3] <= bottom);
        const {
            assert!(super::FE_ITEM_WARN_INK_INSET[0] > 0.0);
            assert!(super::FE_ITEM_WARN_INK_INSET[1] > 0.0);
        }
    }

    /// The ✕ and the glyph it replaces are two rects of one atlas -- `waku_03`, `1024 x 256` --
    /// which is the only reason a source-rect write can reach the ✕ at all.
    #[test]
    fn both_rects_are_inside_waku_03() {
        for rect in [
            super::FE_ITEM_WARN_SOURCE,
            super::FE_ITEM_WARN_SHIPPED_SOURCE,
        ] {
            assert!(rect[0] >= 0.0 && rect[1] >= 0.0);
            assert!(rect[2] <= 1024.0, "past the atlas's right edge");
            assert!(rect[3] <= 256.0, "past the atlas's bottom edge");
            assert!(rect[2] > rect[0] && rect[3] > rect[1]);
        }
        let apart = (super::FE_ITEM_WARN_SOURCE[0] - super::FE_ITEM_WARN_SHIPPED_SOURCE[0]).abs();
        assert!(
            apart > 100.0,
            "these are supposed to be two different pictures"
        );
    }

    /// The destination rect `ds2-item-warn`'s `place` writes: the ✕ at the size of its own rect,
    /// moved into the icon's near corner.
    ///
    /// This lives here rather than beside the code that writes it because that module is
    /// `#[cfg(windows)]` and the host this is developed on is not Windows -- a test in there
    /// compiles nowhere and runs never, which is worse than no test, since it reads like coverage.
    #[test]
    fn the_destination_carries_the_new_arts_size() {
        let dest = super::FE_ITEM_WARN_DEST;
        let source = super::FE_ITEM_WARN_SOURCE;
        assert!(
            (dest[2] - dest[0] - (source[2] - source[0])).abs() < 0.01,
            "a destination narrower than its source squashes the ✕"
        );
        assert!((dest[3] - dest[1] - (source[3] - source[1])).abs() < 0.01);
        assert!(
            dest[1] < dest[3],
            "the ✕ is not directional and is not mirrored"
        );
        assert!(
            dest[0] < super::FE_ITEM_WARN_SHIPPED_SOURCE[0],
            "the badge sits left of the infusion glyph it is cloned from"
        );
    }

    /// The destination puts the ink where [`super::FE_ITEM_WARN_OFFSET`] says, padding discounted.
    ///
    /// That is the whole point of anchoring on the ink: with the rect anchored instead, the ✕
    /// would sit `4.35` right and `4.95` low of every other measurement in this module.
    #[test]
    fn the_ink_lands_on_the_offset() {
        let ink = [
            super::FE_ITEM_WARN_DEST[0] + super::FE_ITEM_WARN_INK_INSET[0]
                - super::FE_ITEM_WARN_SHIPPED_SOURCE[0],
            super::FE_ITEM_WARN_DEST[1] + super::FE_ITEM_WARN_INK_INSET[1]
                - super::FE_ITEM_WARN_SHIPPED_SOURCE[1],
        ];
        assert!((ink[0] - super::FE_ITEM_WARN_OFFSET[0]).abs() < 0.01);
        assert!((ink[1] - super::FE_ITEM_WARN_OFFSET[1]).abs() < 0.01);
    }

    /// The corner the badge lands in is inside the icon, which is what "on the weapon" means.
    #[test]
    fn the_corner_is_inside_the_icon() {
        let at = [
            super::FE_ITEM_INFUSION_CONTAINER_AT[0] + super::FE_ITEM_WARN_OFFSET[0],
            super::FE_ITEM_INFUSION_CONTAINER_AT[1] + super::FE_ITEM_WARN_OFFSET[1],
        ];
        // The TILE bounds the badge, not the icon's art box. Those are different rectangles and
        // conflating them is what put the badge under the portrait and then short of its left
        // margin: the art box is `(15.25, -9.65)-(67.14, 94.13)`, while the parchment the player
        // sees runs `(2.45, -5.85)-(97.30, 85.00)`. The badge is deliberately left of the art box
        // now, on the margin the cell's own durability bar uses.
        const TILE: [f32; 4] = [2.45, -5.85, 97.30, 85.00];
        assert!(at[0] >= TILE[0], "not off the left edge of the tile");
        assert!(
            at[0] + super::FE_ITEM_WARN_SIZE[0] <= TILE[2],
            "and not past its right"
        );
        assert!(
            at[1] + super::FE_ITEM_WARN_SIZE[1] <= super::FE_ITEM_CELL_BAR_TOP,
            "and above the durability bar, which is the edge that actually bounds it"
        );
    }
}

#[cfg(test)]
mod item_icon_box_tests {
    /// The icon box is the quad SCALED, not the quad.
    ///
    /// The sibling tests in `item_warn_tests` all compare the box against itself, so every one of
    /// them passed while `FE_ITEM_ICON_BOX` carried a bottom edge `24.22` below where the icon
    /// actually ends -- and the badge derived from it hung under the portrait on screen. This
    /// compares the box against the two numbers it is built from instead.
    #[test]
    fn the_scale_is_applied_to_both_far_edges() {
        let box_ = super::FE_ITEM_ICON_BOX;
        let width = box_[2] - box_[0];
        let height = box_[3] - box_[1];
        assert!(
            (width - 64.0 * super::FE_ITEM_ICON_SCALE).abs() < 0.01,
            "width must be the quad's 64 scaled, got {width}"
        );
        assert!(
            (height - 128.0 * super::FE_ITEM_ICON_SCALE).abs() < 0.01,
            "height must be the quad's 128 scaled, got {height}"
        );
        const {
            assert!(
                super::FE_ITEM_ICON_SCALE < 1.0,
                "a scale of 1 would make this test vacuous"
            )
        };
    }

    /// The badge sits inside the icon on BOTH axes, measured from the container it hangs off.
    ///
    /// The vertical half is the one that was wrong: the badge's bottom edge ran past the icon's.
    #[test]
    fn the_badge_does_not_hang_below_the_portrait() {
        let bottom = super::FE_ITEM_INFUSION_CONTAINER_AT[1]
            + super::FE_ITEM_WARN_OFFSET[1]
            + super::FE_ITEM_WARN_SIZE[1];
        assert!(
            bottom <= super::FE_ITEM_ICON_BOX[3],
            "badge bottom {bottom} is below the icon's {}",
            super::FE_ITEM_ICON_BOX[3]
        );
        let right = super::FE_ITEM_INFUSION_CONTAINER_AT[0]
            + super::FE_ITEM_WARN_OFFSET[0]
            + super::FE_ITEM_WARN_SIZE[0];
        assert!(
            right <= super::FE_ITEM_ICON_BOX[2],
            "badge right {right} is past the icon's {}",
            super::FE_ITEM_ICON_BOX[2]
        );
    }
}

#[cfg(test)]
mod item_warn_bar_tests {
    /// The badge clears the durability bar, which is the edge that actually bounds the portrait.
    ///
    /// The sibling test `the_badge_does_not_hang_below_the_portrait` passed throughout, because it
    /// measured against `FE_ITEM_ICON_BOX[3]` -- an edge `22.28` below the bar. Four runs put the
    /// arrow under the portrait while that test stayed green.
    #[test]
    fn the_badge_sits_above_the_durability_bar() {
        let bottom = super::FE_ITEM_INFUSION_CONTAINER_AT[1]
            + super::FE_ITEM_WARN_OFFSET[1]
            + super::FE_ITEM_WARN_SIZE[1];
        assert!(
            bottom <= super::FE_ITEM_CELL_BAR_TOP,
            "badge bottom {bottom} is on or under the bar at {}",
            super::FE_ITEM_CELL_BAR_TOP
        );
    }

    /// The bar is the tighter bound, which is the whole reason the anchor moved.
    #[test]
    fn the_bar_is_above_the_icon_boxs_bottom() {
        const {
            assert!(
                super::FE_ITEM_CELL_BAR_TOP < super::FE_ITEM_ICON_BOX[3],
                "if the icon box ended first, anchoring on it would have been correct"
            )
        };
    }

    /// The badge is still inside the portrait vertically, not floated off the top of it.
    #[test]
    fn the_badge_is_below_the_icons_top() {
        let top = super::FE_ITEM_INFUSION_CONTAINER_AT[1] + super::FE_ITEM_WARN_OFFSET[1];
        assert!(
            top > super::FE_ITEM_ICON_BOX[1],
            "badge top {top} is above the icon"
        );
    }
}

#[cfg(test)]
mod item_warn_left_tests {
    /// The badge's left edge is the bar's, so the two share the cell's own margin.
    #[test]
    fn the_badge_lines_up_with_the_bar() {
        let left = super::FE_ITEM_INFUSION_CONTAINER_AT[0] + super::FE_ITEM_WARN_OFFSET[0];
        assert!(
            (left - super::FE_ITEM_CELL_BAR_LEFT).abs() < 0.01,
            "badge left {left} is not the bar's {}",
            super::FE_ITEM_CELL_BAR_LEFT
        );
    }

    /// That margin is further left than the icon box allowed, which is the change.
    #[test]
    fn the_bar_margin_is_left_of_the_icon_box() {
        const {
            assert!(
                super::FE_ITEM_CELL_BAR_LEFT
                    < super::FE_ITEM_ICON_BOX[0] + super::FE_ITEM_WARN_INSET,
                "anchoring on the bar has to move the badge left of where the icon box put it"
            )
        };
    }

    /// And it is still on the tile: the cell's background art starts left of the bar.
    #[test]
    fn the_badge_stays_on_the_parchment() {
        /// The cell background quad, offset plus rect, from `ds2-flo.py shape --shape 0x4d`.
        const TILE_LEFT: f32 = 2.45;
        const {
            assert!(
                super::FE_ITEM_CELL_BAR_LEFT > TILE_LEFT,
                "the bar's margin must sit inside the tile, not on its frame"
            )
        };
    }
}
